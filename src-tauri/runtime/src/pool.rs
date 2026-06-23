//! WorkerPool — runs the spec's per-worker lifecycle. The single-iteration
//! core (process_one_claim) is fully unit-tested with FakeRunner + an in-memory
//! pool, with zero subprocesses. The continuous tokio loop (started at the
//! composition root) just calls process_one_claim repeatedly per team.

use crate::brake::Brake;
use crate::fanout_group::{Continuation, FanOutGroup};
use crate::fanout_store::{BarrierOutcome, FanOutStore};
use crate::revision::{compose_invocation_message, RevisionBundleReader};
use crate::router::{route, RouteError};
use crate::task::{Task, TaskState};
use crate::invocation_audit::{AuditUsage, ErrorClass, InvocationAuditStore, InvocationOutcome};
use crate::task_store::{TaskStore, TaskStoreError};
use agent_bus_core::{UsageEvent, UsageSink};
use pipeline::model::{Pipeline, Team};
use runners::output::{InvocationRequest, LogSink, Runner};
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
    #[error(transparent)]
    FanOut(#[from] crate::fanout_store::FanOutStoreError),
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

/// Builds a per-task display-only log sink. Given a task id, returns a `LogSink`
/// the runner forwards assistant prose to as the worker streams. Display-only:
/// the deltas never influence settle/route (R4). None = no streaming (the
/// runner's non-streaming `invoke` is used) — the no-op case for runtime-only
/// tests. (See DOMAIN.md → Runtime "Stream (live log)" / Runners "Log delta".)
pub type LogSinkFactory = dyn Fn(&str) -> LogSink + Send + Sync;

/// The collaborators one worker iteration needs. Cheap to clone (Arcs).
#[derive(Clone)]
pub struct PoolContext {
    pub pipeline: Arc<Pipeline>,
    pub runner: Arc<dyn Runner>,
    pub tasks: Arc<TaskStore>,
    /// The fan-out barrier store (sub-project 2). Linear pipelines never touch it.
    pub fanout: Arc<FanOutStore>,
    pub brake: Arc<Brake>,
    pub project_root: PathBuf,
    /// Reads the team's prompt file content. Injected so tests don't touch disk
    /// for prompts. Production passes a closure that reads <root>/<team.prompt>.
    pub read_prompt: Arc<dyn Fn(&Team) -> String + Send + Sync>,
    /// Where settled usage is published (the kernel seam, D2). None = drop usage
    /// (the no-op case, e.g. before a project is open / in runtime-only tests).
    pub usage_sink: Option<Arc<dyn UsageSink>>,
    /// Reads a task's persisted revise bundle (Plan 4 vet F1 consumer). None =
    /// topic-only invocations (the no-op case / runtime-only tests). Concrete
    /// reader is wired at the composition root over the comments table.
    pub revision_reader: Option<Arc<dyn RevisionBundleReader>>,
    /// Per-task log-sink factory (R4 live-log streaming). None = use the runner's
    /// non-streaming `invoke`. Display-only side channel; settle/route ignore it.
    pub log_sink: Option<Arc<LogSinkFactory>>,
    /// Per-invocation audit store (R3). None = no audit (runtime-only tests /
    /// pre-project boot). Standalone append-only root — written by start/settle
    /// below; never joined into the Task transaction.
    pub audit: Option<Arc<InvocationAuditStore>>,
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

    // R3: open an audit record for this invocation (outcome NULL = in-flight).
    // Best-effort: an audit write must never fail a settle (mirrors UsageSink).
    let effective_for_audit = team.effective_runner();
    let audit_id = match &ctx.audit {
        Some(store) => store
            .record_start(&task.id.0, &team.id, &effective_for_audit.model, task.attempts, now)
            .await
            .map_err(|e| eprintln!("runtime: invocation audit start failed: {e}"))
            .ok(),
        None => None,
    };

    // 2. PREPARE scope
    let mut vars = PathVars::new(&ctx.project_root).with_task_id(&task.id.0);
    if let Some(repo) = &task.target_repo {
        vars = vars.with_target_repo(repo);
    }
    let scope_settings = prepare(&ctx.project_root, &team.id, &task.id.0, now, &team.scope, &vars)?;

    // 3. INVOKE runner (the ACL seam)
    let effective = team.effective_runner();
    let req = InvocationRequest {
        task_id: task.id.0.clone(),
        team_id: team.id.clone(),
        model: effective.model.clone(),
        thinking_budget: effective.effort.budget_tokens(),
        system_prompt: (ctx.read_prompt)(team),
        user_message: compose_invocation_message(
            &task.topic,
            task.attempts,
            ctx.revision_reader.as_deref(),
            &task.id.0,
        )
        .await,
        settings_path: scope_settings.settings_path.to_string_lossy().into_owned(),
        add_dirs: scope_settings.add_dirs.clone(),
    };

    // Stream display-only log deltas when a sink factory is wired (R4); else use
    // the non-streaming invoke. Both return the identical RunnerOutput — the
    // verdict/artifact/usage parse + settle/route below are byte-for-byte the same.
    let result = match &ctx.log_sink {
        Some(factory) => {
            let sink = factory(&task.id.0);
            ctx.runner.invoke_stream(&req, &sink).await
        }
        None => ctx.runner.invoke(&req).await,
    };

    // 5. SETTLE (cleanup scope file always)
    cleanup(&scope_settings.settings_path);

    let output = match result {
        Ok(o) => o,
        Err(e) if e.is_rate_limited() => {
            settle_audit(ctx, &audit_id, &InvocationOutcome::Error(ErrorClass::of(&e)), &AuditUsage::default()).await;
            // Release the task back to queued (spec: rate-limit releases the
            // held task). Don't bump attempts.
            task.transition_to(TaskState::Queued, now_unix())?;
            ctx.tasks.update(&task).await?;
            return Ok(ClaimOutcome::RateLimited { task_id: task.id.0 });
        }
        Err(e) => {
            settle_audit(ctx, &audit_id, &InvocationOutcome::Error(ErrorClass::of(&e)), &AuditUsage::default()).await;
            // Non-rate-limit failure (spawn/parse/NoResult): this is an
            // OPERATIONAL failure — the invocation broke and produced NO
            // verdict. We reuse the *revise* route (send back, bump attempts)
            // as a safety valve so the task isn't stranded `running`, and so a
            // persistently-failing team eventually escalates to needs_human via
            // the attempts cap rather than needing a new error state. NOTE: this
            // synthetic revise is NOT a model-produced `Verdict::Revise` (canon:
            // a settled judgment); it is an operational fallback reusing the
            // route. Plan 5 telemetry/diagnostics should not read it as a real
            // model verdict. (See Decision D10.) The AUDIT records the error
            // class, not `revise` (R3 D4), so the trail reflects the failure.
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

    // Publish usage to Telemetry (Customer-Supplier via the kernel UsageSink
    // seam). Best-effort; a sink failure never blocks the settle.
    // NOTE on the newtype boundary: `pipeline::model::Team.id` is a plain
    // `String` (so is `InvocationRequest.team_id`), but the kernel
    // `UsageEvent.team_id` is the `TeamId` newtype — wrap it explicitly with
    // `TeamId(...)`. `task.id` is already a `TaskId`, so it passes through.
    if let Some(sink) = &ctx.usage_sink {
        sink.record(UsageEvent {
            ts: now_unix(),
            team_id: agent_bus_core::TeamId(team.id.clone()),
            task_id: Some(task.id.clone()),
            model: output.usage.model.clone(),
            input_tokens: output.usage.input_tokens,
            output_tokens: output.usage.output_tokens,
            cache_creation: output.usage.cache_creation,
            cache_read: output.usage.cache_read,
        });
    }

    // R3: settle the audit record with the model's verdict + usage. This is the
    // invocation outcome, recorded outside the Task transaction (VET F1 / D2).
    settle_audit(
        ctx,
        &audit_id,
        &InvocationOutcome::Verdict(output.verdict),
        &AuditUsage {
            model: output.usage.model.clone(),
            input_tokens: output.usage.input_tokens,
            output_tokens: output.usage.output_tokens,
            cache_creation: output.usage.cache_creation,
            cache_read: output.usage.cache_read,
        },
    )
    .await;

    // 6. ROUTE per verdict.
    settle_and_route(ctx, &mut task, output.verdict).await?;
    let reloaded = ctx.tasks.get(&task.id).await?;
    Ok(ClaimOutcome::Settled {
        task_id: reloaded.id.0,
        next_stage: reloaded.current_stage,
        next_state: reloaded.state,
    })
}

/// Best-effort settle of an audit row (R3). No-op when audit is unwired or the
/// start write was lost; a failure is logged, never propagated — an audit write
/// must never fail a settle (mirrors UsageSink discipline).
///
/// VET F1: this records the *invocation* outcome (the Claude call's verdict/error
/// — DOMAIN.md "Invocation = one Claude call"), which is true independent of
/// whether the subsequent Task transition (`settle_and_route`) persists. It is
/// DELIBERATELY outside the Task transaction (D2) — do NOT move it inside, that
/// would reintroduce the cross-root coupling the two-aggregate model forbids.
async fn settle_audit(
    ctx: &PoolContext,
    audit_id: &Option<String>,
    outcome: &InvocationOutcome,
    usage: &AuditUsage,
) {
    if let (Some(store), Some(id)) = (&ctx.audit, audit_id) {
        if let Err(e) = store.record_settle(id, outcome, usage, now_unix()).await {
            eprintln!("runtime: invocation audit settle failed: {e}");
        }
    }
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

    // FORK EXPANSION: an approve whose target is a Fork node fans the task out
    // into one sibling task per lane, then terminates the original (forked).
    // P1: if the SETTLING task is itself a lane of a parent group, the new group
    // is a CHILD — it carries the parent group id + the parent lane so its
    // completion settles the parent lane's verdict (DD-P1-2).
    if let Some(fork) = ctx.pipeline.forks.iter().find(|f| f.id == routed.next_stage) {
        let parent = task
            .group_id
            .clone()
            .zip(task.lane.clone());
        return expand_fork(ctx, task, fork, parent).await;
    }

    // JOIN BARRIER: an approve whose target is a Join node hits the barrier.
    if routed.next_state == TaskState::Joining {
        return resolve_barrier(ctx, task, agent_bus_core::Verdict::Approve).await;
    }

    if routed.bump_attempts {
        // Cap is enforced inside route(); bump is safe here.
        let _ = task.bump_attempts();
    }
    let now = now_unix();

    // A lane task routing to needs-human reports a reject verdict to its barrier
    // before parking (Decision D5). Return directly — resolve_barrier owns the
    // lane task's terminal persistence + any continuation (no double-write).
    if task.group_id.is_some() && routed.next_state == TaskState::NeedsHuman {
        return resolve_barrier(ctx, task, agent_bus_core::Verdict::Reject).await;
    }

    // The task is currently `running`; move it to the routed next_state.
    task.state = routed.next_state;
    task.current_stage = routed.next_stage.clone();
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    Ok(Some((routed.next_stage, routed.next_state)))
}

/// Expand a fork into one lane sibling per lane and create the (possibly child)
/// FanOutGroup. `parent` is `Some((parent_group_id, parent_lane))` when the fork
/// is nested inside a lane (P1) — the new group is then a child whose completion
/// settles that parent lane. Top-level forks pass `None`.
async fn expand_fork(
    ctx: &PoolContext,
    task: &mut Task,
    fork: &pipeline::model::Fork,
    parent: Option<(String, String)>,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let join = ctx
        .pipeline
        .joins
        .iter()
        .find(|j| fork.lanes.iter().all(|lane| lane_reaches(&ctx.pipeline, lane, &j.id)))
        .ok_or(PoolError::Route(RouteError::NoRoute))?;
    let group_id = format!("G-{}", uuid::Uuid::new_v4());
    let (parent_group_id, parent_lane) = match parent {
        Some((g, l)) => (Some(g), Some(l)),
        None => (None, None),
    };
    let group = FanOutGroup {
        id: group_id.clone(),
        pipeline: task.pipeline.clone(),
        join_target: join.id.clone(),
        downstream: join.downstream.clone(),
        expected_lanes: fork.lanes.clone(),
        completed: false,
        parent_group_id,
        parent_lane,
    };
    ctx.fanout.create(&group).await?;
    let now = now_unix();
    for lane in &fork.lanes {
        ctx.fanout.seed_lane(&group_id, lane).await?;
        let sib = Task::forked(task, lane, &group_id, &join.id, now);
        ctx.tasks.insert(&sib).await?;
    }
    task.state = TaskState::Done;
    task.current_stage = fork.id.clone();
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    Ok(Some((fork.id.clone(), TaskState::Done)))
}

/// Walk a lane from its entry team toward `join_id`, following on_approve.
/// Hierarchical (P1): a team hop follows on_approve; a GATE hop follows the
/// gate's downstream; a FORK hop jumps to that nested fork's paired join's
/// downstream (the nested group resolves to a single continuation there).
/// Reaching `join_id` is success. Bounded by total node count to terminate.
fn lane_reaches(p: &Pipeline, entry: &str, join_id: &str) -> bool {
    let bound = p.teams.len() + p.gates.len() + p.forks.len() + p.joins.len() + 1;
    let mut current = entry.to_string();
    for _ in 0..=bound {
        if current == join_id {
            return true;
        }
        if let Some(t) = p.teams.iter().find(|t| t.id == current) {
            match t.outputs.on_approve.as_deref() {
                Some(next) => { current = next.to_string(); continue; }
                None => return false,
            }
        }
        if let Some(g) = p.gates.iter().find(|g| g.id == current) {
            current = g.downstream.clone();
            continue;
        }
        if let Some(f) = p.forks.iter().find(|f| f.id == current) {
            // Pair this nested fork with its join: the join whose lanes are all
            // reachable from the fork's lanes. Continue from that join's downstream.
            match p.joins.iter().find(|j| f.lanes.iter().all(|lane| lane_reaches(p, lane, &j.id))) {
                Some(j) => { current = j.downstream.clone(); continue; }
                None => return false,
            }
        }
        return false;
    }
    false
}

/// Resolve a lane settlement against its fan-out group barrier. Records the lane
/// verdict, parks the lane task as terminal at its join, and — if this caller
/// completes the group — creates the single continuation task (downstream on
/// all-approve, needs-human otherwise). The completes-once guard lives in the
/// store (vet F1); this never creates more than one continuation.
async fn resolve_barrier(
    ctx: &PoolContext,
    task: &mut Task,
    verdict: agent_bus_core::Verdict,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let group_id = task.group_id.clone().ok_or(PoolError::Route(RouteError::NoRoute))?;
    let lane = task.lane.clone().unwrap_or_default();
    let now = now_unix();

    // Find this lane's join in the pipeline (by the lane task's join_target) to
    // read its resolution policy (P2 early-cancel, P3 quorum). Default => full
    // barrier (all-must-approve).
    let join = task
        .join_target
        .as_deref()
        .and_then(|jt| ctx.pipeline.joins.iter().find(|j| j.id == jt));
    // Quorum (P3) governs success; cancel_on_reject (P2) only applies to the
    // default all-must-approve barrier (DD7 — quorum set => cancel_on_reject
    // ignored).
    let quorum = join.and_then(|j| j.quorum);
    let early_cancel = quorum.is_none() && join.map(|j| j.cancel_on_reject).unwrap_or(false);

    let is_failure = verdict == agent_bus_core::Verdict::Reject;

    let outcome = if let Some(q) = quorum {
        ctx.fanout
            .record_and_try_quorum(&group_id, &lane, verdict, q)
            .await?
    } else if early_cancel && is_failure {
        ctx.fanout
            .record_failure_and_early_cancel(&group_id, &lane, verdict)
            .await?
    } else {
        ctx.fanout
            .record_and_try_complete(&group_id, &lane, verdict)
            .await?
    };

    // Park this lane task as terminal for the lane.
    task.state = TaskState::Done;
    task.current_stage = task.join_target.clone().unwrap_or_else(|| task.current_stage.clone());
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    if let BarrierOutcome::Completed(cont) = outcome {
        // Stop outstanding lanes when an early resolution wins so queued siblings
        // aren't claimed+run wastefully (DD4/DD5): an early-cancel (P2) OR a
        // quorum (P3) decided before all lanes settled. The continuation is
        // created regardless; cancellation only trims waste.
        if (early_cancel && is_failure) || quorum.is_some() {
            ctx.tasks.cancel_outstanding_lanes(&group_id, now_unix()).await?;
        }
        return finish_group(ctx, &group_id, task, cont).await;
    }
    Ok(Some((task.current_stage.clone(), TaskState::Done)))
}

/// A group has completed. If it is a ROOT group, spawn the single continuation
/// task (downstream on all-approve, needs-human otherwise) — the original flat
/// behavior. If it is a CHILD group (a nested fork inside a parent lane), settle
/// the PARENT lane's verdict at the parent group's barrier instead — the same
/// completes-once guard one level up (DD-P1-2). This may complete the parent,
/// which recurses again — exactly-once holds independently at every level.
async fn finish_group(
    ctx: &PoolContext,
    group_id: &str,
    task: &mut Task,
    cont: Continuation,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let group = ctx.fanout.load(group_id).await?;
    if let (Some(parent_group), Some(parent_lane)) = (group.parent_group_id.clone(), group.parent_lane.clone()) {
        // Child group: feed the parent lane the derived verdict through the
        // parent's barrier (respecting the parent join's policy).
        let parent_verdict = FanOutGroup::parent_lane_verdict(&cont);
        return settle_parent_lane(ctx, task, &parent_group, &parent_lane, parent_verdict).await;
    }
    // Root group: create the single continuation task.
    spawn_continuation(ctx, task, cont).await
}

/// Create the single continuation task past a ROOT join (downstream queued, or
/// needs-human escalation). Carries the parent's lineage; clears lane fields so
/// the continuation is back in ordinary linear flow.
async fn spawn_continuation(
    ctx: &PoolContext,
    task: &Task,
    cont: Continuation,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let now = now_unix();
    let (stage, state) = match cont {
        Continuation::Downstream(ds) => (ds, TaskState::Queued),
        Continuation::NeedsHuman => ("needs-human".to_string(), TaskState::NeedsHuman),
    };
    let mut next = Task::injected(
        task.project_id.clone(),
        task.pipeline.clone(),
        stage.clone(),
        task.topic.clone(),
        task.target_repo.clone(),
        now,
    );
    next.state = state;
    next.parent_artifact = task.parent_artifact.clone();
    ctx.tasks.insert(&next).await?;
    Ok(Some((stage, state)))
}

/// Settle a parent lane's verdict at the parent group's barrier when a nested
/// CHILD group resolves (DD-P1-2). Reuses the same barrier methods the lane
/// settlement path uses — the parent join's policy (full / quorum / early-cancel)
/// governs, and the parent's `completed` guard arbitrates exactly-once. If the
/// parent itself completes, recurse via `finish_group` (which handles a
/// grandparent, etc.).
///
/// VET F1 — INVARIANT: each call here is its OWN single-row guard
/// (`UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0`). The
/// child-complete write (in the caller) and this parent-settle write are
/// DELIBERATELY two separate writes — exactly-once holds per group row,
/// independently at each level. Do NOT merge them into one transaction spanning
/// two `fanout_groups` rows: that re-introduces the cross-root coupling the
/// two-aggregate model forbids (and risks a two-row deadlock). Reference by id,
/// settle eventually — the correct cross-aggregate shape.
async fn settle_parent_lane(
    ctx: &PoolContext,
    task: &mut Task,
    parent_group: &str,
    parent_lane: &str,
    verdict: agent_bus_core::Verdict,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let parent = ctx.fanout.load(parent_group).await?;
    let join = ctx.pipeline.joins.iter().find(|j| j.id == parent.join_target);
    let quorum = join.and_then(|j| j.quorum);
    let early_cancel = quorum.is_none() && join.map(|j| j.cancel_on_reject).unwrap_or(false);
    let is_failure = verdict == agent_bus_core::Verdict::Reject;

    let outcome = if let Some(q) = quorum {
        ctx.fanout.record_and_try_quorum(parent_group, parent_lane, verdict, q).await?
    } else if early_cancel && is_failure {
        ctx.fanout.record_failure_and_early_cancel(parent_group, parent_lane, verdict).await?
    } else {
        ctx.fanout.record_and_try_complete(parent_group, parent_lane, verdict).await?
    };

    if let BarrierOutcome::Completed(cont) = outcome {
        if (early_cancel && is_failure) || quorum.is_some() {
            ctx.tasks.cancel_outstanding_lanes(parent_group, now_unix()).await?;
        }
        // Box::pin breaks the mutual-recursion cycle (finish_group ->
        // settle_parent_lane -> finish_group) so the async future has a finite
        // size; the climb is bounded by the validated nesting depth (VET F2).
        return Box::pin(finish_group(ctx, parent_group, task, cont)).await;
    }
    Ok(Some((parent_lane.to_string(), TaskState::Done)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use agent_bus_core::Verdict;
    use pipeline::model::{Gate, Routes, Scope, Workers};
    use agent_bus_core::{EffortMode, RunnerKind};
    use runners::fake::FakeRunner;
    use runners::output::{RunnerError, RunnerOutput, RunnerUsage};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;
    use std::sync::Mutex;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/007_invocation_audit.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/008_nested_groups.sql")).execute(&pool).await.unwrap();
        pool
    }

    fn team(id: &str, approve: Option<&str>, revise: Option<&str>) -> Team {
        Team {
            id: id.into(), name: id.into(), prompt: format!("prompts/{id}.md"),
            runner: Some(pipeline::TeamRunnerConfig { kind: Some(RunnerKind::ClaudeCli), model: Some("claude-opus-4-7".into()), effort: Some(EffortMode::Standard), api_key_env: None }),
            scope: Scope { reads: vec!["${project}/artifacts".into()], writes: vec![], tools: vec!["Read".into()] },
            outputs: Routes { on_approve: approve.map(String::from), on_revise: revise.map(String::from), on_reject: Some("needs-human".into()) },
            workers: Workers::default(),
        }
    }

    fn pipeline_with(teams: Vec<Team>, gates: Vec<Gate>) -> Pipeline {
        Pipeline { id: "p".into(), name: "P".into(), description: String::new(), schema_version: 1,
            defaults: None,
            teams, gates, escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![], joins: vec![] }
    }

    fn ctx_with(pool: SqlitePool, pipeline: Pipeline, runner: Arc<dyn Runner>, root: PathBuf) -> PoolContext {
        PoolContext {
            pipeline: Arc::new(pipeline),
            runner,
            tasks: Arc::new(TaskStore::new(pool.clone())),
            fanout: Arc::new(FanOutStore::new(pool)),
            brake: Arc::new(Brake::new()),
            project_root: root,
            read_prompt: Arc::new(|_t: &Team| "system prompt".to_string()),
            usage_sink: None,
            revision_reader: None,
            log_sink: None,
            audit: None,
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

    /// A test sink that records every event it receives.
    struct RecordingSink(Mutex<Vec<UsageEvent>>);
    impl UsageSink for RecordingSink {
        fn record(&self, e: UsageEvent) {
            self.0.lock().unwrap().push(e);
        }
    }

    #[tokio::test]
    async fn settle_emits_a_usage_event_to_the_sink() {
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("gate-1"), None)],
            vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }]);
        let out = RunnerOutput {
            verdict: Verdict::Approve,
            artifact_path: Some("artifacts/analyses/a.md".into()),
            final_text: "VERDICT: approve".into(),
            usage: RunnerUsage { model: "claude-opus-4-7".into(), input_tokens: 100, output_tokens: 20, cache_creation: 5, cache_read: 3 },
        };
        let mut ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(out)), temp_root());
        let sink = Arc::new(RecordingSink(Mutex::new(Vec::new())));
        ctx.usage_sink = Some(sink.clone());
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap();

        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].team_id.0, "research");
        assert_eq!(events[0].task_id.as_ref().unwrap().0, t.id.0);
        assert_eq!(events[0].input_tokens, 100);
        assert_eq!(events[0].output_tokens, 20);
        assert_eq!(events[0].model, "claude-opus-4-7");
    }

    #[tokio::test]
    async fn streaming_forwards_log_deltas_per_task_and_settles_unchanged() {
        use runners::output::LogSink;
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("gate-1"), None)],
            vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }]);
        // FakeRunner with scripted deltas for the single invocation.
        let runner = Arc::new(FakeRunner::with_deltas(
            vec![Ok(approve_output())],
            vec![vec!["chunk-a ".into(), "chunk-b".into()]],
        ));
        let mut ctx = ctx_with(pool.clone(), p.clone(), runner, temp_root());

        // Record (task_id, delta) pairs the factory's sinks emit.
        let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let seen_c = seen.clone();
        ctx.log_sink = Some(Arc::new(move |task_id: &str| -> LogSink {
            let tid = task_id.to_string();
            let seen_c = seen_c.clone();
            Box::new(move |d: &str| seen_c.lock().unwrap().push((tid.clone(), d.to_string())))
        }));

        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        let outcome = process_one_claim(&ctx, &p.teams[0]).await.unwrap();
        // settle is unchanged: approve into the gate, parked gated
        assert_eq!(outcome, ClaimOutcome::Settled {
            task_id: t.id.0.clone(), next_stage: "gate-1".into(), next_state: TaskState::Gated,
        });
        // the deltas were forwarded, tagged with this task's id
        let seen = seen.lock().unwrap();
        assert_eq!(*seen, vec![
            (t.id.0.clone(), "chunk-a ".to_string()),
            (t.id.0.clone(), "chunk-b".to_string()),
        ]);
    }

    #[tokio::test]
    async fn settle_writes_an_audit_record_with_verdict_and_usage() {
        use crate::invocation_audit::InvocationAuditStore;
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("gate-1"), None)],
            vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }]);
        let out = RunnerOutput {
            verdict: Verdict::Approve,
            artifact_path: Some("artifacts/analyses/a.md".into()),
            final_text: "VERDICT: approve".into(),
            usage: RunnerUsage { model: "claude-opus-4-7".into(), input_tokens: 100, output_tokens: 20, cache_creation: 5, cache_read: 3 },
        };
        let mut ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(out)), temp_root());
        let audit = Arc::new(InvocationAuditStore::new(pool.clone()));
        ctx.audit = Some(audit.clone());
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap();

        let rows = audit.list_for_task(&t.id.0).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.team_id, "research");
        assert_eq!(row.outcome_kind.as_deref(), Some("verdict"));
        assert_eq!(row.outcome.as_deref(), Some("approve"));
        assert!(row.settled_at.is_some());
        assert_eq!(row.usage.input_tokens, 100);
        assert_eq!(row.usage.output_tokens, 20);
    }

    #[tokio::test]
    async fn rate_limit_audits_error_class_and_leaves_no_verdict() {
        use crate::invocation_audit::InvocationAuditStore;
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("done"), None)], vec![]);
        let runner = Arc::new(FakeRunner::new(vec![Err(RunnerError::RateLimited("429".into()))]));
        let mut ctx = ctx_with(pool.clone(), p.clone(), runner, temp_root());
        let audit = Arc::new(InvocationAuditStore::new(pool.clone()));
        ctx.audit = Some(audit.clone());
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap();

        let rows = audit.list_for_task(&t.id.0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome_kind.as_deref(), Some("error"));
        assert_eq!(rows[0].outcome.as_deref(), Some("error:rate_limited"));
    }

    #[tokio::test]
    async fn no_usage_event_on_rate_limit() {
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("done"), None)], vec![]);
        let runner = Arc::new(FakeRunner::new(vec![Err(RunnerError::RateLimited("429".into()))]));
        let mut ctx = ctx_with(pool.clone(), p.clone(), runner, temp_root());
        let sink = Arc::new(RecordingSink(Mutex::new(Vec::new())));
        ctx.usage_sink = Some(sink.clone());
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();
        process_one_claim(&ctx, &p.teams[0]).await.unwrap();
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reclaim_includes_revise_bundle_in_invocation_message() {
        use crate::revision::{FakeRevisionReader, RevisionBundle, RevisionNote};
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("done"), None)], vec![]);
        let recorder = Arc::new(RecordingRunner::new(approve_output()));
        let mut ctx = ctx_with(pool.clone(), p.clone(), recorder.clone(), temp_root());
        ctx.revision_reader = Some(Arc::new(FakeRevisionReader {
            bundle: RevisionBundle {
                notes: vec![RevisionNote {
                    anchor_text: Some("batch key".into()),
                    note: "use per-row keys".into(),
                    kind: "inline".into(),
                }],
            },
        }));
        let mut t = Task::injected("proj".into(), "p".into(), "research".into(), "Bulk write".into(), None, 100);
        t.attempts = 2;
        ctx.tasks.insert(&t).await.unwrap();
        process_one_claim(&ctx, &p.teams[0]).await.unwrap();
        let seen = recorder.last_message();
        assert!(seen.starts_with("Bulk write"));
        assert!(seen.contains("REVISION REQUEST (attempt 2)"));
        assert!(seen.contains("use per-row keys"));
    }

    struct RecordingRunner {
        out: RunnerOutput,
        last: Mutex<String>,
    }
    impl RecordingRunner {
        fn new(out: RunnerOutput) -> Self { Self { out, last: Mutex::new(String::new()) } }
        fn last_message(&self) -> String { self.last.lock().unwrap().clone() }
    }
    #[async_trait::async_trait]
    impl Runner for RecordingRunner {
        async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
            *self.last.lock().unwrap() = req.user_message.clone();
            Ok(self.out.clone())
        }
    }

    // ---- Sub-project 2: fork/join (Tasks 12 & 13) ----

    use pipeline::model::{Fork, Join};

    fn pipeline_v2_forkjoin() -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 2,
            defaults: None,
            teams: vec![
                team("entry", Some("fork-1"), None),
                team("lane-a", Some("join-1"), None),
                team("lane-b", Some("join-1"), None),
                team("after", Some("done"), None),
            ],
            gates: vec![],
            escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: None }],
        }
    }

    fn pipeline_v2_nested() -> Pipeline {
        // entry -> fork-1 {lane-a, lane-b}; lane-a is itself a fork:
        //   lane-a -> fork-2 {a1, a2} -> join-2 -> mid-a -> join-1
        //   lane-b -> join-1
        // join-1 -> after
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 2,
            defaults: None,
            teams: vec![
                team("entry", Some("fork-1"), None),
                team("lane-a", Some("fork-2"), None),
                team("a1", Some("join-2"), None),
                team("a2", Some("join-2"), None),
                team("mid-a", Some("join-1"), None),
                team("lane-b", Some("join-1"), None),
                team("after", Some("done"), None),
            ],
            gates: vec![],
            escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![
                Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] },
                Fork { id: "fork-2".into(), lanes: vec!["a1".into(), "a2".into()] },
            ],
            joins: vec![
                Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: None },
                Join { id: "join-2".into(), waits_for: vec!["a1".into(), "a2".into()], downstream: "mid-a".into(), cancel_on_reject: false, quorum: None },
            ],
        }
    }

    #[tokio::test]
    async fn nested_fork_inside_a_lane_spawns_a_child_group_linked_to_parent() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_nested();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork-1 -> lane-a, lane-b
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approve -> fork-2 (nested) -> a1, a2

        // Two queued lane tasks for the nested fork (a1, a2), each in a CHILD group.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let child_lane_tasks: Vec<_> = queued.iter().filter(|q| q.current_stage == "a1" || q.current_stage == "a2").collect();
        assert_eq!(child_lane_tasks.len(), 2, "nested fork expands into 2 child lane tasks");
        let child_group = child_lane_tasks[0].group_id.clone().unwrap();
        assert!(child_lane_tasks.iter().all(|q| q.group_id.as_deref() == Some(child_group.as_str())));
        // The child group is linked to the parent group + lane-a.
        let cg = ctx.fanout.load(&child_group).await.unwrap();
        assert!(cg.is_child());
        assert_eq!(cg.parent_lane.as_deref(), Some("lane-a"));
        assert!(cg.parent_group_id.is_some());
    }

    #[tokio::test]
    async fn nested_child_all_approve_settles_parent_lane_and_parent_completes() {
        // Full nested run: entry -> fork-1 -> (lane-a -> fork-2 -> a1,a2 -> join-2 -> mid-a -> join-1), lane-b -> join-1.
        // Everything approves; exactly one continuation at `after`.
        let pool = fresh_pool().await;
        let p = pipeline_v2_nested();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork-1
        process_one_claim(&ctx, &p.teams[5]).await.unwrap(); // lane-b approve -> join-1 (parks; parent group not complete)
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approve -> fork-2 (child group)
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // a1 approve -> join-2 (child parks)
        process_one_claim(&ctx, &p.teams[3]).await.unwrap(); // a2 approve -> join-2 (child COMPLETES -> settles parent lane-a)
        // child completing settled parent lane-a=approve; parent had lane-b=approve -> parent completes -> mid-a? No:
        // parent join-1 downstream is `after`. The child's join-2 downstream (mid-a) is a lane-internal continuation that
        // is consumed as the parent-lane verdict (DD-P1-8). So after the child completes, the parent lane-a is settled
        // and the PARENT barrier fires -> one continuation at `after`.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let at_after: Vec<_> = queued.iter().filter(|q| q.current_stage == "after").collect();
        assert_eq!(at_after.len(), 1, "exactly one continuation past the outer join");
        assert_eq!(at_after[0].group_id, None, "continuation is back in linear flow");
    }

    #[tokio::test]
    async fn nested_child_reject_settles_parent_lane_reject_and_parent_needs_human() {
        // a1 rejects inside the nested fork -> child group resolves needs-human ->
        // parent lane-a settles as Reject -> parent (with lane-b approve) -> needs-human.
        let pool = fresh_pool().await;
        let p = pipeline_v2_nested();
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        // reject only the `a1` nested lane
        struct A1Reject { reject: RunnerOutput }
        #[async_trait::async_trait]
        impl Runner for A1Reject {
            async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
                if req.team_id == "a1" { Ok(self.reject.clone()) }
                else { Ok(RunnerOutput { verdict: Verdict::Approve, artifact_path: Some("a.md".into()), final_text: "VERDICT: approve".into(), usage: RunnerUsage::default() }) }
            }
        }
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(A1Reject { reject }), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork-1
        process_one_claim(&ctx, &p.teams[5]).await.unwrap(); // lane-b approve (parks)
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a -> fork-2
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // a1 reject -> child barrier reject
        process_one_claim(&ctx, &p.teams[3]).await.unwrap(); // a2 approve -> child completes needs-human -> parent lane-a reject -> parent needs-human

        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1, "the nested reject escalates the outer join exactly once");
        assert_eq!(nh[0].current_stage, "needs-human");
    }

    #[tokio::test]
    async fn nested_completion_is_exactly_once_against_a_concurrent_parent_straggler() {
        // lane-b and the nested child both try to complete the PARENT barrier concurrently.
        // Exactly one parent continuation results.
        use std::sync::Arc as StdArc;
        let pool = fresh_pool().await;
        let p = pipeline_v2_nested();
        let ctx = StdArc::new(ctx_with(pool.clone(), p.clone(), StdArc::new(FakeRunner::always(approve_output())), temp_root()));
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();
        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork-1
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a -> fork-2
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // a1 parks
        // Now run lane-b (settles parent lane-b) and a2 (completes child -> settles parent lane-a) concurrently.
        let c1 = ctx.clone(); let p1 = p.clone();
        let c2 = ctx.clone(); let p2 = p.clone();
        let h1 = tokio::spawn(async move { process_one_claim(&c1, &p1.teams[5]).await.unwrap() });
        let h2 = tokio::spawn(async move { process_one_claim(&c2, &p2.teams[3]).await.unwrap() });
        let _ = (h1.await.unwrap(), h2.await.unwrap());
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let at_after = queued.iter().filter(|q| q.current_stage == "after").count();
        assert_eq!(at_after, 1, "exactly one parent continuation despite concurrent settles");
    }

    #[test]
    fn lane_reaches_traverses_a_nested_fork_to_the_outer_join() {
        let p = pipeline_v2_nested();
        // lane-a goes through fork-2 / join-2 / mid-a before hitting join-1
        assert!(lane_reaches(&p, "lane-a", "join-1"));
        // lane-b is a direct linear lane to join-1
        assert!(lane_reaches(&p, "lane-b", "join-1"));
        // the nested fork's own lanes reach join-2 directly
        assert!(lane_reaches(&p, "a1", "join-2"));
    }

    /// A runner that returns a seeded reject for the `lane-b` team and approve
    /// for everything else — used to exercise the one-reject barrier path.
    struct StageRunner { reject: RunnerOutput }
    impl StageRunner { fn new(reject: RunnerOutput) -> Self { Self { reject } } }
    #[async_trait::async_trait]
    impl Runner for StageRunner {
        async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
            if req.team_id == "lane-b" {
                Ok(self.reject.clone())
            } else {
                Ok(RunnerOutput { verdict: Verdict::Approve, artifact_path: Some("a.md".into()), final_text: "VERDICT: approve".into(), usage: RunnerUsage::default() })
            }
        }
    }

    #[tokio::test]
    async fn approve_into_fork_spawns_one_sibling_per_lane_and_terminates_original() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap();

        let original = ctx.tasks.get(&t.id).await.unwrap();
        assert_eq!(original.state, TaskState::Done);

        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert_eq!(queued.len(), 2);
        let mut stages: Vec<String> = queued.iter().map(|q| q.current_stage.clone()).collect();
        stages.sort();
        assert_eq!(stages, vec!["lane-a".to_string(), "lane-b".to_string()]);
        let group = queued[0].group_id.clone().unwrap();
        assert!(queued.iter().all(|q| q.group_id.as_deref() == Some(group.as_str())));
        assert!(queued.iter().all(|q| q.join_target.as_deref() == Some("join-1")));
    }

    #[tokio::test]
    async fn all_lanes_approve_creates_one_downstream_continuation() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork -> 2 siblings
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approves into join (parks)
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b approves into join (completes)

        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let at_after: Vec<_> = queued.iter().filter(|q| q.current_stage == "after").collect();
        assert_eq!(at_after.len(), 1, "exactly one continuation past the join");
        assert_eq!(at_after[0].group_id, None, "continuation is back in linear flow");
    }

    #[tokio::test]
    async fn one_lane_reject_routes_join_to_needs_human() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approve (parks)
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b reject -> barrier reject

        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1, "the joined task escalates to needs-human");
        assert_eq!(nh[0].current_stage, "needs-human");
    }

    #[tokio::test]
    async fn partial_completion_parks_without_continuing() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // only lane-a settles

        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().all(|q| q.current_stage != "after"), "no continuation while a lane is outstanding");
        assert!(queued.iter().any(|q| q.current_stage == "lane-b"), "lane-b still queued");
    }

    #[tokio::test]
    async fn restart_mid_group_re_evaluates_idempotently() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a parks
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b completes -> 1 continuation

        let group = {
            let done = ctx.tasks.list_by_state(TaskState::Done).await.unwrap();
            done.iter().find_map(|d| d.group_id.clone()).unwrap()
        };
        let again = ctx.fanout.record_and_try_complete(&group, "lane-b", Verdict::Approve).await.unwrap();
        assert_eq!(again, BarrierOutcome::Parked, "re-settle after completion creates nothing");

        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert_eq!(queued.iter().filter(|q| q.current_stage == "after").count(), 1);
    }

    // ---- P2: early-cancel on first reject ----

    fn pipeline_v2_forkjoin_cancel_on_reject() -> Pipeline {
        let mut p = pipeline_v2_forkjoin();
        p.joins[0].cancel_on_reject = true;
        p
    }

    #[tokio::test]
    async fn early_cancel_resolves_to_needs_human_before_other_lane_runs() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin_cancel_on_reject();
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork -> lane-a, lane-b queued
        // lane-b rejects FIRST, before lane-a ever runs.
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b reject -> early-cancel

        // the joined task escalates to needs-human immediately
        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1);
        assert_eq!(nh[0].current_stage, "needs-human");

        // lane-a was parked (cancelled) — it is no longer queued, so a worker
        // claiming lane-a finds nothing to run.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().all(|q| q.current_stage != "lane-a"),
            "outstanding lane-a is cancelled, not left queued");
        let idle = process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a
        assert_eq!(idle, ClaimOutcome::Idle, "no outstanding lane work remains");
    }

    #[tokio::test]
    async fn early_cancel_straggler_settle_creates_no_second_continuation() {
        // lane-a parks (approve) first; lane-b then rejects with cancel_on_reject.
        // Exactly one needs-human continuation; the early-cancel guard arbitrates.
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin_cancel_on_reject();
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approve (parks; group not complete yet)
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b reject -> early-cancel completes

        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1, "exactly one needs-human continuation");
    }

    #[tokio::test]
    async fn default_join_keeps_full_barrier_even_with_a_reject() {
        // cancel_on_reject = false (default): the reject does NOT short-circuit;
        // both lanes settle, then the group routes to needs-human as before.
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin(); // cancel_on_reject defaults false
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b reject FIRST

        // full barrier: NOT yet escalated — lane-a is still outstanding (queued).
        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert!(nh.is_empty(), "full barrier waits for all lanes before escalating");
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().any(|q| q.current_stage == "lane-a"),
            "lane-a remains queued under the full barrier");

        // lane-a settles -> NOW the group completes to needs-human.
        process_one_claim(&ctx, &p.teams[1]).await.unwrap();
        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1);
    }

    // ---- P3: quorum joins (N-of-M) ----

    fn pipeline_v2_forkjoin_quorum(q: u32) -> Pipeline {
        let mut p = pipeline_v2_forkjoin();
        p.joins[0].quorum = Some(q);
        p
    }

    #[tokio::test]
    async fn quorum_reached_resolves_downstream_before_other_lane_settles() {
        // 2 lanes, quorum 1: lane-a approving alone meets quorum -> downstream
        // continuation created immediately; lane-b is parked (cancelled).
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin_quorum(1);
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork -> lane-a, lane-b queued
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approves -> quorum 1 met

        // exactly one continuation at downstream, back in linear flow
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let at_after: Vec<_> = queued.iter().filter(|q| q.current_stage == "after").collect();
        assert_eq!(at_after.len(), 1, "quorum met -> one downstream continuation");
        assert_eq!(at_after[0].group_id, None, "continuation is back in linear flow");

        // lane-b was parked (cancelled) — not left queued; a worker claiming it idles.
        assert!(queued.iter().all(|q| q.current_stage != "lane-b"),
            "outstanding lane-b is cancelled once quorum resolves");
        let idle = process_one_claim(&ctx, &p.teams[2]).await.unwrap();
        assert_eq!(idle, ClaimOutcome::Idle, "no outstanding lane work remains");
    }

    #[tokio::test]
    async fn quorum_impossible_resolves_needs_human_early() {
        // 2 lanes, quorum 2: lane-b rejecting first makes quorum 2 impossible
        // (0 approvals + 1 unsettled < 2) -> needs-human immediately; lane-a parked.
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin_quorum(2);
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b rejects FIRST -> quorum impossible

        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1, "quorum impossible -> needs-human immediately");
        assert_eq!(nh[0].current_stage, "needs-human");

        // lane-a was parked, not left to run.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().all(|q| q.current_stage != "lane-a"),
            "outstanding lane-a is cancelled once quorum is impossible");
    }

    #[tokio::test]
    async fn quorum_default_none_keeps_full_barrier() {
        // No quorum (None): the full all-must-approve barrier path is unchanged —
        // one lane approving parks without continuing.
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin(); // quorum None
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // only lane-a approves

        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().all(|q| q.current_stage != "after"), "no continuation while a lane is outstanding (full barrier)");
        assert!(queued.iter().any(|q| q.current_stage == "lane-b"), "lane-b still queued under the full barrier");
    }
}
