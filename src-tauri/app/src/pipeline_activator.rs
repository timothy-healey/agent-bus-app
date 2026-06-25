//! PipelineActivator — makes runtime activation a runtime operation, not a
//! boot-only one. It owns the activation lifecycle: swap the active pipeline +
//! (re)spawn the per-team worker loops at a fresh generation. Built once at boot,
//! held in Tauri state, and invoked at boot AND on project create/switch (ONE
//! path).
//!
//! vet: this is a composition-root concern (it alone imports AppHandle + the
//! runner factory + Workspace/Pipeline stores). It hands Runtime only resolved
//! values via `ActivePipeline` — no new cross-context edge, the Task and
//! WorkerPool aggregates stay intact.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use pipeline::model::{Pipeline, Team};
use runners::claude_cli::ClaudeCliRunner;
use runners::output::Runner;
use runtime::api::{source_team, ActivePipeline, RuntimeState};
use runtime::brake::Brake;
use runtime::engine::{self, EngineContext, StepOutcome};
use runtime::fanout_store::FanOutStore;
use runtime::generator_ledger::GeneratorLedger;
use runtime::log_sink::LogSinkFactory;
use runtime::run_store::RunStore;
use runtime::store::StoreRepo;
use runtime::task_store::TaskStore;
use workspace::store::ProjectStore;

/// How long an idle / backpressured / retired worker loop sleeps before its next
/// poll (the bounded-buffer idle backoff). Short enough to keep latency low, long
/// enough that an idle pool doesn't spin.
const LOOP_IDLE_SLEEP: Duration = Duration::from_millis(350);

/// Collaborators a worker loop needs that are built ONCE at boot and reused
/// across activations. Cheap to clone (Arcs / Options of Arcs). ④d adds the
/// bounded-buffer engine aggregates (Store / Run / GeneratorLedger / FanOutGroup)
/// so each loop can build an `EngineContext` for the active run.
#[derive(Clone)]
pub struct WorkerDeps {
    // R (runtime hardening): the bounded-buffer `engine::invoke` seam now threads
    // these three observability side-channels — the usage sink (R5), the live-log
    // factory (R4), and the per-invocation audit store (R3) — into each run's
    // `EngineContext` (see `ctx_builder`), so a live run is observable.
    pub usage_sink: Option<Arc<dyn agent_bus_core::UsageSink>>,
    pub revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>>,
    #[allow(dead_code)]
    pub pool: sqlx::SqlitePool,
    pub log_sink: Option<Arc<LogSinkFactory>>,
    pub audit: Option<Arc<runtime::invocation_audit::InvocationAuditStore>>,
    pub keychain: Option<Arc<dyn secrets::KeychainStore>>,
    /// The Store aggregate (occupancy<=capacity; reserve/release/commit/take).
    pub stores: Arc<StoreRepo>,
    /// The Run aggregate (generator-dry flag + completes-once guard).
    pub runs: Arc<RunStore>,
    /// The generator found-key ledger (dedup + dry detection).
    pub ledger: Arc<GeneratorLedger>,
    /// The fork/join barrier aggregate store (FanOutGroup; P1–P3).
    pub fanout: Arc<FanOutStore>,
}

/// Owns the runtime-activation lifecycle: swap the active pipeline + (re)spawn
/// the per-team worker loops at a fresh generation. Built once at boot, held in
/// Tauri state, and invoked at boot AND on project create/switch.
pub struct PipelineActivator {
    handle: AppHandle,
    runtime: Arc<RuntimeState>,
    project_store: Arc<ProjectStore>,
    tasks: Arc<TaskStore>,
    brake: Arc<Brake>,
    deps: WorkerDeps,
    /// Shared generation. `activate` bumps it; loops retire when it changes.
    generation: Arc<AtomicU64>,
}

