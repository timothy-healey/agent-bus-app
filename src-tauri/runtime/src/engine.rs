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
use crate::fanout_store::FanOutStore;
use crate::generator_ledger::GeneratorLedger;
use crate::revision::RevisionBundleReader;
use crate::run_store::RunStore;
use crate::store::StoreRepo;
use crate::task_store::TaskStore;
use pipeline::model::{Escalation, Fork, Gate, Join, Pipeline, Team};
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
    FanOut(#[from] crate::fanout_store::FanOutStoreError),
    #[error(transparent)]
    Scope(#[from] runners::scope::ScopeError),
    #[error("runner invocation failed: {0}")]
    Invoke(String),
    #[error("pipeline routing target not found: {0}")]
    NoRoute(String),
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
    /// A gate verdict / a join settlement sent the item BACK to a producing team
    /// for revision (gate `revise`, or a join's collect-all/revise-once). The
    /// item is re-queued at `producer` carrying its feedback bundle.
    Revised {
        task_id: String,
        /// The producing team the item was routed back to.
        producer: String,
    },
    /// A gate `reject` / a barrier failure routed the item to the escalation
    /// (needs-human) terminal.
    Escalated { task_id: String },
    /// A gate verdict could not be applied yet because the downstream store was
    /// full (approve backpressure) — the gated item stays put; retry later.
    GateBackpressure { task_id: String },
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
    /// The fork/join barrier aggregate store (FanOutGroup; P1–P3). REUSED
    /// unchanged for within-item parallelism (④c).
    pub fanout: Arc<FanOutStore>,
    pub brake: Arc<Brake>,
    /// The ACL seam — the engine calls invoke/invoke_stream and parses items.
    pub runner: Arc<dyn Runner>,
    pub project_root: PathBuf,
    /// Project-level `${target_repo}` default (A5); a work-item carries none in v1.
    pub target_repo: Option<PathBuf>,
    /// Reads a team's prompt file content (injected so tests don't touch disk).
    pub read_prompt: Arc<dyn Fn(&Team) -> String + Send + Sync>,
    /// Reads a task's persisted revise bundle (revise-once feedback; ④c gate
    /// revise + join revise-once). `None` = no bundle composed (fresh-run text).
    pub revision_reader: Option<Arc<dyn RevisionBundleReader>>,
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

/// The kind of pipeline node a route target id resolves to (④c). An approved
/// item's `on_approve` target may be another team (1→1, ④b), a **gate** (park in
/// the gate store, await the human verdict), a **fork** (expand into lane
/// work-items + a `FanOutGroup`), a **join** (settle a lane at the barrier), or
/// an **escalation** (terminal needs-human sink). `None` = the id matches no
/// node (a terminal `"done"`-style sink or an unknown target). The router stays
/// pure: this only classifies a single-valued target — multiplicity lives in the
/// pool/store fork/join operations (spec DDD note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteTarget {
    /// Boxed because `Team` is markedly larger than the other variants
    /// (clippy::large_enum_variant) — keeps `RouteTarget` cheap to move.
    Team(Box<Team>),
    Gate(Gate),
    Fork(Fork),
    Join(Join),
    Escalation(Escalation),
    None,
}

/// Classify a route-target stage id against the pipeline's node sets (④c).
/// Teams, gates, forks, joins, and escalations have disjoint id spaces; the
/// first matching set wins. An id matching nothing (e.g. a literal `"done"`
/// sink, or `None` on_approve already filtered upstream) is `RouteTarget::None`.
/// PURE.
pub fn resolve_target(pipeline: &Pipeline, stage_id: &str) -> RouteTarget {
    if let Some(t) = pipeline.teams.iter().find(|t| t.id == stage_id) {
        return RouteTarget::Team(Box::new(t.clone()));
    }
    if let Some(g) = pipeline.gates.iter().find(|g| g.id == stage_id) {
        return RouteTarget::Gate(g.clone());
    }
    if let Some(f) = pipeline.forks.iter().find(|f| f.id == stage_id) {
        return RouteTarget::Fork(f.clone());
    }
    if let Some(j) = pipeline.joins.iter().find(|j| j.id == stage_id) {
        return RouteTarget::Join(j.clone());
    }
    if let Some(e) = pipeline.escalations.iter().find(|e| e.id == stage_id) {
        return RouteTarget::Escalation(e.clone());
    }
    RouteTarget::None
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
    //
    //    Node-kind awareness (④c): the on_approve target may be a TEAM (the ④b
    //    1→1 case) or a GATE (a bounded store whose consumer is the human — the
    //    item is parked there in state `gated`). For a gate target we ensure the
    //    gate's store (it is not a team, so nothing else ensures it) with the
    //    default capacity before reserving — capacity = the human-backlog bound.
    //    Forks/joins/escalations are NOT a transformer's on_approve target in the
    //    assembly-line model (a fork is reached via a dedicated fork step; a join
    //    via a lane's terminal approve) — they fall through to the generic store
    //    path here, harmless if a store was ensured.
    let downstream = ctx.downstream_stage(team);
    let downstream_is_gate = downstream
        .as_deref()
        .map(|ds| matches!(resolve_target(&ctx.pipeline, ds), RouteTarget::Gate(_)))
        .unwrap_or(false);
    if let Some(ds) = &downstream {
        if downstream_is_gate {
            // The gate store is not a team store; ensure it before reserving.
            ctx.stores
                .ensure(&ctx.run_id, ds, pipeline::model::DEFAULT_STORE_CAPACITY)
                .await?;
        }
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
        let mut child = Task::work_item(
            task.project_id.clone(),
            task.pipeline.clone(),
            ctx.run_id.clone(),
            produced_key.clone(),
            ds.clone(),
            artifact.clone(),
            task.target_repo.clone(),
            now_unix(),
        );
        if downstream_is_gate {
            // A gated item waits in the gate store for the human verdict — the
            // human is the consumer (Gates-as-stores). It is NOT queued for a
            // worker; `apply_gate_verdict` later moves it on/back/out.
            child.state = TaskState::Gated;
        }
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

/// Apply a human's verdict to a GATED work-item (④c task 3 — the engine-side
/// logic; the live `approve_gate`/`revise_gate`/`reject_gate` OHS commands are
/// re-pointed at this at cutover, ④d). The task must be `gated` at a gate stage.
///
/// * **approve** → reserve a slot in the gate's `downstream` store
///   (block-before-claim); if full, leave the item gated and report
///   `GateBackpressure`. On success, commit a child work-item queued downstream,
///   mark the gated task done, and free the gate slot. → `Advanced`.
/// * **revise** → route the item BACK to its producing team's store (the team
///   whose `on_approve` targets this gate), re-queued for another pass with its
///   attempts bumped (the revision-bundle reader composes the feedback at
///   re-claim). Free the gate slot. → `Revised`.
/// * **reject** → escalate to the needs-human terminal; free the gate slot. →
///   `Escalated`.
///
/// Reuses the Store reserve/release atomic guards — no count-then-act race.
pub async fn apply_gate_verdict(
    ctx: &EngineContext,
    task_id: &str,
    verdict: agent_bus_core::Verdict,
) -> Result<StepOutcome, EngineError> {
    use agent_bus_core::Verdict;
    let mut task = ctx.tasks.get(&agent_bus_core::TaskId(task_id.to_string())).await?;
    let gate_id = task.current_stage.clone();
    // Resolve the gate this item is parked at.
    let RouteTarget::Gate(gate) = resolve_target(&ctx.pipeline, &gate_id) else {
        return Err(EngineError::NoRoute(gate_id));
    };

    match verdict {
        Verdict::Approve => {
            // Block-before-claim: reserve the gate's downstream store FIRST.
            let downstream = &gate.downstream;
            // The downstream may be a team store; ensure it bounds work even if
            // not yet ensured (a gate's downstream is a normal stage store).
            ctx.stores
                .ensure(&ctx.run_id, downstream, stage_store_capacity(ctx, downstream))
                .await?;
            if !ctx.stores.reserve(&ctx.run_id, downstream).await? {
                // Full → leave the item gated; the gate slot is NOT freed.
                return Ok(StepOutcome::GateBackpressure { task_id: task.id.0 });
            }
            // Commit the approved item downstream as a queued child work-item.
            let key = task.item_key.clone().unwrap_or_default();
            let child = Task::work_item(
                task.project_id.clone(),
                task.pipeline.clone(),
                ctx.run_id.clone(),
                key.clone(),
                downstream.clone(),
                task.parent_artifact.clone(),
                task.target_repo.clone(),
                now_unix(),
            );
            ctx.tasks.insert(&child).await?;
            // The gated item leaves the gate store: mark done + free the gate slot.
            task.state = TaskState::Done;
            task.updated_at = now_unix();
            ctx.tasks.update(&task).await?;
            ctx.stores.release(&ctx.run_id, &gate_id).await?;
            Ok(StepOutcome::Advanced {
                task_id: task.id.0,
                downstream: downstream.clone(),
                produced_keys: vec![key],
            })
        }
        Verdict::Revise => {
            // Route BACK to the producing team (the upstream whose on_approve
            // targets this gate). Re-queue a child there with attempts bumped so
            // the revision-bundle reader composes the feedback at re-claim.
            let producer = producer_of_gate(ctx, &gate_id).ok_or_else(|| EngineError::NoRoute(gate_id.clone()))?;
            ctx.stores
                .ensure(&ctx.run_id, &producer, stage_store_capacity(ctx, &producer))
                .await?;
            if !ctx.stores.reserve(&ctx.run_id, &producer).await? {
                return Ok(StepOutcome::GateBackpressure { task_id: task.id.0 });
            }
            let key = task.item_key.clone().unwrap_or_default();
            let mut child = Task::work_item(
                task.project_id.clone(),
                task.pipeline.clone(),
                ctx.run_id.clone(),
                key,
                producer.clone(),
                task.parent_artifact.clone(),
                task.target_repo.clone(),
                now_unix(),
            );
            // A revise is a re-claim: bump attempts so compose_invocation_message
            // pulls the persisted feedback bundle (revise-once feedback path).
            child.attempts = (task.attempts + 1).min(MAX_ATTEMPTS);
            ctx.tasks.insert(&child).await?;
            task.state = TaskState::Done;
            task.updated_at = now_unix();
            ctx.tasks.update(&task).await?;
            ctx.stores.release(&ctx.run_id, &gate_id).await?;
            Ok(StepOutcome::Revised { task_id: task.id.0, producer })
        }
        Verdict::Reject => {
            // Escalate to the needs-human terminal; free the gate slot.
            task.state = TaskState::NeedsHuman;
            task.current_stage = escalation_id(ctx);
            task.updated_at = now_unix();
            ctx.tasks.update(&task).await?;
            ctx.stores.release(&ctx.run_id, &gate_id).await?;
            Ok(StepOutcome::Escalated { task_id: task.id.0 })
        }
    }
}

/// The producing team that feeds a gate: the team whose `on_approve` targets the
/// gate id. PURE-ish (reads the pipeline).
fn producer_of_gate(ctx: &EngineContext, gate_id: &str) -> Option<String> {
    ctx.pipeline
        .teams
        .iter()
        .find(|t| t.outputs.on_approve.as_deref() == Some(gate_id))
        .map(|t| t.id.clone())
}

/// The pipeline's escalation (needs-human) terminal id. Falls back to the
/// conventional `"needs-human"` when no escalation node is declared.
fn escalation_id(ctx: &EngineContext) -> String {
    ctx.pipeline
        .escalations
        .first()
        .map(|e| e.id.clone())
        .unwrap_or_else(|| "needs-human".to_string())
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
    let cap = stage_store_capacity(ctx, &downstream);
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

/// The result of driving a pipeline to quiescence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuiescenceReport {
    /// Whether the run completed (generator dry + stores empty + no running).
    pub completed: bool,
    /// Whether backpressure was observed at least once during the drive (a full
    /// store turned away a generator pass or a transformer reservation).
    pub backpressure_seen: bool,
    /// How many driver rounds ran before quiescence.
    pub rounds: usize,
}

/// Drive the whole pipeline to quiescence (the in-memory pool driver — proves
/// the bounded-buffer model end-to-end without touching the live activator).
///
/// `source` is the generator (entry) team; `transformers` are every non-source
/// team, in flow order. Each round runs one generator pass then one transformer
/// step per team. The loop ends when a round makes NO progress — the generator is
/// Retired/Backpressure and every transformer is Idle/Backpressure/Braked — then
/// it tries to finish the run. A `max_rounds` bound prevents a runaway loop on a
/// logic error.
///
/// Single-threaded and deterministic: it is a test/driver harness, not the live
/// scheduler (worker pools + tokio loops land at cutover, ④d).
pub async fn run_pool_until_quiescent(
    ctx: &EngineContext,
    source: &Team,
    transformers: &[Team],
    max_rounds: usize,
) -> Result<QuiescenceReport, EngineError> {
    let mut backpressure_seen = false;
    let mut rounds = 0;
    for _ in 0..max_rounds {
        rounds += 1;
        let mut progressed = false;

        // Run the generator until it can't place more this round (filling the
        // downstream store to capacity is what surfaces backpressure): loop until
        // a pass returns Backpressure / Retired / produces nothing.
        loop {
            match generate_once(ctx, source).await? {
                StepOutcome::Generated { keys } if !keys.is_empty() => {
                    progressed = true;
                    continue;
                }
                StepOutcome::Backpressure => {
                    backpressure_seen = true;
                    break;
                }
                _ => break,
            }
        }

        // One transformer step per non-source team.
        for team in transformers {
            match transform_once(ctx, team).await? {
                StepOutcome::Advanced { .. } | StepOutcome::Failed { .. } => progressed = true,
                StepOutcome::Backpressure => backpressure_seen = true,
                _ => {}
            }
        }

        if !progressed {
            let completed = try_finish_run(ctx, &ctx.run_id).await?;
            return Ok(QuiescenceReport { completed, backpressure_seen, rounds });
        }
    }
    let completed = try_finish_run(ctx, &ctx.run_id).await?;
    Ok(QuiescenceReport { completed, backpressure_seen, rounds })
}

/// Attempt to finish the run: complete it (exactly once) iff the completion
/// precondition holds — the generator is dry AND every stage store is empty
/// (occupancy 0) AND no work-item of this run is `running`. Returns `true` iff
/// THIS call completed the run (`RunStore::try_complete`'s conditional-UPDATE
/// guard arbitrates exactly-once under any race). Returns `false` if the
/// precondition is unmet or the run was already completed.
pub async fn try_finish_run(ctx: &EngineContext, run_id: &str) -> Result<bool, EngineError> {
    // 1. The generator must be dry.
    if !ctx.runs.get(run_id).await?.generator_dry {
        return Ok(false);
    }
    // 2. Every stage store must be empty — both the teams' input stores AND the
    //    gate stores (Gates-as-stores: a gated item occupies a gate slot, so a
    //    non-empty gate store means the run is not done; ④c).
    for team in &ctx.pipeline.teams {
        if ctx.stores.occupancy(run_id, &team.id).await?.unwrap_or(0) > 0 {
            return Ok(false);
        }
    }
    for gate in &ctx.pipeline.gates {
        if ctx.stores.occupancy(run_id, &gate.id).await?.unwrap_or(0) > 0 {
            return Ok(false);
        }
    }
    // 3. No open FanOutGroup (a fork awaiting its join barrier) belongs to this
    //    run — its lanes are still in flight (④c).
    if ctx.fanout.has_open_group(run_id).await? {
        return Ok(false);
    }
    // 4. No running or gated work-item belongs to this run. A gated item is
    //    waiting on the human; the run is not complete while one exists.
    let running = ctx.tasks.list_by_state(TaskState::Running).await?;
    if running.iter().any(|t| t.run_id.as_deref() == Some(run_id)) {
        return Ok(false);
    }
    let gated = ctx.tasks.list_by_state(TaskState::Gated).await?;
    if gated.iter().any(|t| t.run_id.as_deref() == Some(run_id)) {
        return Ok(false);
    }
    // Precondition holds → complete exactly once.
    Ok(ctx.runs.try_complete(run_id).await?)
}

/// The authored input-store capacity for a stage id (a team store). Falls back
/// to `DEFAULT_STORE_CAPACITY` when the stage is not a team (e.g. a gate's
/// downstream that is a sink, or a not-yet-ensured store). Used to bound a
/// generator pass and to ensure team/gate-downstream/producer stores on demand.
fn stage_store_capacity(ctx: &EngineContext, stage: &str) -> u32 {
    ctx.pipeline
        .teams
        .iter()
        .find(|t| t.id == stage)
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

    // Compose the user message: the topic on a fresh pass, the topic + the
    // persisted revise feedback bundle on a re-claim (attempts > 1) — the gate
    // `revise` / join collect-all-revise-once feedback path. Reuses the
    // revision-bundle CONSUMER unchanged (Runtime-local seam).
    let user_message = crate::revision::compose_invocation_message(
        &task.topic,
        task.attempts,
        ctx.revision_reader.as_deref(),
        &task.id.0,
    )
    .await;

    let effective = team.effective_runner();
    let req = InvocationRequest {
        task_id: task.id.0.clone(),
        team_id: team.id.clone(),
        model: effective.model.clone(),
        thinking_budget: effective.effort.budget_tokens(),
        system_prompt,
        user_message,
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
        sqlx::query(include_str!("../../app/migrations/008_nested_groups.sql")).execute(&pool).await.unwrap();
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

    /// A pipeline carrying gates/forks/joins in addition to teams (④c tests).
    pub fn pipeline_full(
        teams: Vec<Team>,
        gates: Vec<pipeline::model::Gate>,
        forks: Vec<pipeline::model::Fork>,
        joins: Vec<pipeline::model::Join>,
    ) -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 3,
            defaults: None, teams, gates,
            escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks, joins,
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
            tasks: Arc::new(TaskStore::new(pool.clone())),
            fanout: Arc::new(FanOutStore::new(pool)),
            brake: Arc::new(Brake::new()),
            runner,
            project_root: temp_root(),
            target_repo: None,
            read_prompt: Arc::new(|_t: &Team| "system prompt".to_string()),
            revision_reader: None,
        }
    }

    /// Build a Gate node (id + downstream). Gates have no capacity field in the
    /// model; the engine uses `DEFAULT_STORE_CAPACITY` unless a test ensures the
    /// gate store with another capacity first.
    pub fn gate(id: &str, downstream: &str) -> pipeline::model::Gate {
        pipeline::model::Gate { id: id.into(), label: id.into(), downstream: downstream.into() }
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

    // ---- Task 1: route-target resolution ----

    #[test]
    fn resolve_target_classifies_each_node_kind() {
        use pipeline::model::{Fork, Join};
        let p = pipeline_full(
            vec![team("research", Some("spec"), Role::Producer, 8), team("spec", None, Role::Producer, 8)],
            vec![gate("human-gate", "spec")],
            vec![Fork { id: "fan".into(), lanes: vec!["ddd".into(), "sec".into()] }],
            vec![Join { id: "rejoin".into(), waits_for: vec!["ddd".into(), "sec".into()], downstream: "spec".into(), cancel_on_reject: false, quorum: None }],
        );
        assert!(matches!(resolve_target(&p, "research"), RouteTarget::Team(t) if t.id == "research"));
        assert!(matches!(resolve_target(&p, "human-gate"), RouteTarget::Gate(g) if g.id == "human-gate"));
        assert!(matches!(resolve_target(&p, "fan"), RouteTarget::Fork(f) if f.id == "fan"));
        assert!(matches!(resolve_target(&p, "rejoin"), RouteTarget::Join(j) if j.id == "rejoin"));
        assert!(matches!(resolve_target(&p, "needs-human"), RouteTarget::Escalation(e) if e.id == "needs-human"));
        assert_eq!(resolve_target(&p, "done"), RouteTarget::None);
        assert_eq!(resolve_target(&p, "unknown-id"), RouteTarget::None);
    }

    // ---- Task 2: gate-as-store routing ----

    #[tokio::test]
    async fn transform_to_a_gate_parks_the_item_gated_in_the_gate_store() {
        // research --on_approve--> human-gate (a gate, downstream spec).
        let p = pipeline_full(
            vec![team("research", Some("human-gate"), Role::Producer, 8), team("spec", None, Role::Producer, 8)],
            vec![gate("human-gate", "spec")],
            vec![], vec![],
        );
        let out = items_out("KEY: alpha\nARTIFACT: artifacts/research/alpha.md");
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(out))).await;
        ctx.stores.ensure(&ctx.run_id, "research", 8).await.unwrap();
        // gate store NOT pre-ensured — transform_once must ensure it.

        ctx.stores.reserve(&ctx.run_id, "research").await.unwrap();
        let item = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "alpha".into(), "research".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();

        let outcome = transform_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap();
        assert!(matches!(outcome, StepOutcome::Advanced { downstream, .. } if downstream == "human-gate"));
        // research input slot freed; the gate store now holds one gated item
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "research").await.unwrap(), Some(0));
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "human-gate").await.unwrap(), Some(1));
        // the parked item is in state Gated at the gate stage (NOT queued)
        let gated = ctx.tasks.list_by_state(TaskState::Gated).await.unwrap();
        assert_eq!(gated.len(), 1);
        assert_eq!(gated[0].current_stage, "human-gate");
        assert_eq!(gated[0].item_key.as_deref(), Some("alpha"));
        assert_eq!(ctx.tasks.list_by_state(TaskState::Queued).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn a_full_gate_store_backpressures_the_upstream() {
        let p = pipeline_full(
            vec![team("research", Some("human-gate"), Role::Producer, 8), team("spec", None, Role::Producer, 8)],
            vec![gate("human-gate", "spec")],
            vec![], vec![],
        );
        let recorder = Arc::new(FakeRunner::always(items_out("KEY: x")));
        let ctx = ctx_with(fresh_pool().await, p.clone(), recorder.clone()).await;
        ctx.stores.ensure(&ctx.run_id, "research", 8).await.unwrap();
        // gate store ensured at capacity 1 and already full
        ctx.stores.ensure(&ctx.run_id, "human-gate", 1).await.unwrap();
        ctx.stores.reserve(&ctx.run_id, "human-gate").await.unwrap();

        ctx.stores.reserve(&ctx.run_id, "research").await.unwrap();
        let item = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "y".into(), "research".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();

        let outcome = transform_once(&ctx, &ctx.pipeline.teams[0]).await.unwrap();
        assert_eq!(outcome, StepOutcome::Backpressure);
        // no claim, no run
        assert_eq!(ctx.tasks.get(&item.id).await.unwrap().state, TaskState::Queued);
        assert!(recorder.received.lock().unwrap().is_empty());
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "human-gate").await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn try_finish_does_not_complete_while_a_gated_item_exists() {
        let p = pipeline_full(
            vec![team("research", Some("human-gate"), Role::Producer, 8)],
            vec![gate("human-gate", "spec")],
            vec![], vec![],
        );
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(items_out("KEY: x")))).await;
        ctx.runs.set_generator_dry(&ctx.run_id).await.unwrap();
        // a gated item occupying the gate store
        ctx.stores.ensure(&ctx.run_id, "human-gate", 8).await.unwrap();
        ctx.stores.reserve(&ctx.run_id, "human-gate").await.unwrap();
        let mut gated = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "g".into(), "human-gate".into(), None, None, 100);
        gated.state = TaskState::Gated;
        ctx.tasks.insert(&gated).await.unwrap();
        // dry, but the gate store is non-empty AND a gated item exists
        assert!(!try_finish_run(&ctx, &ctx.run_id).await.unwrap());
    }

    // ---- Task 3: gate verdict (engine fn) ----

    async fn gated_item_ctx() -> (EngineContext, String) {
        // research --> human-gate (downstream spec). One gated item sits in the
        // gate store, having been produced by research.
        let p = pipeline_full(
            vec![team("research", Some("human-gate"), Role::Producer, 8), team("spec", None, Role::Producer, 8)],
            vec![gate("human-gate", "spec")],
            vec![], vec![],
        );
        let ctx = ctx_with(fresh_pool().await, p, Arc::new(FakeRunner::always(items_out("KEY: a")))).await;
        ctx.stores.ensure(&ctx.run_id, "human-gate", 8).await.unwrap();
        ctx.stores.reserve(&ctx.run_id, "human-gate").await.unwrap();
        let mut t = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "alpha".into(), "human-gate".into(), Some("art.md".into()), None, 100);
        t.state = TaskState::Gated;
        ctx.tasks.insert(&t).await.unwrap();
        (ctx, t.id.0)
    }

    #[tokio::test]
    async fn gate_approve_commits_downstream_and_frees_the_gate_slot() {
        let (ctx, id) = gated_item_ctx().await;
        let outcome = apply_gate_verdict(&ctx, &id, agent_bus_core::Verdict::Approve).await.unwrap();
        match outcome {
            StepOutcome::Advanced { downstream, produced_keys, .. } => {
                assert_eq!(downstream, "spec");
                assert_eq!(produced_keys, vec!["alpha".to_string()]);
            }
            other => panic!("expected Advanced, got {other:?}"),
        }
        // gate slot freed; spec store now holds the queued child
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "human-gate").await.unwrap(), Some(0));
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "spec").await.unwrap(), Some(1));
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].current_stage, "spec");
        assert_eq!(queued[0].parent_artifact.as_deref(), Some("art.md"));
        // the gated task is done
        assert_eq!(ctx.tasks.get(&agent_bus_core::TaskId(id)).await.unwrap().state, TaskState::Done);
    }

    #[tokio::test]
    async fn gate_approve_backpressures_when_downstream_full_and_keeps_item_gated() {
        let (ctx, id) = gated_item_ctx().await;
        // fill spec
        ctx.stores.ensure(&ctx.run_id, "spec", 1).await.unwrap();
        ctx.stores.reserve(&ctx.run_id, "spec").await.unwrap();
        let outcome = apply_gate_verdict(&ctx, &id, agent_bus_core::Verdict::Approve).await.unwrap();
        assert_eq!(outcome, StepOutcome::GateBackpressure { task_id: id.clone() });
        // gate slot NOT freed; item still gated
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "human-gate").await.unwrap(), Some(1));
        assert_eq!(ctx.tasks.get(&agent_bus_core::TaskId(id)).await.unwrap().state, TaskState::Gated);
    }

    #[tokio::test]
    async fn gate_revise_routes_back_to_producer_with_bumped_attempts() {
        let (ctx, id) = gated_item_ctx().await;
        let outcome = apply_gate_verdict(&ctx, &id, agent_bus_core::Verdict::Revise).await.unwrap();
        assert_eq!(outcome, StepOutcome::Revised { task_id: id.clone(), producer: "research".into() });
        // gate slot freed; research store holds the re-queued child
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "human-gate").await.unwrap(), Some(0));
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "research").await.unwrap(), Some(1));
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].current_stage, "research");
        assert_eq!(queued[0].attempts, 2, "a revise is a re-claim: attempts bumped");
        assert_eq!(ctx.tasks.get(&agent_bus_core::TaskId(id)).await.unwrap().state, TaskState::Done);
    }

    #[tokio::test]
    async fn gate_reject_escalates_to_needs_human_and_frees_the_slot() {
        let (ctx, id) = gated_item_ctx().await;
        let outcome = apply_gate_verdict(&ctx, &id, agent_bus_core::Verdict::Reject).await.unwrap();
        assert_eq!(outcome, StepOutcome::Escalated { task_id: id.clone() });
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "human-gate").await.unwrap(), Some(0));
        let t = ctx.tasks.get(&agent_bus_core::TaskId(id)).await.unwrap();
        assert_eq!(t.state, TaskState::NeedsHuman);
        assert_eq!(t.current_stage, "needs-human");
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

    // ---- Task 6: run completion ----

    #[tokio::test]
    async fn try_finish_completes_once_only_when_dry_and_empty_and_idle() {
        let p = pipeline(vec![
            team("source", Some("spec"), Role::Producer, 8),
            team("spec", None, Role::Producer, 8),
        ]);
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(items_out("KEY: x")))).await;
        ctx.stores.ensure(&ctx.run_id, "source", 8).await.unwrap();
        ctx.stores.ensure(&ctx.run_id, "spec", 8).await.unwrap();

        // not dry yet -> no completion
        assert!(!try_finish_run(&ctx, &ctx.run_id).await.unwrap());

        ctx.runs.set_generator_dry(&ctx.run_id).await.unwrap();

        // dry but a store is occupied -> no completion
        ctx.stores.reserve(&ctx.run_id, "spec").await.unwrap();
        assert!(!try_finish_run(&ctx, &ctx.run_id).await.unwrap());
        ctx.stores.release(&ctx.run_id, "spec").await.unwrap();

        // dry + empty but a running work-item exists -> no completion
        let mut running = Task::work_item("proj".into(), "p".into(), ctx.run_id.clone(), "r".into(), "spec".into(), None, None, 100);
        running.state = TaskState::Running;
        ctx.tasks.insert(&running).await.unwrap();
        assert!(!try_finish_run(&ctx, &ctx.run_id).await.unwrap());

        // drain the running item -> precondition holds -> completes once
        running.state = TaskState::Done;
        ctx.tasks.update(&running).await.unwrap();
        assert!(try_finish_run(&ctx, &ctx.run_id).await.unwrap(), "first call completes");
        assert!(ctx.runs.get(&ctx.run_id).await.unwrap().completed);
        // second call no-ops (completes-once guard)
        assert!(!try_finish_run(&ctx, &ctx.run_id).await.unwrap(), "second call no-ops");
    }

    #[tokio::test]
    async fn try_finish_ignores_running_items_of_other_runs() {
        let p = pipeline(vec![team("spec", None, Role::Producer, 8)]);
        let ctx = ctx_with(fresh_pool().await, p.clone(), Arc::new(FakeRunner::always(items_out("KEY: x")))).await;
        ctx.stores.ensure(&ctx.run_id, "spec", 8).await.unwrap();
        ctx.runs.set_generator_dry(&ctx.run_id).await.unwrap();
        // a running item belonging to a DIFFERENT run must not block this run
        let mut other = Task::work_item("proj".into(), "p".into(), "OTHER".into(), "o".into(), "spec".into(), None, None, 100);
        other.state = TaskState::Running;
        ctx.tasks.insert(&other).await.unwrap();
        assert!(try_finish_run(&ctx, &ctx.run_id).await.unwrap());
    }

    // ---- Task 7: end-to-end pool driver (the crux) ----

    #[tokio::test]
    async fn crux_generator_overfills_single_worker_drains_with_backpressure_then_completes() {
        // Pipeline: source -> spec(capacity 2) -> done(terminal).
        // The generator wants to emit M=5 > capacity candidates; the bounded
        // spec store (cap 2) forces backpressure; a single-worker spec transformer
        // drains items to the terminal `done` stage one at a time; the run
        // completes when the generator is dry and all stores are empty.
        let p = pipeline(vec![
            team("source", Some("spec"), Role::Producer, 8),
            team("spec", Some("done"), Role::Producer, 2),
            team("done", None, Role::Producer, 16),
        ]);
        // Generator emits 5 candidates the first pass (capped to free slots each
        // call), keeps offering the same 5 (deduped by the ledger), then nothing.
        let batch = "KEY: c1\nKEY: c2\nKEY: c3\nKEY: c4\nKEY: c5";
        let runner = Arc::new(FakeRunner::new(vec![
            Ok(items_out(batch)),
            Ok(items_out(batch)),
            Ok(items_out(batch)),
            Ok(items_out(batch)),
            // transformer outputs (spec/done) reuse the last response too: any
            // non-empty KEY works for a 1->1 transform. Keep emitting a batch so
            // both source passes AND transformer steps get a parseable item.
        ]));
        let ctx = ctx_with(fresh_pool().await, p.clone(), runner).await;
        ctx.stores.ensure(&ctx.run_id, "spec", 2).await.unwrap();
        ctx.stores.ensure(&ctx.run_id, "done", 16).await.unwrap();

        let source = ctx.pipeline.teams[0].clone();
        let transformers = vec![ctx.pipeline.teams[1].clone(), ctx.pipeline.teams[2].clone()];

        let report = run_pool_until_quiescent(&ctx, &source, &transformers, 200).await.unwrap();

        assert!(report.completed, "run completes when dry + empty + idle");
        assert!(report.backpressure_seen, "the bounded spec store (cap 2) applied backpressure");

        // all 5 candidates were generated exactly once (ledger dedup)
        assert_eq!(ctx.ledger.found_keys(&ctx.run_id, "source").await.unwrap().len(), 5);
        // every store drained to empty
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "spec").await.unwrap(), Some(0));
        assert_eq!(ctx.stores.occupancy(&ctx.run_id, "done").await.unwrap(), Some(0));
        // no work-item left queued or running; 5 items flowed through to Done
        assert_eq!(ctx.tasks.list_by_state(TaskState::Queued).await.unwrap().len(), 0);
        assert_eq!(ctx.tasks.list_by_state(TaskState::Running).await.unwrap().len(), 0);
        // 5 spec children + 5 done children all settled Done = 10 done tasks
        let done = ctx.tasks.list_by_state(TaskState::Done).await.unwrap();
        assert_eq!(done.len(), 10, "5 spec + 5 done work-items all settled");
        assert!(ctx.runs.get(&ctx.run_id).await.unwrap().generator_dry);
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
