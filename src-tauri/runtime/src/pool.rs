//! WorkerPool — runs the spec's per-worker lifecycle. The single-iteration
//! core (process_one_claim) is fully unit-tested with FakeRunner + an in-memory
//! pool, with zero subprocesses. The continuous tokio loop (started at the
//! composition root) just calls process_one_claim repeatedly per team.

use crate::brake::Brake;
use crate::router::{route, RouteError};
use crate::task::{Task, TaskState};
use crate::task_store::{TaskStore, TaskStoreError};
use pipeline::model::{Pipeline, Team};
use runners::output::{InvocationRequest, Runner, RunnerError};
use runners::scope::{cleanup, prepare, ScopeError};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use workspace::paths::PathVars;

#[derive(Debug, Error)]
pub enum PoolError {
    #[error(transparent)]
    Task(#[from] TaskStoreError),
    #[error(transparent)]
    Scope(#[from] ScopeError),
    #[error(transparent)]
    Transition(#[from] crate::task::TaskTransitionError),
    #[error("route error: {0:?}")]
    Route(RouteError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// What an iteration did — surfaced so the loop + tests can assert behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// No queued task for this stage.
    Idle,
    /// Brake is on; no claim issued.
    Braked,
    /// A task was claimed, invoked, and settled/routed to next_stage/next_state.
    Settled { task_id: String, next_stage: String, next_state: TaskState },
    /// The runner rate-limited; the task was released back to queued.
    RateLimited { task_id: String },
}

/// The collaborators one worker iteration needs. Cheap to clone (Arcs).
#[derive(Clone)]
pub struct PoolContext {
    pub pipeline: Arc<Pipeline>,
    pub runner: Arc<dyn Runner>,
    pub tasks: Arc<TaskStore>,
    pub brake: Arc<Brake>,
    pub project_root: PathBuf,
    /// Reads the team's prompt file content. Injected so tests don't touch disk
    /// for prompts. Production passes a closure that reads <root>/<team.prompt>.
    pub read_prompt: Arc<dyn Fn(&Team) -> String + Send + Sync>,
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Run at most one claim→settle→route iteration for a team stage.
pub async fn process_one_claim(ctx: &PoolContext, team: &Team) -> Result<ClaimOutcome, PoolError> {
    if ctx.brake.is_on() {
        return Ok(ClaimOutcome::Braked);
    }

    // 1. CLAIM
    let now = now_unix();
    let Some(mut task) = ctx.tasks.claim_next_for_stage(&team.id, now).await? else {
        return Ok(ClaimOutcome::Idle);
    };

    // 2. PREPARE scope
    let mut vars = PathVars::new(&ctx.project_root).with_task_id(&task.id.0);
    if let Some(repo) = &task.target_repo {
        vars = vars.with_target_repo(repo);
    }
    let scope_settings = prepare(&ctx.project_root, &team.id, &task.id.0, now, &team.scope, &vars)?;

    // 3. INVOKE runner (the ACL seam)
    let req = InvocationRequest {
        task_id: task.id.0.clone(),
        team_id: team.id.clone(),
        model: team.runner.model.clone(),
        thinking_budget: team.runner.effort.budget_tokens(),
        system_prompt: (ctx.read_prompt)(team),
        user_message: invocation_message(&task),
        settings_path: scope_settings.settings_path.to_string_lossy().into_owned(),
        add_dirs: scope_settings.add_dirs.clone(),
    };

    let result = ctx.runner.invoke(&req).await;

    // 5. SETTLE (cleanup scope file always)
    cleanup(&scope_settings.settings_path);

    let output = match result {
        Ok(o) => o,
        Err(e) if e.is_rate_limited() => {
            // Release the task back to queued (spec: rate-limit releases the
            // held task). Don't bump attempts.
            task.transition_to(TaskState::Queued, now_unix())?;
            ctx.tasks.update(&task).await?;
            return Ok(ClaimOutcome::RateLimited { task_id: task.id.0 });
        }
        Err(_e) => {
            // Non-rate-limit failure (spawn/parse/NoResult): this is an
            // OPERATIONAL failure — the invocation broke and produced NO
            // verdict. We reuse the *revise* route (send back, bump attempts)
            // as a safety valve so the task isn't stranded `running`, and so a
            // persistently-failing team eventually escalates to needs_human via
            // the attempts cap rather than needing a new error state. NOTE: this
            // synthetic revise is NOT a model-produced `Verdict::Revise` (canon:
            // a settled judgment); it is an operational fallback reusing the
            // route. Plan 5 telemetry/diagnostics should not read it as a real
            // model verdict. (See Decision D10.)
            let _ = settle_and_route(ctx, &mut task, agent_bus_core::Verdict::Revise).await?;
            let reloaded = ctx.tasks.get(&task.id).await?;
            return Ok(ClaimOutcome::Settled {
                task_id: reloaded.id.0,
                next_stage: reloaded.current_stage,
                next_state: reloaded.state,
            });
        }
    };

    // Record artifact pointer on the task.
    if let Some(ap) = &output.artifact_path {
        task.parent_artifact = Some(ap.clone());
    }

    // 6. ROUTE per verdict.
    settle_and_route(ctx, &mut task, output.verdict).await?;
    let reloaded = ctx.tasks.get(&task.id).await?;
    Ok(ClaimOutcome::Settled {
        task_id: reloaded.id.0,
        next_stage: reloaded.current_stage,
        next_state: reloaded.state,
    })
}

/// Build the user message handed to the model. v1: the topic for a fresh run.
fn invocation_message(task: &Task) -> String {
    task.topic.clone()
}

/// Settle the task (DOMAIN.md canon verb **Settle** — "the worker finishing;
/// emits a verdict event"): apply the router decision to the task and persist
/// it. This is the named home of the spec's lifecycle step 5 (SETTLE) + step 6
/// (ROUTE); `process_one_claim` calls it once a verdict is in hand.
async fn settle_and_route(
    ctx: &PoolContext,
    task: &mut Task,
    verdict: agent_bus_core::Verdict,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let routed = route(&ctx.pipeline, &task.current_stage, verdict, task.attempts)
        .map_err(PoolError::Route)?;
    if routed.bump_attempts {
        // Cap is enforced inside route(); bump is safe here.
        let _ = task.bump_attempts();
    }
    let now = now_unix();
    // The task is currently `running`; move it to the routed next_state.
    task.state = routed.next_state;
    task.current_stage = routed.next_stage.clone();
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    Ok(Some((routed.next_stage, routed.next_state)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use agent_bus_core::Verdict;
    use pipeline::model::{Gate, Routes, RunnerConfig, Scope, Workers};
    use agent_bus_core::{EffortMode, RunnerKind};
    use runners::fake::FakeRunner;
    use runners::output::{RunnerOutput, RunnerUsage};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        pool
    }

    fn team(id: &str, approve: Option<&str>, revise: Option<&str>) -> Team {
        Team {
            id: id.into(), name: id.into(), prompt: format!("prompts/{id}.md"),
            runner: RunnerConfig { kind: RunnerKind::ClaudeCli, model: "claude-opus-4-7".into(), effort: EffortMode::Standard, api_key_env: None },
            scope: Scope { reads: vec!["${project}/artifacts".into()], writes: vec![], tools: vec!["Read".into()] },
            outputs: Routes { on_approve: approve.map(String::from), on_revise: revise.map(String::from), on_reject: Some("needs-human".into()) },
            workers: Workers::default(),
        }
    }

    fn pipeline_with(teams: Vec<Team>, gates: Vec<Gate>) -> Pipeline {
        Pipeline { id: "p".into(), name: "P".into(), description: String::new(), schema_version: 1,
            teams, gates, escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }] }
    }

    fn ctx_with(pool: SqlitePool, pipeline: Pipeline, runner: Arc<dyn Runner>, root: PathBuf) -> PoolContext {
        PoolContext {
            pipeline: Arc::new(pipeline),
            runner,
            tasks: Arc::new(TaskStore::new(pool)),
            brake: Arc::new(Brake::new()),
            project_root: root,
            read_prompt: Arc::new(|_t: &Team| "system prompt".to_string()),
        }
    }

    fn approve_output() -> RunnerOutput {
        RunnerOutput { verdict: Verdict::Approve, artifact_path: Some("artifacts/analyses/a.md".into()), final_text: "VERDICT: approve".into(), usage: RunnerUsage::default() }
    }

    fn temp_root() -> PathBuf {
        let d = std::env::temp_dir().join(format!("abp-pool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn idle_when_no_queued_task() {
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("gate-1"), None)], vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }]);
        let ctx = ctx_with(pool, p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let outcome = process_one_claim(&ctx, &p.teams[0]).await.unwrap();
        assert_eq!(outcome, ClaimOutcome::Idle);
    }

    #[tokio::test]
    async fn braked_issues_no_claim() {
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("gate-1"), None)], vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }]);
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        // queue a task
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();
        ctx.brake.set_on("test");
        assert_eq!(process_one_claim(&ctx, &p.teams[0]).await.unwrap(), ClaimOutcome::Braked);
        // task remains queued
        assert_eq!(ctx.tasks.get(&t.id).await.unwrap().state, TaskState::Queued);
    }

    #[tokio::test]
    async fn approve_routes_into_gate_and_parks_gated() {
        let pool = fresh_pool().await;
        let p = pipeline_with(
            vec![team("research", Some("gate-1"), None)],
            vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }],
        );
        let root = temp_root();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), root.clone());
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        let outcome = process_one_claim(&ctx, &p.teams[0]).await.unwrap();
        assert_eq!(outcome, ClaimOutcome::Settled {
            task_id: t.id.0.clone(),
            next_stage: "gate-1".into(),
            next_state: TaskState::Gated,
        });
        let reloaded = ctx.tasks.get(&t.id).await.unwrap();
        assert_eq!(reloaded.state, TaskState::Gated);
        assert_eq!(reloaded.parent_artifact.as_deref(), Some("artifacts/analyses/a.md"));
        // scope file was cleaned up
        assert!(std::fs::read_dir(root.join(".agent-bus/runtime")).map(|mut d| d.next().is_none()).unwrap_or(true));
    }

    #[tokio::test]
    async fn revise_routes_back_and_bumps_attempts() {
        let pool = fresh_pool().await;
        let p = pipeline_with(
            vec![team("research", Some("done"), None), team("writers", Some("done"), Some("research"))],
            vec![],
        );
        let revise = RunnerOutput { verdict: Verdict::Revise, artifact_path: None, final_text: "VERDICT: revise".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(revise)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "writers".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[1]).await.unwrap();
        let reloaded = ctx.tasks.get(&t.id).await.unwrap();
        assert_eq!(reloaded.current_stage, "research");
        assert_eq!(reloaded.state, TaskState::Queued);
        assert_eq!(reloaded.attempts, 2);
    }

    #[tokio::test]
    async fn rate_limit_releases_the_task_without_bumping_attempts() {
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("done"), None)], vec![]);
        let runner = Arc::new(FakeRunner::new(vec![Err(RunnerError::RateLimited("429".into()))]));
        let ctx = ctx_with(pool.clone(), p.clone(), runner, temp_root());
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        let outcome = process_one_claim(&ctx, &p.teams[0]).await.unwrap();
        assert_eq!(outcome, ClaimOutcome::RateLimited { task_id: t.id.0.clone() });
        let reloaded = ctx.tasks.get(&t.id).await.unwrap();
        assert_eq!(reloaded.state, TaskState::Queued);
        assert_eq!(reloaded.attempts, 1);
    }
}