impl PipelineActivator {
    pub fn new(
        handle: AppHandle,
        runtime: Arc<RuntimeState>,
        project_store: Arc<ProjectStore>,
        tasks: Arc<TaskStore>,
        brake: Arc<Brake>,
        deps: WorkerDeps,
    ) -> Self {
        Self {
            handle,
            runtime,
            project_store,
            tasks,
            brake,
            deps,
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Load `project_id` + its active pipeline (resolved at the root), swap the
    /// runtime's active state to it, then bump the generation and spawn one guarded
    /// worker loop per team of the NEW pipeline. Old-generation loops retire on
    /// their next poll. Idempotent under rapid re-activation (only the latest
    /// generation survives). An empty/absent project is a no-op activation that
    /// still swaps an empty ActivePipeline + retires old loops (so a stale
    /// project's loops stop even when switching to "nothing").
    pub async fn activate(&self, project_id: &str) -> Result<(), String> {
        let (active, teams) = self.resolve(project_id).await?;
        // Swap FIRST so any reader (start_run) or loop that observes the new
        // generation also observes the new active state (swap happens-before bump).
        self.runtime.activate_into(active.clone());
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // ④d: spawn the bounded-buffer engine loops for the NEW pipeline. The
        // SOURCE (generator) team gets ONE generator loop (`generate_once`);
        // every other (transformer) team gets `workers.max` worker loops
        // (`transform_once`, which routes to gate/fork/join/team). Each loop is
        // generation-guarded (retires on a newer activation) and drives the
        // project's latest active run — created by `start_run`.
        let source = source_team(&active.pipeline).map(|t| t.id.clone());
        for team in teams {
            if Some(&team.id) == source.as_ref() {
                self.spawn_generator_loop(active.clone(), team, my_gen);
            } else {
                let workers = team.workers.max.max(1);
                for _ in 0..workers {
                    self.spawn_transformer_loop(active.clone(), team.clone(), my_gen);
                }
            }
        }
        Ok(())
    }

    /// Resolve a project id into a swappable ActivePipeline + the team list to
    /// spawn loops for. Reuses the boot resolution rules (newest pipeline file).
    async fn resolve(&self, project_id: &str) -> Result<(ActivePipeline, Vec<Team>), String> {
        let empty = Pipeline {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            schema_version: pipeline::model::SCHEMA_VERSION,
            defaults: None,
            teams: vec![],
            gates: vec![],
            escalations: vec![],
            forks: vec![],
            joins: vec![],
        };
        if project_id.is_empty() {
            return Ok((
                ActivePipeline {
                    pipeline: Arc::new(empty),
                    project_id: String::new(),
                    project_root: String::new(),
                    project_target_repo: None,
                },
                vec![],
            ));
        }
        let project = self
            .project_store
            .get(&agent_bus_core::ProjectId(project_id.to_string()))
            .await
            .map_err(|e| e.to_string())?;
        let root = project.root_path.to_string_lossy().into_owned();
        let target_repo = project.target_repo.clone();
        let store = pipeline::store::PipelineStore::new(&root);
        let pipe = store
            .list_ids()
            .ok()
            .and_then(|ids| ids.into_iter().next())
            .and_then(|id| store.load(&id).ok())
            .unwrap_or(empty);
        let teams = pipe.teams.clone();
        Ok((
            ActivePipeline {
                pipeline: Arc::new(pipe),
                project_id: project.id.0,
                project_root: root,
                project_target_repo: target_repo,
            },
            teams,
        ))
    }

    /// Per-team runner selection (keychain-first, then api_key_env). On failure
    /// keep the loop alive on claude-cli rather than panicking — we never crash
    /// the whole pool over one team's runner config.
    fn runner_for_team(&self, team: &Team) -> Arc<dyn Runner> {
        let effective = team.effective_runner();
        let keychain = self.deps.keychain.clone();
        let resolve_key = |c: &pipeline::model::RunnerConfig| -> Option<String> {
            let account = c.api_key_env.as_deref().unwrap_or("anthropic-api");
            if let Some(kc) = keychain.as_ref() {
                if let Ok(k) = kc.get(secrets::api::SERVICE, account) {
                    if !k.is_empty() {
                        return Some(k);
                    }
                }
            }
            c.api_key_env.as_ref().and_then(|n| std::env::var(n).ok())
        };
        match runner_for(&effective, &resolve_key) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "app: team `{}` runner selection failed ({e}); falling back to claude-cli",
                    team.id
                );
                Arc::new(ClaudeCliRunner::new())
            }
        }
    }

    /// Spawn the SOURCE team's generator loop: poll `engine::generate_once` for the
    /// project's latest active run, loop-until-dry, with the backpressure/idle
    /// backoff. Generation-guarded.
    fn spawn_generator_loop(&self, active: ActivePipeline, team: Team, my_gen: u64) {
        let runner = self.runner_for_team(&team);
        let handle = self.handle.clone();
        let generation = self.generation.clone();
        let runs = self.deps.runs.clone();
        let project_id = active.project_id.clone();
        let ctx_builder = self.ctx_builder(active, runner);
        tauri::async_runtime::spawn(async move {
            loop {
                if generation.load(Ordering::SeqCst) != my_gen {
                    break;
                }
                let Some(run) = active_run(&runs, &project_id).await else {
                    tokio::time::sleep(LOOP_IDLE_SLEEP).await;
                    continue;
                };
                let ctx = ctx_builder(run.id.clone());
                match engine::generate_once(&ctx, &team).await {
                    Ok(StepOutcome::Generated { keys }) if !keys.is_empty() => {
                        let _ = handle.emit(crate::events::TASK_CHANGED, "generated");
                        // No sleep: keep filling downstream until backpressure/dry.
                    }
                    Ok(_) => {
                        // Idle / Backpressure / Retired / Braked: try to finish the
                        // run, then back off. (A generator that just went dry may be
                        // the last thing the run was waiting on.)
                        finish_and_emit(&ctx, &handle).await;
                        tokio::time::sleep(LOOP_IDLE_SLEEP).await;
                    }
                    Err(e) => {
                        eprintln!("app: generator loop step failed for `{}`: {e}", team.id);
                        tokio::time::sleep(LOOP_IDLE_SLEEP).await;
                    }
                }
            }
        });
    }

    /// Spawn one transformer worker loop for `team`: poll `engine::transform_once`
    /// (which routes to gate/fork/join/team) for the project's latest active run,
    /// with the backpressure/idle backoff. Generation-guarded. `workers.max` of
    /// these run per non-source team (the pool).
    fn spawn_transformer_loop(&self, active: ActivePipeline, team: Team, my_gen: u64) {
        let runner = self.runner_for_team(&team);
        let handle = self.handle.clone();
        let generation = self.generation.clone();
        let runs = self.deps.runs.clone();
        let pipeline = active.pipeline.clone();
        let project_id = active.project_id.clone();
        let ctx_builder = self.ctx_builder(active, runner);
        tauri::async_runtime::spawn(async move {
            loop {
                if generation.load(Ordering::SeqCst) != my_gen {
                    break;
                }
                let Some(run) = active_run(&runs, &project_id).await else {
                    tokio::time::sleep(LOOP_IDLE_SLEEP).await;
                    continue;
                };
                let ctx = ctx_builder(run.id.clone());
                // A transformer step. If this team feeds a fork, also expand one
                // fork item (the fork store is fed by this team's on_approve) so a
                // single pool loop keeps within-item lanes flowing.
                let progressed = match engine::transform_once(&ctx, &team).await {
                    Ok(outcome) => {
                        let settled = matches!(
                            outcome,
                            StepOutcome::Advanced { .. }
                                | StepOutcome::Failed { .. }
                                | StepOutcome::Revised { .. }
                                | StepOutcome::Escalated { .. }
                        );
                        if settled {
                            let _ = handle.emit(crate::events::TASK_CHANGED, "settled");
                            let _ = handle.emit(crate::events::USAGE_CHANGED, ());
                        }
                        settled
                    }
                    Err(e) => {
                        eprintln!("app: transformer loop step failed for `{}`: {e}", team.id);
                        false
                    }
                };
                // Expand any fork this team feeds (one item per poll), so lanes
                // flow without a dedicated fork loop.
                let mut forked = false;
                for fork in pipeline.forks.iter().filter(|f| feeds_fork(&pipeline, &team.id, &f.id)) {
                    if let Ok(StepOutcome::Advanced { .. }) = engine::fork_once(&ctx, fork).await {
                        let _ = handle.emit(crate::events::TASK_CHANGED, "forked");
                        forked = true;
                    }
                }
                if !progressed && !forked {
                    finish_and_emit(&ctx, &handle).await;
                    tokio::time::sleep(LOOP_IDLE_SLEEP).await;
                }
            }
        });
    }

    /// A cheap per-poll `EngineContext` builder closure: captures the resolved
    /// active pipeline + this loop's runner; takes the current `run_id`. Only Arc
    /// clones per poll. `self`-free so it moves into the spawned task.
    fn ctx_builder(
        &self,
        active: ActivePipeline,
        runner: Arc<dyn Runner>,
    ) -> impl Fn(String) -> EngineContext + Send + Sync + 'static {
        let stores = self.deps.stores.clone();
        let runs = self.deps.runs.clone();
        let ledger = self.deps.ledger.clone();
        let fanout = self.deps.fanout.clone();
        let tasks = self.tasks.clone();
        let brake = self.brake.clone();
        let revision_reader = self.deps.revision_reader.clone();
        let usage_sink = self.deps.usage_sink.clone();
        let log_sink = self.deps.log_sink.clone();
        let audit = self.deps.audit.clone();
        let project_root = active.project_root.clone();
        let pipeline = active.pipeline.clone();
        let target_repo: Option<std::path::PathBuf> =
            active.project_target_repo.as_deref().map(std::path::PathBuf::from);
        let read_root = project_root.clone();
        let read_prompt: Arc<dyn Fn(&Team) -> String + Send + Sync> = Arc::new(move |t: &Team| {
            std::fs::read_to_string(std::path::Path::new(&read_root).join(&t.prompt)).unwrap_or_default()
        });
        move |run_id: String| EngineContext {
            run_id,
            pipeline: pipeline.clone(),
            stores: stores.clone(),
            runs: runs.clone(),
            ledger: ledger.clone(),
            tasks: tasks.clone(),
            fanout: fanout.clone(),
            brake: brake.clone(),
            runner: runner.clone(),
            project_root: std::path::PathBuf::from(&project_root),
            target_repo: target_repo.clone(),
            read_prompt: read_prompt.clone(),
            revision_reader: revision_reader.clone(),
            usage_sink: usage_sink.clone(),
            log_sink: log_sink.clone(),
            audit: audit.clone(),
        }
    }
}

