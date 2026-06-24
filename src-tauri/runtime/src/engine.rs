//! Engine — the bounded-buffer assembly-line execution core (Runtime redesign
//! ④b). A NEW, parallel runtime built on the ④a aggregates (`StoreRepo`,
//! `RunStore`, `GeneratorLedger`) + the work-item `Task`. It is **additive and
//! unwired**: the existing single-task `pool.rs` runtime stays untouched and
//! green, and nothing here is referenced by the activator / composition root.
//! Cutover is a later plan (④d).
//!
//! The model (see the bounded-buffer spec):
//!   * Stages are connected by capacity-limited **stores**; a full store applies
//!     backpressure to its upstream.
//!   * A **transformer** worker reserves a downstream slot BEFORE claiming its
//!     input item (block-before-claim) — no agent run is started that can't be
//!     placed.
//!   * The **generator** (source) has no input store; it produces items in
//!     capacity-bounded passes, loop-until-dry, deduping against the ledger.
//!   * A **run completes** when the generator is dry AND all stores are empty AND
//!     no workers are running — guarded exactly-once by `RunStore::try_complete`.
//!
//! The Runner trait is UNCHANGED: the engine calls `Runner::invoke`/`invoke_stream`
//! and parses `final_text` with the new `parse_items`, composing `output_contract`
//! into the system prompt (the L1 fix).

use crate::brake::Brake;
use crate::generator_ledger::GeneratorLedger;
use crate::run_store::RunStore;
use crate::store::StoreRepo;
use crate::task_store::TaskStore;
use pipeline::model::{Pipeline, Team};
use runners::output::Runner;
use std::path::PathBuf;
use std::sync::Arc;

/// What one engine step did — surfaced so the driver + tests can assert behaviour.
/// Mirrors the spec's worker-loop outcomes (idle / backpressure / settled / dry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// No queued input item for this stage (transformer) — nothing to do.
    Idle,
    /// Brake is on; no claim / no run issued.
    Braked,
    /// The downstream store is full — backpressure. No input claimed, no run.
    Backpressure,
    /// A transformer moved an item downstream (committed into the reserved slot).
    Advanced {
        /// The work-item that was processed.
        task_id: String,
        /// The downstream stage the produced item landed in.
        downstream: String,
        /// The key(s) of the produced child item(s).
        produced_keys: Vec<String>,
    },
    /// A transformer's invocation failed / parsed empty: the reservation was
    /// released and the item routed onto the operational-failure path (mirroring
    /// the existing pool's synthetic-revise fallback).
    Failed { task_id: String },
    /// A generator pass committed new items downstream.
    Generated {
        /// The new candidate keys committed this pass (bounded by free slots K).
        keys: Vec<String>,
    },
    /// The generator is dry (a pass yielded no new keys) — the source loop retires.
    Retired,
}

/// The collaborators one engine step needs. Cheap to clone (Arcs). The
/// bounded-buffer dual of `PoolContext`: it bundles the ④a aggregates plus the
/// `Runner` ACL seam, the pipeline, and the project/repo roots + a prompt reader.
#[derive(Clone)]
pub struct EngineContext {
    /// The run this engine instance is driving.
    pub run_id: String,
    pub pipeline: Arc<Pipeline>,
    /// The Store aggregate (occupancy<=capacity; reserve/release/commit/take).
    pub stores: Arc<StoreRepo>,
    /// The Run aggregate (generator-dry flag + completes-once guard).
    pub runs: Arc<RunStore>,
    /// The generator found-key ledger (dedup + dry detection).
    pub ledger: Arc<GeneratorLedger>,
    /// The Task (work-item) store.
    pub tasks: Arc<TaskStore>,
    pub brake: Arc<Brake>,
    /// The ACL seam — the engine calls invoke/invoke_stream and parses items.
    pub runner: Arc<dyn Runner>,
    pub project_root: PathBuf,
    /// Project-level `${target_repo}` default (A5); a work-item carries none in v1.
    pub target_repo: Option<PathBuf>,
    /// Reads a team's prompt file content (injected so tests don't touch disk).
    pub read_prompt: Arc<dyn Fn(&Team) -> String + Send + Sync>,
}

impl EngineContext {
    /// The downstream stage an item leaving `team` flows into: the team's
    /// `on_approve` target's input store (a team or a gate). `None` for a
    /// terminal team (no `on_approve` — the item leaves the pipeline). PURE.
    pub fn downstream_stage(&self, team: &Team) -> Option<String> {
        team.outputs.on_approve.clone()
    }

    /// Whether `stage` is a terminal sink (no further store to push into): a
    /// team with no `on_approve`, or the literal `"done"` sink. PURE.
    pub fn is_terminal(&self, team: &Team) -> bool {
        team.outputs.on_approve.is_none()
    }
}

