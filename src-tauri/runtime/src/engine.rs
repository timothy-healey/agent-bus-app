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
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Task(#[from] crate::task_store::TaskStoreError),
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    #[error(transparent)]
    Run(#[from] crate::run_store::RunStoreError),
    #[error(transparent)]
    Ledger(#[from] crate::generator_ledger::GeneratorLedgerError),
    #[error(transparent)]
    Scope(#[from] runners::scope::ScopeError),
    #[error("runner invocation failed: {0}")]
    Invoke(String),
}

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

use crate::task::{Task, TaskState, MAX_ATTEMPTS};
use runners::output::InvocationRequest;
use runners::scope::{cleanup, prepare};
use runners::stream_json::{output_contract, parse_items, OutputItem};
use workspace::paths::PathVars;

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// The role string handed to `output_contract` for a team. Reviewer teams ask
/// the agent for a per-item verdict; everything else is a producer.
fn role_str(team: &Team) -> &'static str {
    match team.role {
        pipeline::model::Role::Reviewer => "reviewer",
        pipeline::model::Role::Producer => "producer",
    }
}

/// Run at most one transformer step for `team` using **block-before-claim**: a
/// downstream slot is reserved BEFORE the input item is claimed, so no agent run
/// is ever started that can't be placed (the backpressure barrier). On success
/// the produced item is committed into the reserved slot as a child work-item and
/// this team's own input slot is freed (the item left). On failure the reservation
/// is released and the item follows the operational-failure path (a bounded
/// synthetic revise → needs-human), mirroring `pool::process_one_claim`.
pub async fn transform_once(ctx: &EngineContext, team: &Team) -> Result<StepOutcome, EngineError> {
    // 1. BRAKE
    if ctx.brake.is_on() {
        return Ok(StepOutcome::Braked);
    }

    // 2. Determine the downstream store + RESERVE a slot there (block-before-claim).
    //    A terminal team (no on_approve) has no downstream store: items simply
    //    leave the pipeline; no reservation is needed.
    let downstream = ctx.downstream_stage(team);
    if let Some(ds) = &downstream {
        let reserved = ctx.stores.reserve(&ctx.run_id, ds).await?;
        if !reserved {
            // Full → backpressure. No claim, no run.
            return Ok(StepOutcome::Backpressure);
        }
    }

    // 3. CLAIM one input item for this stage. None → release the reservation, idle.
    let now = now_unix();
    let Some(mut task) = ctx.tasks.claim_next_for_stage(&team.id, now).await? else {
        if let Some(ds) = &downstream {
            ctx.stores.release(&ctx.run_id, ds).await?;
        }
        return Ok(StepOutcome::Idle);
    };

    // 4. BUILD invocation: system prompt = team prompt + output contract; scope
    //    grants write access to this stage's artifact dir (the L1 fix); invoke.
    let dir = artifact_dir(&team.id);
    let already_found: Vec<String> = Vec::new(); // transformers don't dedup
    let system_prompt = format!(
        "{}\n\n{}",
        (ctx.read_prompt)(team),
        output_contract(role_str(team), &dir, &already_found)
    );

    let result = invoke(ctx, team, &task, system_prompt).await;

    // 5a. FAILURE (invoke error OR no parseable items): release the downstream
    //     reservation, route onto the operational-failure path.
    let items: Vec<OutputItem> = match result {
        Ok(items) if !items.is_empty() => items,
        _ => {
            if let Some(ds) = &downstream {
                ctx.stores.release(&ctx.run_id, ds).await?;
            }
            operational_failure(ctx, &mut task).await?;
            return Ok(StepOutcome::Failed { task_id: task.id.0 });
        }
    };

    // 5b. SUCCESS. A transformer is 1→1: take the first produced item. Commit it
    //     into the reserved downstream slot as a child work-item (carrying the
    //     run, the produced/inherited key, and the artifact). Then free THIS
    //     team's own input slot (the claimed item has left this store).
    let first = &items[0];
    let produced_key = if first.key.is_empty() {
        // No KEY emitted → inherit the parent item's key (1→1 lineage).
        task.item_key.clone().unwrap_or_default()
    } else {
        first.key.clone()
    };
    let artifact = first
        .artifact_path
        .clone()
        .or_else(|| Some(artifact_path(&team.id, &produced_key, task.attempts)));

    if let Some(ds) = &downstream {
        // The slot is already reserved (step 2); committing = inserting the child
        // work-item linked to that store (occupancy already reflects the reserve).
        let child = Task::work_item(
            task.project_id.clone(),
            task.pipeline.clone(),
            ctx.run_id.clone(),
            produced_key.clone(),
            ds.clone(),
            artifact.clone(),
            task.target_repo.clone(),
            now_unix(),
        );
        ctx.tasks.insert(&child).await?;
    }

    // The claimed item is consumed: mark it Done and free this stage's input slot.
    task.parent_artifact = artifact;
    task.state = TaskState::Done;
    task.updated_at = now_unix();
    ctx.tasks.update(&task).await?;
    // Release this team's own input store slot (the item left the store). A
    // missing store (e.g. the source has none) is a harmless no-op.
    ctx.stores.release(&ctx.run_id, &team.id).await?;

    Ok(StepOutcome::Advanced {
        task_id: task.id.0,
        downstream: downstream.unwrap_or_else(|| team.id.clone()),
        produced_keys: vec![produced_key],
    })
}

/// Run at most one generator (source) pass for `source_team`, loop-until-dry +
/// capacity-bounded (the spec's source loop). The source has no input store; it
/// produces NEW items by scanning, deduped against the per-run found-key ledger.
///
/// 1. Brake check; if the run is already `generator_dry` → `Retired`.
/// 2. Compute free downstream slots K = capacity − occupancy; K==0 → `Backpressure`.
/// 3. Run the generator with `output_contract("generator", dir, &found)`.
/// 4. Filter the emitted items to keys NOT already found, cap at K, record them
///    in the ledger, and for each reserve a downstream slot + commit a work-item.
/// 5. A pass that yields 0 new keys marks the generator dry → `Retired`.
pub async fn generate_once(ctx: &EngineContext, source_team: &Team) -> Result<StepOutcome, EngineError> {
    // 1. BRAKE / already-dry
    if ctx.brake.is_on() {
        return Ok(StepOutcome::Braked);
    }
    if ctx.runs.get(&ctx.run_id).await?.generator_dry {
        return Ok(StepOutcome::Retired);
    }

    // The source pushes into its on_approve target's input store.
    let Some(downstream) = ctx.downstream_stage(source_team) else {
        // A source with no downstream can't place items; treat as dry.
        ctx.runs.set_generator_dry(&ctx.run_id).await?;
        return Ok(StepOutcome::Retired);
    };

    // 2. Free downstream slots K.
    let cap = source_team_downstream_capacity(ctx, &downstream);
    let occ = ctx.stores.occupancy(&ctx.run_id, &downstream).await?.unwrap_or(0);
    let k = cap.saturating_sub(occ);
    if k == 0 {
        return Ok(StepOutcome::Backpressure);
    }

    // 3. RUN the generator, handing it the already-found set.
    let found = ctx.ledger.found_keys(&ctx.run_id, &source_team.id).await?;
    let mut found_vec: Vec<String> = found.iter().cloned().collect();
    found_vec.sort();
    let dir = artifact_dir(&source_team.id);
    let system_prompt = format!(
        "{}\n\n{}",
        (ctx.read_prompt)(source_team),
        output_contract("generator", &dir, &found_vec)
    );
    // The generator pass uses a transient source task purely to drive one invoke.
    let pass_task = Task::work_item(
        "proj-pass".into(),
        ctx.pipeline.id.clone(),
        ctx.run_id.clone(),
        String::new(),
        source_team.id.clone(),
        None,
        ctx.target_repo.as_ref().map(|p| p.to_string_lossy().into_owned()),
        now_unix(),
    );
    let items = invoke(ctx, source_team, &pass_task, system_prompt).await?;

    // 4. Filter to NEW keys (not already found), de-duplicate within the batch,
    //    and cap at K (capacity-bounded pass).
    let mut new_keys: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = found.clone();
    for item in &items {
        if item.key.is_empty() || seen.contains(&item.key) {
            continue;
        }
        seen.insert(item.key.clone());
        new_keys.push(item.key.clone());
        if new_keys.len() as u32 >= k {
            break;
        }
    }

    // 5. No new keys → the generator is dry; retire the source.
    if new_keys.is_empty() {
        ctx.runs.set_generator_dry(&ctx.run_id).await?;
        return Ok(StepOutcome::Retired);
    }

    // Record the new keys in the ledger (dedup source of truth), then reserve +
    // commit each into the downstream store as a work-item.
    ctx.ledger.record_keys(&ctx.run_id, &source_team.id, &new_keys).await?;
    let mut committed: Vec<String> = Vec::new();
    for key in &new_keys {
        // Reserve the slot (block-before-commit). If a racing consumer filled the
        // store between the K computation and now, stop — backpressure for the
        // rest. The recorded ledger key stays (it is genuinely a found candidate);
        // a future pass will see it in `found` and skip re-emitting it.
        if !ctx.stores.reserve(&ctx.run_id, &downstream).await? {
            break;
        }
        let artifact = items
            .iter()
            .find(|i| &i.key == key)
            .and_then(|i| i.artifact_path.clone())
            .or_else(|| Some(artifact_path(&source_team.id, key, 1)));
        let child = Task::work_item(
            "proj".into(),
            ctx.pipeline.id.clone(),
            ctx.run_id.clone(),
            key.clone(),
            downstream.clone(),
            artifact,
            ctx.target_repo.as_ref().map(|p| p.to_string_lossy().into_owned()),
            now_unix(),
        );
        ctx.tasks.insert(&child).await?;
        committed.push(key.clone());
    }

    Ok(StepOutcome::Generated { keys: committed })
}

/// The capacity of the source's downstream store. Read from the (already-ensured)
/// store row when present; otherwise fall back to the authored capacity of the
/// downstream team (so a not-yet-ensured store still bounds the pass).
fn source_team_downstream_capacity(ctx: &EngineContext, downstream: &str) -> u32 {
    ctx.pipeline
        .teams
        .iter()
        .find(|t| t.id == downstream)
        .map(|t| t.store.capacity)
        .unwrap_or(pipeline::model::DEFAULT_STORE_CAPACITY)
}

/// Invoke the runner for a work-item and parse its emitted item list. Builds the
/// scope (granting the artifact-dir write access), streams nothing (engine tests
/// use non-streaming invoke), cleans up the scope file, and returns the parsed
/// items. An invoke error or unparseable output maps to `Err`.
async fn invoke(
    ctx: &EngineContext,
    team: &Team,
    task: &Task,
    system_prompt: String,
) -> Result<Vec<OutputItem>, EngineError> {
    let now = now_unix();
    let mut vars = PathVars::new(&ctx.project_root).with_task_id(&task.id.0);
    if let Some(repo) = &ctx.target_repo {
        vars = vars.with_target_repo(repo.clone());
    }
    // Grant write access to this stage's artifact dir (the L1 write-access fix):
    // inject it into the team scope's writes for this invocation.
    let mut scope = team.scope.clone();
    scope.writes.push(artifact_dir(&team.id));
    let scope_settings = prepare(&ctx.project_root, &team.id, &task.id.0, now, &scope, &vars)?;

    let effective = team.effective_runner();
    let req = InvocationRequest {
        task_id: task.id.0.clone(),
        team_id: team.id.clone(),
        model: effective.model.clone(),
        thinking_budget: effective.effort.budget_tokens(),
        system_prompt,
        user_message: task.topic.clone(),
        settings_path: scope_settings.settings_path.to_string_lossy().into_owned(),
        add_dirs: scope_settings.add_dirs.clone(),
        sandbox_profile: None,
    };

    let result = ctx.runner.invoke(&req).await;
    cleanup(&scope_settings.settings_path);
    let output = result.map_err(|e| EngineError::Invoke(e.to_string()))?;
    Ok(parse_items(&output.final_text))
}

/// The operational-failure path for a transformer (mirrors `pool.rs`'s synthetic
/// revise fallback): bump attempts; under the cap re-queue the item at the SAME
/// stage for another pass; at the cap escalate it to needs-human. Not a
/// model-produced verdict — an operational safety valve so a broken invocation
/// never strands a `running` work-item.
async fn operational_failure(ctx: &EngineContext, task: &mut Task) -> Result<(), EngineError> {
    let now = now_unix();
    if task.attempts >= MAX_ATTEMPTS {
        task.state = TaskState::NeedsHuman;
        task.current_stage = "needs-human".to_string();
    } else {
        task.attempts += 1;
        task.state = TaskState::Queued;
    }
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    Ok(())
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

    fn items_out(items: &str) -> RunnerOutput {
        RunnerOutput {
            verdict: agent_bus_core::Verdict::Approve,
            artifact_path: None,
            final_text: items.to_string(),
            usage: RunnerUsage::default(),
        }
    }

    // ---- Task 4: transformer (block-before-claim) ----

    #[tokio::test]
    async fn transform_happy_path_moves_item_downstream_and_accounts_occupancy() {
        // research -> spec. A queued item sits in research's input store.
        let p = pipeline(vec![
            team("research", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 8),
        ]);
        let out = items_out("KEY: alpha\nARTIFACT: artifacts/spec/alpha.md");
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(out))).await;
        ctx.stores.ensure(&ctx.run_id, "research", 8).await.unwrap();
        ctx.stores.ensure(&ctx.run_id, "spec", 8).await.unwrap();

        // seed: an item occupying research's input store
        ctx.stores.reserve(&ctx.run_id, "research").await.unwrap();
        let item = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "alpha".into(), "research".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();

        let outcome = transform_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap();
        match outcome {
            StepOutcome::Advanced { downstream, produced_keys, .. } => {
                assert_eq!(downstream, "spec");
                assert_eq!(produced_keys, vec!["alpha".to_string()]);
            }
            other => panic!("expected Advanced, got {other:?}"),
        }
        // research input slot freed; spec input slot occupied by the new child
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "research").await.unwrap(), Some(0));
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "spec").await.unwrap(), Some(1));
        // a child work-item exists queued at spec
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].current_stage, "spec");
        assert_eq!(queued[0].item_key.as_deref(), Some("alpha"));
        // the claimed item is Done
        assert_eq!(ctx.tasks.get(&item.id).await.unwrap().state, TaskState::Done);
    }

    #[tokio::test]
    async fn transform_full_downstream_is_backpressure_no_claim_no_run() {
        let p = pipeline(vec![
            team("research", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 1),
        ]);
        let recorder = Arc::new(FakeRunner::always(items_out("KEY: x")));
        let ctx = ctx_with(fresh_pool().await, p.clone(), recorder.clone()).await;
        ctx.stores.ensure(&ctx.run_id, "research", 8).await.unwrap();
        ctx.stores.ensure(&ctx.run_id, "spec", 1).await.unwrap();
        // fill spec to capacity
        ctx.stores.reserve(&ctx.run_id, "spec").await.unwrap();
        // a claimable item in research
        ctx.stores.reserve(&ctx.run_id, "research").await.unwrap();
        let item = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "y".into(), "research".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();

        let outcome = transform_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap();
        assert_eq!(outcome, StepOutcome::Backpressure);
        // no claim: the item is still queued
        assert_eq!(ctx.tasks.get(&item.id).await.unwrap().state, TaskState::Queued);
        // no run: the runner was never invoked
        assert!(recorder.received.lock().unwrap().is_empty());
        // spec occupancy unchanged
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "spec").await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn transform_idle_releases_reservation_when_no_input() {
        let p = pipeline(vec![
            team("research", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 8),
        ]);
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(items_out("KEY: x")))).await;
        ctx.stores.ensure(&ctx.run_id, "spec", 8).await.unwrap();

        let outcome = transform_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap();
        assert_eq!(outcome, StepOutcome::Idle);
        // the reservation taken before the (empty) claim was released
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "spec").await.unwrap(), Some(0));
    }

    #[tokio::test]
    async fn transform_failure_releases_reservation_and_routes_operational() {
        let p = pipeline(vec![
            team("research", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 8),
        ]);
        // empty final_text → parse_items yields nothing → operational failure
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(items_out("")))).await;
        ctx.stores.ensure(&ctx.run_id, "research", 8).await.unwrap();
        ctx.stores.ensure(&ctx.run_id, "spec", 8).await.unwrap();
        ctx.stores.reserve(&ctx.run_id, "research").await.unwrap();
        let item = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "z".into(), "research".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();

        let outcome = transform_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap();
        assert_eq!(outcome, StepOutcome::Failed { task_id: item.id.0.clone() });
        // downstream reservation released
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "spec").await.unwrap(), Some(0));
        // item re-queued for another attempt (under the cap)
        let reloaded = ctx.tasks.get(&item.id).await.unwrap();
        assert_eq!(reloaded.state, TaskState::Queued);
        assert_eq!(reloaded.attempts, 2);
    }

    #[tokio::test]
    async fn transform_terminal_team_needs_no_reservation() {
        // a single terminal team: items leave the pipeline, no downstream store.
        let p = pipeline(vec![team("sink", None, Role::Producer, 8)]);
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(items_out("KEY: done")))).await;
        ctx.stores.ensure(&ctx.run_id, "sink", 8).await.unwrap();
        ctx.stores.reserve(&ctx.run_id, "sink").await.unwrap();
        let item = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "done".into(), "sink".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();

        let outcome = transform_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap();
        assert!(matches!(outcome, StepOutcome::Advanced { .. }));
        // the sink's own input slot freed; no child created
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "sink").await.unwrap(), Some(0));
        assert_eq!(ctx.tasks.list_by_state(TaskState::Queued).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn transform_braked_issues_no_claim() {
        let p = pipeline(vec![
            team("research", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 8),
        ]);
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(items_out("KEY: x")))).await;
        ctx.stores.ensure(&ctx.run_id, "spec", 8).await.unwrap();
        ctx.brake.set_on("test");
        assert_eq!(transform_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap(), StepOutcome::Braked);
        // no reservation taken
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "spec").await.unwrap(), Some(0));
    }

    // ---- Task 5: generator (loop-until-dry) ----

    #[tokio::test]
    async fn generate_emits_bounded_by_free_slots_then_dedups_then_retires() {
        // source -> spec(capacity 3). Scripted: batch1 [a,b,c,d] (K caps to 3),
        // batch2 [c,d,e] (c,d already found -> only e new), then empty -> dry.
        let p = pipeline(vec![
            team("source", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 3),
        ]);
        let runner = Arc::new(FakeRunner::new(vec![
            Ok(items_out("KEY: a\nKEY: b\nKEY: c\nKEY: d")),
            Ok(items_out("KEY: c\nKEY: d\nKEY: e")),
            Ok(items_out("(nothing new)")),
        ]));
        let ctx = ctx_with(fresh_pool().await, p.clone(), runner).await;
        ctx.stores.ensure(&ctx.run_id, "spec", 3).await.unwrap();
        let source = &ctx.pipeline.teams[0];

        // pass 1: K = 3 (capacity 3 - occ 0) -> commits a,b,c (capped at K)
        let o1 = generate_once(&ctx, source).await.unwrap();
        match o1 {
            StepOutcome::Generated { keys } => assert_eq!(keys, vec!["a", "b", "c"]),
            other => panic!("expected Generated, got {other:?}"),
        }
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "spec").await.unwrap(), Some(3));

        // drain spec so there's room again (simulate a consumer)
        ctx.stores.release(&ctx.run_id, "spec").await.unwrap();
        ctx.stores.release(&ctx.run_id, "spec").await.unwrap();
        ctx.stores.release(&ctx.run_id, "spec").await.unwrap();

        // pass 2: found {a,b,c} (recorded in pass1, capped at K); batch [c,d,e]
        // -> c is deduped against the ledger, only d,e are new.
        let o2 = generate_once(&ctx, source).await.unwrap();
        match o2 {
            StepOutcome::Generated { keys } => assert_eq!(keys, vec!["d", "e"]),
            other => panic!("expected Generated, got {other:?}"),
        }

        // drain again
        for _ in 0..3 { let _ = ctx.stores.release(&ctx.run_id, "spec").await.unwrap(); }

        // pass 3: nothing new -> dry / Retired
        let o3 = generate_once(&ctx, source).await.unwrap();
        assert_eq!(o3, StepOutcome::Retired);
        assert!(ctx.runs.get(&ctx.run_id).await.unwrap().generator_dry);

        // a further pass after dry short-circuits to Retired
        assert_eq!(generate_once(&ctx, source).await.unwrap(), StepOutcome::Retired);
    }

    #[tokio::test]
    async fn generate_backpressures_when_downstream_full() {
        let p = pipeline(vec![
            team("source", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 1),
        ]);
        let runner = Arc::new(FakeRunner::always(items_out("KEY: a")));
        let ctx = ctx_with(fresh_pool().await, p.clone(), runner.clone()).await;
        ctx.stores.ensure(&ctx.run_id, "spec", 1).await.unwrap();
        ctx.stores.reserve(&ctx.run_id, "spec").await.unwrap(); // full

        let outcome = generate_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap();
        assert_eq!(outcome, StepOutcome::Backpressure);
        // no run when fully backpressured (K==0 short-circuits before invoke)
        assert!(runner.received.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn generate_dedups_against_the_ledger_across_passes() {
        let p = pipeline(vec![
            team("source", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 8),
        ]);
        // both passes emit the same key; the second must record 0 new -> dry
        let runner = Arc::new(FakeRunner::new(vec![
            Ok(items_out("KEY: only")),
            Ok(items_out("KEY: only")),
        ]));
        let ctx = ctx_with(fresh_pool().await, p.clone(), runner).await;
        ctx.stores.ensure(&ctx.run_id, "spec", 8).await.unwrap();
        let source = &ctx.pipeline.teams[0];

        assert_eq!(generate_once(&ctx, source).await.unwrap(), StepOutcome::Generated { keys: vec!["only".into()] });
        assert_eq!(generate_once(&ctx, source).await.unwrap(), StepOutcome::Retired);
        assert_eq!(ctx.ledger.found_keys(&ctx.run_id, "source").await.unwrap().len(), 1);
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