/// The project's latest active (incomplete) run, or `None` (no run started yet).
async fn active_run(runs: &RunStore, project_id: &str) -> Option<runtime::run_store::Run> {
    runs.latest_active_for_project(project_id).await.ok().flatten()
}

/// Try to complete the run; on completion emit `run-changed` + `usage-changed`.
async fn finish_and_emit(ctx: &EngineContext, handle: &AppHandle) {
    match engine::try_finish_run(ctx, &ctx.run_id).await {
        Ok(true) => {
            let _ = handle.emit(crate::events::RUN_CHANGED, ctx.run_id.clone());
            let _ = handle.emit(crate::events::USAGE_CHANGED, ());
        }
        Ok(false) => {}
        Err(e) => eprintln!("app: try_finish_run failed: {e}"),
    }
}

/// Whether `team_id` feeds `fork_id` (its on_approve targets the fork). The
/// transformer loop of that team also expands the fork it feeds.
fn feeds_fork(pipeline: &Pipeline, team_id: &str, fork_id: &str) -> bool {
    pipeline
        .teams
        .iter()
        .find(|t| t.id == team_id)
        .and_then(|t| t.outputs.on_approve.as_deref())
        == Some(fork_id)
}

/// Composition-root factory: map a team's resolved RunnerConfig to a concrete
/// Runner. claude-cli is the default and always available. anthropic-api resolves
/// its per-team API key from the keychain / `api_key_env` via the injected
/// resolver; a team requesting anthropic-api with no resolvable key yields a clear
/// RunnerError (NOT a panic) so the worker loop can surface it rather than crash.
/// Runtime depends only on `Arc<dyn Runner>` and never learns which kind it got
/// (the ACL seal).
pub(crate) fn runner_for(
    config: &pipeline::model::RunnerConfig,
    resolve_key: &dyn Fn(&pipeline::model::RunnerConfig) -> Option<String>,
) -> Result<Arc<dyn Runner>, runners::output::RunnerError> {
    use agent_bus_core::RunnerKind;
    use runners::anthropic_api::AnthropicApiRunner;
    match config.kind {
        RunnerKind::ClaudeCli => Ok(Arc::new(ClaudeCliRunner::new())),
        RunnerKind::AnthropicApi => {
            let key = resolve_key(config).ok_or_else(|| {
                runners::output::RunnerError::Other(
                    "anthropic-api runner: no API key found in the keychain or `api_key_env`"
                        .into(),
                )
            })?;
            Ok(Arc::new(AnthropicApiRunner::new(key)))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// The pure generation predicate the loop checks each iteration.
    fn still_current(my_gen: u64, shared: &AtomicU64) -> bool {
        shared.load(Ordering::SeqCst) == my_gen
    }

    #[test]
    fn loop_retires_when_generation_bumps() {
        let generation = Arc::new(AtomicU64::new(1));
        assert!(still_current(1, &generation)); // current loop keeps going
        generation.fetch_add(1, Ordering::SeqCst); // activate() bumped to 2
        assert!(!still_current(1, &generation)); // old gen-1 loop retires
        assert!(still_current(2, &generation)); // new gen-2 loop runs
    }

    #[test]
    fn activate_bumps_generation_each_call() {
        let generation = Arc::new(AtomicU64::new(0));
        let g1 = generation.fetch_add(1, Ordering::SeqCst) + 1;
        let g2 = generation.fetch_add(1, Ordering::SeqCst) + 1;
        assert_eq!((g1, g2), (1, 2));
        assert_eq!(generation.load(Ordering::SeqCst), 2);
    }
}

#[cfg(test)]
mod runner_factory_tests {
    use super::runner_for;
    use agent_bus_core::{EffortMode, RunnerKind};
    use pipeline::model::RunnerConfig;

    fn cfg(kind: RunnerKind, api_key_env: Option<&str>) -> RunnerConfig {
        RunnerConfig {
            kind,
            model: "claude-opus-4-7".into(),
            effort: EffortMode::Standard,
            api_key_env: api_key_env.map(|s| s.to_string()),
        }
    }

    // A resolver that mimics the root's env fallback (no keychain in unit tests).
    fn env_resolver(c: &RunnerConfig) -> Option<String> {
        c.api_key_env.as_ref().and_then(|n| std::env::var(n).ok())
    }

    #[test]
    fn claude_cli_kind_builds_a_runner() {
        let r = runner_for(&cfg(RunnerKind::ClaudeCli, None), &env_resolver);
        assert!(r.is_ok(), "claude-cli must always build");
    }

    #[test]
    fn anthropic_api_with_resolvable_key_builds_a_runner() {
        std::env::set_var("R1_TEST_KEY_PRESENT", "sk-test-123");
        let r = runner_for(
            &cfg(RunnerKind::AnthropicApi, Some("R1_TEST_KEY_PRESENT")),
            &env_resolver,
        );
        std::env::remove_var("R1_TEST_KEY_PRESENT");
        assert!(r.is_ok(), "anthropic-api with a resolvable key must build");
    }

    #[test]
    fn anthropic_api_resolves_via_keychain_first() {
        // A resolver that returns a key WITHOUT any env var set proves the
        // keychain-first path: the factory uses whatever the resolver yields.
        let kc_resolver = |_c: &RunnerConfig| Some("sk-from-keychain".to_string());
        let r = runner_for(&cfg(RunnerKind::AnthropicApi, None), &kc_resolver);
        assert!(r.is_ok(), "a keychain-resolved key must build even with no api_key_env");
    }

    #[test]
    fn anthropic_api_with_no_resolvable_key_is_a_clear_error_not_a_panic() {
        // Arc<dyn Runner> is not Debug, so match the Result rather than unwrap_err.
        let none_resolver = |_c: &RunnerConfig| None;
        match runner_for(&cfg(RunnerKind::AnthropicApi, None), &none_resolver) {
            Err(runners::output::RunnerError::Other(msg)) => {
                assert!(msg.to_lowercase().contains("api key"), "msg: {msg}");
            }
            Err(other) => panic!("expected Other, got {other:?}"),
            Ok(_) => panic!("expected a clear error, got a runner"),
        }
    }
}
