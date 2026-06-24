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

use tauri::{AppHandle, Emitter};

use pipeline::model::{Pipeline, Team};
use runners::claude_cli::ClaudeCliRunner;
use runners::output::Runner;
use runtime::api::{ActivePipeline, RuntimeState};
use runtime::brake::Brake;
use runtime::pool::{process_one_claim, ClaimOutcome, LogSinkFactory, PoolContext};
use runtime::task_store::TaskStore;
use workspace::store::ProjectStore;

/// Collaborators a worker loop needs that are built ONCE at boot and reused
/// across activations. Cheap to clone (Arcs / Options of Arcs).
#[derive(Clone)]
pub struct WorkerDeps {
    pub usage_sink: Option<Arc<dyn agent_bus_core::UsageSink>>,
    pub revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>>,
    pub pool: sqlx::SqlitePool,
    pub log_sink: Option<Arc<LogSinkFactory>>,
    pub audit: Option<Arc<runtime::invocation_audit::InvocationAuditStore>>,
    pub keychain: Option<Arc<dyn secrets::KeychainStore>>,
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
        // Swap FIRST so any reader (inject) or loop that observes the new
        // generation also observes the new active state (swap happens-before bump).
        self.runtime.activate_into(active.clone());
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        for team in teams {
            self.spawn_team_loop(active.clone(), team, my_gen);
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

    fn spawn_team_loop(&self, active: ActivePipeline, team: Team, my_gen: u64) {
        let pipeline = active.pipeline.clone();
        let project_root = active.project_root.clone();
        let project_target_repo: Option<std::path::PathBuf> =
            active.project_target_repo.as_deref().map(std::path::PathBuf::from);
        let fanout = Arc::new(runtime::fanout_store::FanOutStore::new(self.deps.pool.clone()));

        // Per-team runner selection (keychain-first, then api_key_env). On failure
        // keep the loop alive on claude-cli rather than panicking — we never crash
        // the whole pool over one team's runner config.
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
        let runner: Arc<dyn Runner> = match runner_for(&effective, &resolve_key) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "app: team `{}` runner selection failed ({e}); falling back to claude-cli",
                    team.id
                );
                Arc::new(ClaudeCliRunner::new())
            }
        };

        let ctx = PoolContext {
            pipeline,
            runner,
            tasks: self.tasks.clone(),
            fanout,
            brake: self.brake.clone(),
            project_root: std::path::PathBuf::from(&project_root),
            project_target_repo,
            read_prompt: Arc::new({
                let root = project_root.clone();
                move |t: &Team| {
                    std::fs::read_to_string(std::path::Path::new(&root).join(&t.prompt))
                        .unwrap_or_default()
                }
            }),
            usage_sink: self.deps.usage_sink.clone(),
            revision_reader: self.deps.revision_reader.clone(),
            log_sink: self.deps.log_sink.clone(),
            audit: self.deps.audit.clone(),
            // S3: OS sandbox confinement is EXPERIMENTAL, macOS-only, Apple-
            // deprecated, and OFF by default.
            sandbox: false,
        };
        let handle = self.handle.clone();
        let generation = self.generation.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                // GENERATION GUARD: retire when a newer activation has happened.
                // Checked at the TOP of the iteration so a retired loop never
                // issues a fresh claim against the old pipeline.
                if generation.load(Ordering::SeqCst) != my_gen {
                    break;
                }
                match process_one_claim(&ctx, &team).await {
                    Ok(ClaimOutcome::Settled { task_id, .. }) => {
                        let _ = handle.emit("task.changed", task_id);
                        // A settle recorded worker usage; tell the meter to refresh.
                        let _ = handle.emit("usage.changed", ());
                    }
                    Ok(ClaimOutcome::RateLimited { .. }) => {
                        // Reactive brake (spec) — reason surfaces on the meter.
                        ctx.brake.set_on("rate-limit");
                        let _ = handle.emit("task.changed", "rate-limited");
                        let _ = handle.emit("usage.changed", ());
                    }
                    _ => {}
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }
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