/// The artifact path for a work-item's output, under `${project}/artifacts/...`.
/// The engine grants the agent write access to this location (the L1 fix). The
/// `${project}` token is resolved by the Scope layer at invocation time; here we
/// compose the stable relative shape `${project}/artifacts/<stage>/<key>-v<attempt>.md`.
/// `key` is sanitised so a path-like candidate key (e.g. `src/foo.rs`) is a single
/// safe path segment. PURE.
pub fn artifact_path(stage: &str, key: &str, attempt: u32) -> String {
    let safe_key = sanitize_key(key);
    format!("${{project}}/artifacts/{stage}/{safe_key}-v{attempt}.md")
}

/// The directory artifacts for `stage` live under (handed to `output_contract`
/// and granted write access). PURE.
pub fn artifact_dir(stage: &str) -> String {
    format!("${{project}}/artifacts/{stage}")
}

/// Turn a candidate key into one safe path segment: non-alphanumeric runs become
/// a single `-`, trimmed. An empty result falls back to `item`.
fn sanitize_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut last_dash = false;
    for c in key.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "item".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared in-memory `EngineContext` builder for the engine tests (tasks 3–7).
    use super::*;
    use agent_bus_core::{EffortMode, RunnerKind};
    use pipeline::model::{Role, Routes, Scope, Store, TeamRunnerConfig, Workers};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    pub async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/012_runtime_stores.sql")).execute(&pool).await.unwrap();
        pool
    }

    pub fn team(id: &str, approve: Option<&str>, role: Role, capacity: u32) -> Team {
        Team {
            id: id.into(),
            name: id.into(),
            prompt: format!("prompts/{id}.md"),
            runner: Some(TeamRunnerConfig {
                kind: Some(RunnerKind::ClaudeCli),
                model: Some("claude-opus-4-7".into()),
                effort: Some(EffortMode::Standard),
                api_key_env: None,
            }),
            scope: Scope { reads: vec![], writes: vec![], tools: vec!["Read".into(), "Write".into()] },
            outputs: Routes { on_approve: approve.map(String::from), on_revise: None, on_reject: Some("needs-human".into()) },
            workers: Workers::default(),
            role,
            store: Store { capacity },
        }
    }

    pub fn pipeline(teams: Vec<Team>) -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 3,
            defaults: None, teams, gates: vec![],
            escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![], joins: vec![],
        }
    }

    pub fn temp_root() -> PathBuf {
        let d = std::env::temp_dir().join(format!("abp-engine-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Build an EngineContext over a fresh pool + the given pipeline + runner, and
    /// create the run row. Stores are NOT ensured here — each test ensures the
    /// stores it needs with the capacities it wants.
    pub async fn ctx_with(pool: SqlitePool, pipeline: Pipeline, runner: Arc<dyn Runner>) -> EngineContext {
        let runs = Arc::new(RunStore::new(pool.clone()));
        runs.create(&crate::run_store::Run::new("R1".into(), "p".into(), "proj".into(), 100)).await.unwrap();
        EngineContext {
            run_id: "R1".into(),
            pipeline: Arc::new(pipeline),
            stores: Arc::new(StoreRepo::new(pool.clone())),
            runs,
            ledger: Arc::new(GeneratorLedger::new(pool.clone())),
            tasks: Arc::new(TaskStore::new(pool)),
            brake: Arc::new(Brake::new()),
            runner,
            project_root: temp_root(),
            target_repo: None,
            read_prompt: Arc::new(|_t: &Team| "system prompt".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use pipeline::model::Role;
    use runners::fake::FakeRunner;
    use runners::output::{RunnerOutput, RunnerUsage};

    fn approve_out() -> RunnerOutput {
        RunnerOutput { verdict: agent_bus_core::Verdict::Approve, artifact_path: None, final_text: String::new(), usage: RunnerUsage::default() }
    }

    #[tokio::test]
    async fn downstream_stage_is_the_on_approve_target() {
        let p = pipeline(vec![
            team("research", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 8),
        ]);
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(approve_out()))).await;
        assert_eq!(ctx.downstream_stage(&ctx.pipeline.teams[0]).as_deref(), Some("spec"));
        assert_eq!(ctx.downstream_stage(&ctx.pipeline.teams[1]), None);
        assert!(!ctx.is_terminal(&ctx.pipeline.teams[0]));
        assert!(ctx.is_terminal(&ctx.pipeline.teams[1]));
    }

    #[test]
    fn artifact_path_is_under_project_artifacts_with_stage_and_key() {
        let p = artifact_path("spec", "alpha", 1);
        assert_eq!(p, "${project}/artifacts/spec/alpha-v1.md");
    }

    #[test]
    fn artifact_path_sanitizes_path_like_keys_to_one_segment() {
        let p = artifact_path("research", "src/foo/bar.rs", 2);
        assert_eq!(p, "${project}/artifacts/research/src-foo-bar-rs-v2.md");
    }

    #[test]
    fn artifact_dir_matches_the_path_prefix() {
        assert_eq!(artifact_dir("spec"), "${project}/artifacts/spec");
    }

    #[test]
    fn sanitize_key_falls_back_to_item_for_empty() {
        assert_eq!(sanitize_key(""), "item");
        assert_eq!(sanitize_key("///"), "item");
        assert_eq!(sanitize_key("a.b"), "a-b");
    }
}
