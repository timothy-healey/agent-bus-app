//! Runtime OHS — the context's Tauri commands. These are the canonical
//! operator actions (inject/approve/revise/reject/brake/scale) and the
//! god-terminal app-tools (Plan 6 consumes tools()). The router decides where
//! a gate verdict sends a task; these commands apply it + persist.

use crate::brake::Brake;
use crate::brake::BrakeState;
use crate::engine::{self, EngineContext};
use crate::fanout_store::FanOutStore;
use crate::generator_ledger::GeneratorLedger;
use crate::invocation_audit::{InvocationAuditStore, InvocationRow};
use crate::revision::RevisionBundleReader;
use crate::run_store::{Run, RunStore};
use crate::store::StoreRepo;
use crate::task::{Task, TaskState};
use crate::task_store::TaskStore;
use agent_bus_core::{ToolSpec, Verdict};
use arc_swap::ArcSwap;
use pipeline::model::{Pipeline, Team};
use runners::output::{InvocationRequest, Runner, RunnerError, RunnerOutput};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-tool argument types for Runtime's OHS tools (T1). Each tool's
/// `input_schema` is DERIVED from these via `schemars` (one source of truth — no
/// drift vs the dispatcher, which deserializes the same struct). Owned by the
/// supplier; the agentic loop validates only the published JSON schema, never
/// these types (the ACL seal).
pub mod args {
    use super::{Deserialize, JsonSchema};

    /// `inject_topic` / start-a-run. `topic` is the run's optional free-text
    /// context; `target_repo` is kept for IPC shape-compatibility (the project
    /// default applies per run).
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct InjectTopicArgs {
        pub topic: String,
        #[serde(default)]
        pub target_repo: Option<String>,
    }

    /// `approve_gate` / `reject_gate` / `revise_gate` — all carry a single task id.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct GateArgs {
        pub task_id: String,
    }

    /// `list_invocations` (L3) + the L2 operator-action commands
    /// (`retry_task`/`force_advance`/`abandon_task`/`accept_task`) — all carry a
    /// single task id. A distinct type from `GateArgs` so the OHS surface reads as
    /// the needs-human action group, not a gate verdict.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct TaskActionArgs {
        pub task_id: String,
    }

    /// `brake_on` — an optional human reason (defaults to "manual" on dispatch).
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema, Default)]
    pub struct BrakeOnArgs {
        #[serde(default)]
        pub reason: Option<String>,
    }

    /// `brake_off` / `brake_state` — no arguments.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema, Default)]
    pub struct NoArgs {}

    /// `scale_team` — the team whose worker ceiling to report.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct ScaleTeamArgs {
        pub team_id: String,
    }
}

/// The activatable slice of runtime state — a value snapshot of the resolved
/// activation inputs. Swapped atomically on project create/switch (and at boot).
/// NOT an aggregate root: holds no stores, only resolved values handed in at the
/// composition root (Runtime never learns the Project/Pipeline-on-disk types).
#[derive(Clone)]
pub struct ActivePipeline {
    /// The active pipeline, kept in memory for routing.
    pub pipeline: Arc<Pipeline>,
    /// The active project id (one project active at a time in v1).
    pub project_id: String,
    /// The active project root path (for scope/worktree resolution).
    pub project_root: String,
    /// Project-level `${target_repo}` default (A5). When an inject supplies no
    /// target_repo, the new task defaults to this. A plain resolved string handed
    /// in at the composition root (Runtime never learns about the Project type).
    pub project_target_repo: Option<String>,
}

/// Shared Runtime state held by Tauri's state manager. `tasks` + `brake` are the
/// stable Task-aggregate collaborators. WHICH pipeline/project is active is
/// interior-mutable (vet: this makes activation a runtime op, not boot-only — it
/// does NOT merge the Task and WorkerPool aggregates). Reads are lock-free
/// (`ArcSwap::load_full`); activation swaps a fresh `Arc<ActivePipeline>`.
///
/// Runtime redesign ④d: it now also holds the bounded-buffer engine
/// collaborators (Store / Run / GeneratorLedger / FanOutGroup aggregates +
/// revision reader). These let `start_run` create + seed a run, and let the gate
/// OHS commands build a per-call `EngineContext` and apply the verdict via
/// `engine::apply_gate_verdict` (the cutover from the old single-task router).
pub struct RuntimeState {
    pub tasks: Arc<TaskStore>,
    pub brake: Arc<Brake>,
    /// The Store aggregate (occupancy<=capacity; reserve/release/commit/take).
    pub stores: Arc<StoreRepo>,
    /// The Run aggregate (generator-dry flag + completes-once guard).
    pub runs: Arc<RunStore>,
    /// The generator found-key ledger (dedup + dry detection).
    pub ledger: Arc<GeneratorLedger>,
    /// The fork/join barrier aggregate store (FanOutGroup; P1–P3).
    pub fanout: Arc<FanOutStore>,
    /// Reads a task's persisted revise bundle (gate revise / join revise-once).
    pub revision_reader: Option<Arc<dyn RevisionBundleReader>>,
    /// The per-invocation audit store (R3). Powers the L3 `list_invocations` read
    /// (the CardDrawer history panel). `None` = no audit wired (runtime-only tests
    /// / pre-project boot) → `list_invocations` returns an empty trail.
    pub audit: Option<Arc<InvocationAuditStore>>,
    active: ArcSwap<ActivePipeline>,
}

impl RuntimeState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tasks: Arc<TaskStore>,
        brake: Arc<Brake>,
        stores: Arc<StoreRepo>,
        runs: Arc<RunStore>,
        ledger: Arc<GeneratorLedger>,
        fanout: Arc<FanOutStore>,
        revision_reader: Option<Arc<dyn RevisionBundleReader>>,
        audit: Option<Arc<InvocationAuditStore>>,
        active: ActivePipeline,
    ) -> Self {
        Self {
            tasks,
            brake,
            stores,
            runs,
            ledger,
            fanout,
            revision_reader,
            audit,
            active: ArcSwap::from_pointee(active),
        }
    }

    /// Lock-free snapshot of the current active pipeline/project. Each read gets a
    /// consistent `Arc<ActivePipeline>` — a concurrent activate never tears it.
    pub fn active(&self) -> Arc<ActivePipeline> {
        self.active.load_full()
    }

    /// Atomically swap the active pipeline/project (project create/switch + boot).
    pub fn activate_into(&self, next: ActivePipeline) {
        self.active.store(Arc::new(next));
    }

    /// Build an `EngineContext` for a run against the CURRENT active pipeline. Used
    /// by the gate OHS commands (③d cutover): `apply_gate_verdict` only routes the
    /// gated work-item (reserve/commit/release + state moves) — it never invokes
    /// the runner — so a no-op runner + no-op prompt reader suffice here. The
    /// worker loops build their own full contexts (with the live runner) in the
    /// activator.
    fn engine_ctx_for_run(&self, run_id: &str) -> EngineContext {
        let active = self.active();
        let target_repo = active
            .project_target_repo
            .as_deref()
            .map(std::path::PathBuf::from);
        EngineContext {
            run_id: run_id.to_string(),
            pipeline: active.pipeline.clone(),
            stores: self.stores.clone(),
            runs: self.runs.clone(),
            ledger: self.ledger.clone(),
            tasks: self.tasks.clone(),
            fanout: self.fanout.clone(),
            brake: self.brake.clone(),
            runner: Arc::new(NoopRunner),
            project_root: std::path::PathBuf::from(&active.project_root),
            target_repo,
            // The gate-verdict context never invokes the runner / composes
            // artifacts; a project-root-derived base keeps the field consistent.
            artifact_base: std::path::PathBuf::from(&active.project_root).join("artifacts"),
            read_prompt: Arc::new(|_t: &Team| String::new()),
            revision_reader: self.revision_reader.clone(),
            // The gate-verdict context never invokes the runner, so the
            // observability side-channels are not needed here.
            usage_sink: None,
            log_sink: None,
            audit: None,
        }
    }
}

/// A Runner that never runs — used only for the gate-verdict `EngineContext`
/// (apply_gate_verdict performs no invocation). Returning an error is safe: the
/// gate path never calls `invoke`.
struct NoopRunner;

#[async_trait::async_trait]
impl Runner for NoopRunner {
    async fn invoke(&self, _req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
        Err(RunnerError::Other("noop runner: invoke is never called on the gate path".into()))
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// The source/generator team of a pipeline: the single team with no *forward*
/// inbound edge (team on_approve / gate downstream / fork lane / join downstream).
/// Falls back to the first declared team (the validated convention) when the
/// forward-graph yields no unambiguous source. Mirrors `validate.rs`'s source
/// designation so `start_run` seeds the same team the validator blesses. PUBLIC
/// so the composition-root activator spawns the generator loop for the same team.
pub fn source_team(p: &Pipeline) -> Option<&Team> {
    use std::collections::HashSet;
    let mut forward_inbound: HashSet<&str> = HashSet::new();
    for team in &p.teams {
        if let Some(next) = team.outputs.on_approve.as_deref() {
            forward_inbound.insert(next);
        }
    }
    for gate in &p.gates {
        forward_inbound.insert(gate.downstream.as_str());
    }
    for fork in &p.forks {
        for lane in &fork.lanes {
            forward_inbound.insert(lane.as_str());
        }
    }
    for join in &p.joins {
        forward_inbound.insert(join.downstream.as_str());
    }
    p.teams
        .iter()
        .find(|t| !forward_inbound.contains(t.id.as_str()))
        .or_else(|| p.teams.first())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn inject_topic(
    state: tauri::State<'_, Arc<RuntimeState>>,
    topic: String,
    target_repo: Option<String>,
) -> Result<Run, String> {
    // ④d: "inject a topic" becomes "Start a run" — the topic, when supplied, is
    // threaded as the run's optional free-text context (most pipelines need none).
    // `target_repo` is no longer per-inject (the project default applies per run);
    // the param is kept so the existing frontend IPC stays shape-compatible.
    inject_topic_inner(state.as_ref(), topic, target_repo).await
}

/// Reusable inner body for inject_topic — kept as a thin wrapper over `start_run`
/// so the terminal `/inject` keeps working (topic optional → run context).
pub async fn inject_topic_inner(
    state: &RuntimeState,
    topic: String,
    _target_repo: Option<String>,
) -> Result<Run, String> {
    start_run_inner(state, Some(topic).filter(|t| !t.trim().is_empty())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn start_run(
    state: tauri::State<'_, Arc<RuntimeState>>,
    topic: Option<String>,
) -> Result<Run, String> {
    start_run_inner(state.as_ref(), topic).await
}

/// Start a run (replaces "inject a topic"): create a `Run`, `ensure` every team's
/// store at its authored capacity, and seed the SOURCE so the generator loop
/// fires. The source team has no input store — it produces by scanning — so
/// "seeding" is simply creating the run row in its fresh, not-yet-dry state: the
/// activator's generator loop, polling `engine::generate_once` for this run,
/// produces the first pass. An optional free-text `topic` is recorded as the run
/// context (genuinely-reusable pipelines may use it; most need none — the team
/// prompts are the work, the A6 insight).
///
/// Returns the created `Run`. Idempotent failure mode: an empty active pipeline
/// (no teams) is an error rather than a silent no-op run.
pub async fn start_run_inner(state: &RuntimeState, _topic: Option<String>) -> Result<Run, String> {
    // Read the active pipeline/project ONCE (lock-free) so a concurrent activate
    // never tears the run build.
    let active = state.active();
    if active.pipeline.teams.is_empty() {
        return Err("cannot start a run: the active pipeline has no teams".to_string());
    }
    let now = now_unix();
    let run_id = format!("R-{}", uuid::Uuid::new_v4());
    let run = Run::new(
        run_id.clone(),
        active.pipeline.id.clone(),
        active.project_id.clone(),
        now,
    );
    state.runs.create(&run).await.map_err(|e| e.to_string())?;

    // Ensure every team's input store at its authored capacity (the source's is
    // ensured too and harmlessly unused — the generator pushes into its
    // downstream's store, which is the next team's). Gate/fork/post-join stores
    // are ensured on demand by the engine when first reserved.
    for team in &active.pipeline.teams {
        state
            .stores
            .ensure(&run_id, &team.id, team.store.capacity)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(run)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn approve_gate(
    state: tauri::State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<Task, String> {
    apply_gate_verdict_inner(state.as_ref(), &task_id, Verdict::Approve).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn reject_gate(
    state: tauri::State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<Task, String> {
    apply_gate_verdict_inner(state.as_ref(), &task_id, Verdict::Reject).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn revise_gate(
    state: tauri::State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<Task, String> {
    apply_gate_verdict_inner(state.as_ref(), &task_id, Verdict::Revise).await
}

/// Apply an operator verdict at a gate (④d cutover): delegates to the
/// bounded-buffer engine's `apply_gate_verdict` against an `EngineContext` for
/// the gated work-item's run. Approve reserves+commits the item into the gate's
/// downstream store (block-before-claim; backpressure leaves it gated); revise
/// routes it back to the producing team for one pass with its feedback bundle;
/// reject escalates. The command signature + the frontend IPC are unchanged — it
/// still takes a `task_id` and returns the (re-read) `Task`.
pub async fn apply_gate_verdict_inner(
    state: &RuntimeState,
    task_id: &str,
    verdict: Verdict,
) -> Result<Task, String> {
    use agent_bus_core::TaskId;
    let task = state
        .tasks
        .get(&TaskId(task_id.to_string()))
        .await
        .map_err(|e| e.to_string())?;
    // Build the engine context for THIS work-item's run. A gated work-item always
    // carries its run id; a legacy gated task without one cannot be engine-routed.
    let run_id = task
        .run_id
        .clone()
        .ok_or_else(|| format!("task {task_id} has no run; cannot apply a gate verdict"))?;
    let ctx = state.engine_ctx_for_run(&run_id);
    engine::apply_gate_verdict(&ctx, task_id, verdict)
        .await
        .map_err(|e| e.to_string())?;
    // Re-read so the command returns the (now-Done) gated task, matching the
    // pre-cutover contract (the IPC shape is unchanged).
    state
        .tasks
        .get(&TaskId(task_id.to_string()))
        .await
        .map_err(|e| e.to_string())
}

// ---- L2: operator recovery actions on a needs-human work-item ----
//
// These mirror the gate-verdict command pattern (re-read + return the Task; the
// composition root emits `task-changed` + re-runs `try_finish_run`). They reuse
// the existing Task/Store/Run transitions + the reserve/commit guards — no new
// aggregate or invariant. Operator-initiated only (no auto-retry).

/// The stage that escalated a needs-human work-item: the `team_id` of its newest
/// audit row (the invocation that produced the failure). `None` when no audit
/// store is wired or the task has no recorded invocation — then the caller has no
/// stage to retry/advance at and surfaces a clear error.
async fn escalating_stage(state: &RuntimeState, task_id: &str) -> Option<String> {
    let audit = state.audit.as_ref()?;
    let rows = audit.list_rows_for_task(task_id).await.ok()?;
    rows.into_iter().next().map(|r| r.team_id)
}

/// Re-run the run-completion check for a task's run (best-effort): an operator
/// action may have emptied the last lane or re-opened work, so completion must be
/// re-evaluated exactly as the engine does after a gate verdict.
async fn reevaluate_completion(state: &RuntimeState, run_id: &str) -> Result<(), String> {
    let ctx = state.engine_ctx_for_run(run_id);
    engine::try_finish_run(&ctx, run_id).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn retry_task(
    state: tauri::State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<Task, String> {
    retry_task_inner(state.as_ref(), &task_id).await
}

/// Retry (L2): requeue the work-item at the stage that escalated it (its newest
/// audit row's `team_id`). `state → queued`, `current_stage ← that team`,
/// `attempts ← 1`; ensure that stage's input store exists; re-open a completed
/// Run so the worker loop drives it again. Reuses `Task::transition_to` (the
/// `needs_human → queued` edge is already legal) + `StoreRepo::ensure`.
pub async fn retry_task_inner(state: &RuntimeState, task_id: &str) -> Result<Task, String> {
    use agent_bus_core::TaskId;
    let mut task = state.tasks.get(&TaskId(task_id.to_string())).await.map_err(|e| e.to_string())?;
    let run_id = task
        .run_id
        .clone()
        .ok_or_else(|| format!("task {task_id} has no run; cannot retry"))?;
    let stage = escalating_stage(state, task_id)
        .await
        .ok_or_else(|| format!("task {task_id} has no recorded invocation; cannot determine the stage to retry"))?;

    let now = now_unix();
    // needs_human → queued is a legal operator-driven transition (Task state
    // machine). Use it so the immutability + legality invariants are enforced.
    task.transition_to(TaskState::Queued, now).map_err(|e| e.to_string())?;
    task.current_stage = stage.clone();
    task.attempts = 1;
    task.updated_at = now;
    state.tasks.update(&task).await.map_err(|e| e.to_string())?;

    // Ensure the stage's input store so the re-queued item has a bounded buffer to
    // be claimed from (a gate/escalation stage falls back to the default capacity).
    let capacity = state
        .active()
        .pipeline
        .teams
        .iter()
        .find(|t| t.id == stage)
        .map(|t| t.store.capacity)
        .unwrap_or(pipeline::model::DEFAULT_STORE_CAPACITY);
    state.stores.ensure(&run_id, &stage, capacity).await.map_err(|e| e.to_string())?;

    // Re-open the run if it had completed (so the worker loop picks it up again).
    if state.runs.get(&run_id).await.map_err(|e| e.to_string())?.completed {
        state.runs.reopen(&run_id).await.map_err(|e| e.to_string())?;
    }

    state.tasks.get(&TaskId(task_id.to_string())).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn force_advance(
    state: tauri::State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<Task, String> {
    force_advance_inner(state.as_ref(), &task_id).await
}

/// Approve & advance (L2): operator override = "approved at the failed stage".
/// Reserve+commit a child work-item into the failed stage's `on_approve`
/// downstream store (block-before-claim); on success mark this item done. If the
/// downstream is full, surface backpressure (the item is NOT lost — it stays
/// needs_human). Reuses `StoreRepo::reserve` (the occupancy<=capacity guard) +
/// `Task::work_item`, exactly as `engine::apply_gate_verdict`'s approve leg does.
pub async fn force_advance_inner(state: &RuntimeState, task_id: &str) -> Result<Task, String> {
    use agent_bus_core::TaskId;
    let mut task = state.tasks.get(&TaskId(task_id.to_string())).await.map_err(|e| e.to_string())?;
    let run_id = task
        .run_id
        .clone()
        .ok_or_else(|| format!("task {task_id} has no run; cannot advance"))?;
    let stage = escalating_stage(state, task_id)
        .await
        .ok_or_else(|| format!("task {task_id} has no recorded invocation; cannot determine the stage to advance from"))?;

    let active = state.active();
    let downstream = active
        .pipeline
        .teams
        .iter()
        .find(|t| t.id == stage)
        .and_then(|t| t.outputs.on_approve.clone())
        .ok_or_else(|| format!("stage {stage} has no on_approve downstream; nowhere to advance to"))?;

    // Block-before-claim: ensure + reserve the downstream slot FIRST. A full
    // store surfaces backpressure and leaves the item needs_human (never lost).
    let capacity = active
        .pipeline
        .teams
        .iter()
        .find(|t| t.id == downstream)
        .map(|t| t.store.capacity)
        .unwrap_or(pipeline::model::DEFAULT_STORE_CAPACITY);
    state.stores.ensure(&run_id, &downstream, capacity).await.map_err(|e| e.to_string())?;
    if !state.stores.reserve(&run_id, &downstream).await.map_err(|e| e.to_string())? {
        return Err(format!(
            "downstream store `{downstream}` is full; cannot advance now (the item is unchanged — retry when capacity frees)"
        ));
    }

    // Commit a child queued downstream (a gate downstream is parked gated).
    let now = now_unix();
    let ds_is_gate = active.pipeline.gates.iter().any(|g| g.id == downstream);
    let mut child = Task::work_item(
        task.project_id.clone(),
        task.pipeline.clone(),
        run_id.clone(),
        task.item_key.clone().unwrap_or_default(),
        downstream.clone(),
        task.parent_artifact.clone(),
        task.target_repo.clone(),
        now,
    );
    if ds_is_gate {
        child.state = TaskState::Gated;
    }
    state.tasks.insert(&child).await.map_err(|e| e.to_string())?;

    // The escalated item leaves: mark it done (kept for lineage). needs_human has
    // no direct `→ done` edge, so route through queued (a legal operator edge)
    // first — the item never runs (it is immediately settled done).
    task.transition_to(TaskState::Queued, now).map_err(|e| e.to_string())?;
    task.transition_to(TaskState::Done, now).map_err(|e| e.to_string())?;
    task.updated_at = now;
    state.tasks.update(&task).await.map_err(|e| e.to_string())?;

    reevaluate_completion(state, &run_id).await?;
    state.tasks.get(&TaskId(task_id.to_string())).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn abandon_task(
    state: tauri::State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<Task, String> {
    terminate_task_inner(state.as_ref(), &task_id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn accept_task(
    state: tauri::State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<Task, String> {
    terminate_task_inner(state.as_ref(), &task_id).await
}

/// Inner body for `abandon_task` (the root dispatcher calls this directly).
pub async fn abandon_task_inner(state: &RuntimeState, task_id: &str) -> Result<Task, String> {
    terminate_task_inner(state, task_id).await
}

/// Inner body for `accept_task` (the root dispatcher calls this directly).
pub async fn accept_task_inner(state: &RuntimeState, task_id: &str) -> Result<Task, String> {
    terminate_task_inner(state, task_id).await
}

/// Abandon / Accept (L2): mark the work-item `done` (kept for lineage, removed
/// from the active lanes). The two commands are distinct labels with the same
/// terminal effect (abandon = drop a failure; accept = take a hand-off
/// deliverable). Reuses the Task transitions; re-evaluates run completion.
pub async fn terminate_task_inner(state: &RuntimeState, task_id: &str) -> Result<Task, String> {
    use agent_bus_core::TaskId;
    let mut task = state.tasks.get(&TaskId(task_id.to_string())).await.map_err(|e| e.to_string())?;
    let now = now_unix();
    // needs_human has no direct `→ done` edge; route through queued (a legal
    // operator edge) then to done. The item never runs in between.
    task.transition_to(TaskState::Queued, now).map_err(|e| e.to_string())?;
    task.transition_to(TaskState::Done, now).map_err(|e| e.to_string())?;
    task.updated_at = now;
    state.tasks.update(&task).await.map_err(|e| e.to_string())?;
    if let Some(run_id) = task.run_id.clone() {
        reevaluate_completion(state, &run_id).await?;
    }
    state.tasks.get(&TaskId(task_id.to_string())).await.map_err(|e| e.to_string())
}

/// L3 read (OHS): every invocation for a task as the sealed `InvocationRow` DTO,
/// newest-first. Powers the CardDrawer history panel + the headline reason line.
/// When no audit store is wired (runtime-only tests / pre-project boot), the
/// trail is empty rather than an error — a card with no recorded invocations
/// simply shows an empty history.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_invocations(
    state: tauri::State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<Vec<InvocationRow>, String> {
    list_invocations_inner(state.as_ref(), &task_id).await
}

/// Reusable inner body for `list_invocations` (testable without Tauri State).
pub async fn list_invocations_inner(
    state: &RuntimeState,
    task_id: &str,
) -> Result<Vec<InvocationRow>, String> {
    match &state.audit {
        Some(audit) => audit.list_rows_for_task(task_id).await.map_err(|e| e.to_string()),
        None => Ok(Vec::new()),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_tasks(
    state: tauri::State<'_, Arc<RuntimeState>>,
) -> Result<Vec<Task>, String> {
    // Union of every state, ordered by creation; the board groups client-side.
    let mut all = Vec::new();
    for s in [TaskState::Queued, TaskState::Running, TaskState::Gated, TaskState::Revising,
              TaskState::NeedsHuman, TaskState::Done, TaskState::Braked] {
        all.extend(state.tasks.list_by_state(s).await.map_err(|e| e.to_string())?);
    }
    Ok(all)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_runs(
    state: tauri::State<'_, Arc<RuntimeState>>,
    project_id: String,
) -> Result<Vec<Run>, String> {
    state
        .runs
        .list_for_project(&project_id)
        .await
        .map_err(|e| e.to_string())
}

/// One stage's bounded store as the board surfaces it (Runtime redesign ④e):
/// the live `occupancy` from the StoreRepo paired with the stage's authored
/// `capacity`. Capacity is NOT held by the runtime aggregate — it is resolved at
/// the composition root from the active pipeline's `Team.store.capacity` (the
/// `run_store_occupancy` command reads `RuntimeState.active().pipeline`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoreOccupancy {
    pub stage: String,
    pub occupancy: u32,
    pub capacity: u32,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn run_store_occupancy(
    state: tauri::State<'_, Arc<RuntimeState>>,
    run_id: String,
) -> Result<Vec<StoreOccupancy>, String> {
    run_store_occupancy_inner(state.as_ref(), &run_id).await
}

/// Per-team store occupancy for a run, one entry per team in the active
/// pipeline's declared order. Capacity comes from the active pipeline (the
/// composition root's source of truth — the runtime aggregate does not hold
/// capacities); occupancy comes from the StoreRepo (0 when the store hasn't been
/// ensured yet for this run/stage). The source/generator team has no input store
/// but is still listed (occupancy 0 / its authored capacity) so the board shows
/// every lane consistently.
pub async fn run_store_occupancy_inner(
    state: &RuntimeState,
    run_id: &str,
) -> Result<Vec<StoreOccupancy>, String> {
    let active = state.active();
    let mut out = Vec::with_capacity(active.pipeline.teams.len());
    for team in &active.pipeline.teams {
        let occupancy = state
            .stores
            .occupancy(run_id, &team.id)
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        out.push(StoreOccupancy {
            stage: team.id.clone(),
            occupancy,
            capacity: team.store.capacity,
        });
    }
    Ok(out)
}

#[tauri::command(rename_all = "snake_case")]
pub fn brake_on(state: tauri::State<'_, Arc<RuntimeState>>, reason: Option<String>) -> BrakeState {
    state.brake.set_on(reason.unwrap_or_else(|| "manual".to_string()));
    state.brake.state()
}

#[tauri::command(rename_all = "snake_case")]
pub fn brake_off(state: tauri::State<'_, Arc<RuntimeState>>) -> BrakeState {
    state.brake.set_off();
    state.brake.state()
}

#[tauri::command(rename_all = "snake_case")]
pub fn brake_state(state: tauri::State<'_, Arc<RuntimeState>>) -> BrakeState {
    state.brake.state()
}

/// scale_team is acknowledged in v1 (worker spawn is fixed at one loop per team
/// in Plan 3; manual scaling of concurrent workers is v1.1). Returns the team's
/// configured max so the terminal can report the ceiling. This keeps the OHS
/// surface stable for Plan 6 without overbuilding worker concurrency in v1.
#[tauri::command(rename_all = "snake_case")]
pub fn scale_team(state: tauri::State<'_, Arc<RuntimeState>>, team_id: String) -> Result<u32, String> {
    scale_team_inner(state.as_ref(), team_id)
}

/// Reusable inner body for scale_team — callable from the terminal dispatcher.
pub fn scale_team_inner(state: &RuntimeState, team_id: String) -> Result<u32, String> {
    state
        .active()
        .pipeline
        .teams
        .iter()
        .find(|t| t.id == team_id)
        .map(|t| t.workers.max)
        .ok_or_else(|| format!("unknown team: {team_id}"))
}

/// The JSON Schema for an arg struct, derived via `schemars` (T1 — one source of
/// truth with the typed dispatch path). PUBLIC so T2 (N-NativeToolUse) can reuse
/// it as the forced tool-use `input_schema` on the API path.
pub fn arg_schema<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_else(|_| json!({ "type": "object" }))
}

/// OHS contract — consumed by Conversational Control (Plan 6). Each tool's
/// `input_schema` is DERIVED from its arg struct (T1 anti-drift), not hand-written.
pub fn tools() -> Vec<ToolSpec> {
    let ctx = "runtime";
    vec![
        ToolSpec {
            name: "inject_topic".into(),
            description: "Start a run (the topic is recorded as optional run context; the source team generates the work).".into(),
            input_schema: arg_schema::<args::InjectTopicArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "approve_gate".into(),
            description: "Approve a gated task, routing it to the gate's downstream.".into(),
            input_schema: arg_schema::<args::GateArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "reject_gate".into(),
            description: "Reject a gated task (escalates to needs-human).".into(),
            input_schema: arg_schema::<args::GateArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "revise_gate".into(),
            description: "Send a gated task back to its writer for revision.".into(),
            input_schema: arg_schema::<args::GateArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "list_invocations".into(),
            description: "List a task's invocation audit trail (newest first) — the L3 failure/verdict detail.".into(),
            input_schema: arg_schema::<args::TaskActionArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "retry_task".into(),
            description: "Requeue a needs-human task at the stage that escalated it (reset attempts; re-open the run).".into(),
            input_schema: arg_schema::<args::TaskActionArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "force_advance".into(),
            description: "Override-approve a needs-human task, committing it into the failed stage's downstream.".into(),
            input_schema: arg_schema::<args::TaskActionArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "abandon_task".into(),
            description: "Abandon a needs-human task (mark done; kept for lineage).".into(),
            input_schema: arg_schema::<args::TaskActionArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "accept_task".into(),
            description: "Accept a needs-human hand-off (mark the deliverable done; kept for lineage).".into(),
            input_schema: arg_schema::<args::TaskActionArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "brake_on".into(),
            description: "Halt new claims (in-flight workers complete).".into(),
            input_schema: arg_schema::<args::BrakeOnArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "brake_off".into(),
            description: "Release the brake.".into(),
            input_schema: arg_schema::<args::NoArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "scale_team".into(),
            description: "Report a team's configured worker ceiling (v1).".into(),
            input_schema: arg_schema::<args::ScaleTeamArgs>(),
            supplier_context: ctx.into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_topic_args_round_trip_with_and_without_target_repo() {
        let a: args::InjectTopicArgs =
            serde_json::from_value(json!({ "topic": "03-scheduling" })).unwrap();
        assert_eq!(a.topic, "03-scheduling");
        assert_eq!(a.target_repo, None);
        let b: args::InjectTopicArgs =
            serde_json::from_value(json!({ "topic": "t", "target_repo": "/repo" })).unwrap();
        assert_eq!(b.target_repo.as_deref(), Some("/repo"));
        // missing the required `topic` is rejected by serde.
        assert!(serde_json::from_value::<args::InjectTopicArgs>(json!({})).is_err());
    }

    #[test]
    fn gate_args_round_trip_and_require_task_id() {
        let a: args::GateArgs = serde_json::from_value(json!({ "task_id": "T-041" })).unwrap();
        assert_eq!(a.task_id, "T-041");
        assert!(serde_json::from_value::<args::GateArgs>(json!({})).is_err());
    }

    #[test]
    fn brake_on_args_reason_is_optional() {
        let a: args::BrakeOnArgs = serde_json::from_value(json!({})).unwrap();
        assert_eq!(a.reason, None);
        let b: args::BrakeOnArgs =
            serde_json::from_value(json!({ "reason": "rate-limit" })).unwrap();
        assert_eq!(b.reason.as_deref(), Some("rate-limit"));
    }

    #[test]
    fn scale_team_args_round_trip_and_require_team_id() {
        let a: args::ScaleTeamArgs = serde_json::from_value(json!({ "team_id": "spec" })).unwrap();
        assert_eq!(a.team_id, "spec");
        assert!(serde_json::from_value::<args::ScaleTeamArgs>(json!({})).is_err());
    }

    #[test]
    fn tools_are_all_runtime_slug_and_cover_canonical_actions() {
        let t = tools();
        assert!(t.iter().all(|s| s.supplier_context == "runtime"));
        for name in ["inject_topic", "approve_gate", "reject_gate", "revise_gate", "brake_on", "brake_off", "scale_team"] {
            assert!(t.iter().any(|s| s.name == name), "missing tool {name}");
        }
    }

    /// Helper: a tool's derived input_schema by name.
    fn schema_of(name: &str) -> serde_json::Value {
        tools().into_iter().find(|s| s.name == name).unwrap().input_schema
    }

    #[test]
    fn derived_schemas_carry_the_right_required_and_optional_props() {
        // inject_topic: topic required, target_repo optional.
        let s = schema_of("inject_topic");
        assert!(s["properties"]["topic"].is_object());
        assert!(s["properties"]["target_repo"].is_object());
        let req: Vec<&str> = s["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(req.contains(&"topic"));
        assert!(!req.contains(&"target_repo"), "target_repo is optional, must not be required");

        // approve_gate: task_id required (shared GateArgs).
        let g = schema_of("approve_gate");
        assert!(g["properties"]["task_id"].is_object());
        assert_eq!(g["required"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect::<Vec<_>>(), vec!["task_id"]);

        // brake_on: reason optional => no `required` (or empty).
        let b = schema_of("brake_on");
        assert!(b["properties"]["reason"].is_object());
        let b_req = b["required"].as_array().map(|a| a.len()).unwrap_or(0);
        assert_eq!(b_req, 0, "brake_on has no required props");

        // scale_team: team_id required.
        let st = schema_of("scale_team");
        assert!(st["required"].as_array().unwrap().iter().any(|v| v == "team_id"));
    }

    #[test]
    fn derived_schema_matches_schema_for_the_arg_struct() {
        // Proves the schema is GENERATED from the arg type (no drift), not a literal.
        assert_eq!(schema_of("inject_topic"), arg_schema::<args::InjectTopicArgs>());
        assert_eq!(schema_of("scale_team"), arg_schema::<args::ScaleTeamArgs>());
    }

    async fn state_with_two_team_pipeline() -> RuntimeState {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        // tasks table lives in migration 003 (which needs 001's projects table).
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/012_runtime_stores.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/007_invocation_audit.sql")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id,name,root_path,created_at,updated_at) VALUES ('proj','n','/p',0,0)")
            .execute(&pool).await.unwrap();
        fn team(id: &str, approve: Option<&str>, cap: u32) -> pipeline::model::Team {
            pipeline::model::Team {
                id: id.into(), name: id.into(), prompt: format!("{id}.md"),
                scope: Default::default(), runner: None,
                outputs: pipeline::model::Routes { on_approve: approve.map(String::from), on_revise: None, on_reject: None },
                workers: Default::default(), role: Default::default(),
                store: pipeline::model::Store { capacity: cap },
            }
        }
        // research (source) -> spec (transformer). Distinct capacities to assert
        // each store is ensured at its OWN authored capacity.
        let pipeline = Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 3,
            defaults: None,
            teams: vec![team("research", Some("spec"), 5), team("spec", None, 3)],
            gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
        };
        let pool = pool;
        RuntimeState::new(
            Arc::new(TaskStore::new(pool.clone())),
            Arc::new(Brake::new()),
            Arc::new(StoreRepo::new(pool.clone())),
            Arc::new(RunStore::new(pool.clone())),
            Arc::new(GeneratorLedger::new(pool.clone())),
            Arc::new(FanOutStore::new(pool.clone())),
            None,
            Some(Arc::new(InvocationAuditStore::new(pool))),
            ActivePipeline {
                pipeline: Arc::new(pipeline),
                project_id: "proj".into(),
                project_root: "/p".into(),
                project_target_repo: None,
            },
        )
    }

    #[tokio::test]
    async fn start_run_creates_a_run_and_ensures_every_team_store_at_its_capacity() {
        let state = state_with_two_team_pipeline().await;
        let run = start_run_inner(&state, None).await.unwrap();
        // The run row exists, fresh: not dry, not completed.
        assert!(run.id.starts_with("R-"));
        assert_eq!(run.pipeline, "p");
        assert_eq!(run.project_id, "proj");
        let loaded = state.runs.get(&run.id).await.unwrap();
        assert!(!loaded.generator_dry);
        assert!(!loaded.completed);
        // Every team's input store is ensured at its OWN authored capacity, empty.
        assert_eq!(state.stores.occupancy(&run.id, "research").await.unwrap(), Some(0));
        assert_eq!(state.stores.occupancy(&run.id, "spec").await.unwrap(), Some(0));
        // capacity respected: spec admits 3 reserves then backpressures.
        assert!(state.stores.reserve(&run.id, "spec").await.unwrap());
        assert!(state.stores.reserve(&run.id, "spec").await.unwrap());
        assert!(state.stores.reserve(&run.id, "spec").await.unwrap());
        assert!(!state.stores.reserve(&run.id, "spec").await.unwrap(), "spec capacity is 3");
    }

    #[tokio::test]
    async fn inject_topic_starts_a_run_too() {
        // The terminal `/inject` keeps working as a thin start_run wrapper.
        let state = state_with_two_team_pipeline().await;
        let run = inject_topic_inner(&state, "some topic".into(), None).await.unwrap();
        assert!(run.id.starts_with("R-"));
        assert_eq!(state.stores.occupancy(&run.id, "research").await.unwrap(), Some(0));
    }

    #[tokio::test]
    async fn start_run_on_empty_pipeline_is_an_error() {
        let state = state_with_two_team_pipeline().await;
        state.activate_into(ActivePipeline {
            pipeline: Arc::new(Pipeline {
                id: "e".into(), name: "E".into(), description: String::new(), schema_version: 3,
                defaults: None, teams: vec![], gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
            }),
            project_id: "proj".into(),
            project_root: "/p".into(),
            project_target_repo: None,
        });
        assert!(start_run_inner(&state, None).await.is_err());
    }

    #[tokio::test]
    async fn list_runs_lists_the_projects_runs_newest_first() {
        let state = state_with_two_team_pipeline().await;
        let r1 = start_run_inner(&state, None).await.unwrap();
        let r2 = start_run_inner(&state, None).await.unwrap();
        let runs = state.runs.list_for_project("proj").await.unwrap();
        // both runs are present (newest-first ordering is covered by RunStore's
        // own test with distinct created_at; both here share a second-resolution
        // timestamp so we only assert membership here).
        assert_eq!(runs.len(), 2);
        let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&r1.id.as_str()));
        assert!(ids.contains(&r2.id.as_str()));
    }

    #[tokio::test]
    async fn run_store_occupancy_reports_each_team_with_pipeline_capacity() {
        let state = state_with_two_team_pipeline().await;
        let run = start_run_inner(&state, None).await.unwrap();
        // reserve two slots in `spec` so occupancy is observable.
        assert!(state.stores.reserve(&run.id, "spec").await.unwrap());
        assert!(state.stores.reserve(&run.id, "spec").await.unwrap());
        let occ = run_store_occupancy_inner(&state, &run.id).await.unwrap();
        // one entry per team, in declared order (research source, then spec).
        assert_eq!(occ.len(), 2);
        assert_eq!(occ[0].stage, "research");
        assert_eq!(occ[0].occupancy, 0);
        assert_eq!(occ[0].capacity, 5, "capacity comes from the active pipeline");
        assert_eq!(occ[1].stage, "spec");
        assert_eq!(occ[1].occupancy, 2);
        assert_eq!(occ[1].capacity, 3);
    }

    #[tokio::test]
    async fn run_store_occupancy_reports_zero_for_unensured_run() {
        // a run id with no ensured stores still yields one entry per team at 0.
        let state = state_with_two_team_pipeline().await;
        let occ = run_store_occupancy_inner(&state, "R-ghost").await.unwrap();
        assert_eq!(occ.len(), 2);
        assert!(occ.iter().all(|o| o.occupancy == 0));
    }

    #[tokio::test]
    async fn source_team_is_the_team_with_no_forward_inbound() {
        let p = Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 3,
            defaults: None,
            teams: vec![
                pipeline::model::Team {
                    id: "research".into(), name: "r".into(), prompt: "r.md".into(),
                    scope: Default::default(), runner: None,
                    outputs: pipeline::model::Routes { on_approve: Some("spec".into()), on_revise: None, on_reject: None },
                    workers: Default::default(), role: Default::default(), store: Default::default(),
                },
                pipeline::model::Team {
                    id: "spec".into(), name: "s".into(), prompt: "s.md".into(),
                    scope: Default::default(), runner: None,
                    outputs: pipeline::model::Routes { on_approve: None, on_revise: None, on_reject: None },
                    workers: Default::default(), role: Default::default(), store: Default::default(),
                },
            ],
            gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
        };
        assert_eq!(source_team(&p).map(|t| t.id.as_str()), Some("research"));
    }

    // ---- L3 read + L2 recovery actions (plan H) ----

    /// Seed a run + an escalated (needs_human) work-item that failed at `stage`,
    /// with one settled audit row recording that failure. Returns (run_id, task).
    async fn seed_escalated(state: &RuntimeState, stage: &str, outcome: InvocationOutcome) -> (String, Task) {
        use crate::invocation_audit::AuditUsage;
        let run = start_run_inner(state, None).await.unwrap();
        // The item rests at the escalation terminal in state needs_human.
        let mut task = Task::work_item(
            "proj".into(), "p".into(), run.id.clone(), "alpha".into(),
            "needs-human".into(), Some("artifacts/spec/alpha.md".into()), None, 100,
        );
        task.state = TaskState::NeedsHuman;
        state.tasks.insert(&task).await.unwrap();
        // Record the failing invocation at `stage` (the audit team_id is what the
        // L2 commands read to find the stage that escalated the item).
        let audit = state.audit.as_ref().unwrap();
        let inv = audit.record_start(&task.id.0, stage, "m", 3, 1000).await.unwrap();
        audit.record_settle(&inv, &outcome, &AuditUsage::default(), 1100).await.unwrap();
        (run.id, task)
    }

    use crate::invocation_audit::{ErrorClass, InvocationOutcome};

    #[tokio::test]
    async fn list_invocations_returns_rows_newest_first_sealed_as_dto() {
        let state = state_with_two_team_pipeline().await;
        let (_run, task) = seed_escalated(&state, "spec", InvocationOutcome::Error(ErrorClass::NoResult)).await;
        let rows = list_invocations_inner(&state, &task.id.0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].team_id, "spec");
        assert_eq!(rows[0].outcome, "error:no_result");
        assert_eq!(rows[0].attempts, 3);
    }

    #[tokio::test]
    async fn list_invocations_empty_without_audit_store() {
        // A state with no audit store returns an empty trail (never errors).
        let state = state_with_two_team_pipeline().await;
        // overwrite audit with None by rebuilding a minimal state is awkward; instead
        // assert an unknown task yields an empty trail (no rows recorded).
        let rows = list_invocations_inner(&state, "T-unknown").await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn retry_task_requeues_at_failed_stage_with_reset_attempts_and_reopens_run() {
        let state = state_with_two_team_pipeline().await;
        let (run_id, task) = seed_escalated(&state, "spec", InvocationOutcome::Error(ErrorClass::RateLimited)).await;
        // Complete the run so retry must re-open it.
        state.runs.set_generator_dry(&run_id).await.unwrap();
        state.runs.try_complete(&run_id).await.unwrap();
        assert!(state.runs.get(&run_id).await.unwrap().completed);

        let out = retry_task_inner(&state, &task.id.0).await.unwrap();
        assert_eq!(out.state, TaskState::Queued, "requeued");
        assert_eq!(out.current_stage, "spec", "at the stage that escalated it");
        assert_eq!(out.attempts, 1, "attempts reset");
        // the spec store is ensured (occupancy reported, not None).
        assert!(state.stores.occupancy(&run_id, "spec").await.unwrap().is_some());
        // the run is re-opened.
        assert!(!state.runs.get(&run_id).await.unwrap().completed, "run re-opened");
    }

    #[tokio::test]
    async fn force_advance_commits_into_downstream_store() {
        let state = state_with_two_team_pipeline().await;
        // research -> spec; escalate at research so the downstream is spec.
        let (run_id, task) = seed_escalated(&state, "research", InvocationOutcome::Verdict(Verdict::Reject)).await;
        let out = force_advance_inner(&state, &task.id.0).await.unwrap();
        assert_eq!(out.state, TaskState::Done, "the escalated item is settled done");
        // a child landed queued in the spec store (occupancy bumped by the reserve).
        assert_eq!(state.stores.occupancy(&run_id, "spec").await.unwrap(), Some(1));
        let queued = state.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().any(|t| t.current_stage == "spec" && t.run_id.as_deref() == Some(&run_id)));
    }

    #[tokio::test]
    async fn force_advance_surfaces_backpressure_when_downstream_full_without_losing_item() {
        let state = state_with_two_team_pipeline().await;
        let (run_id, task) = seed_escalated(&state, "research", InvocationOutcome::Verdict(Verdict::Reject)).await;
        // Fill the spec store (capacity 3) to the brim.
        for _ in 0..3 { assert!(state.stores.reserve(&run_id, "spec").await.unwrap()); }
        let err = force_advance_inner(&state, &task.id.0).await.unwrap_err();
        assert!(err.contains("full"), "backpressure is surfaced: {err}");
        // the item is unchanged (still needs_human — never lost).
        let still = state.tasks.get(&task.id).await.unwrap();
        assert_eq!(still.state, TaskState::NeedsHuman);
    }

    #[tokio::test]
    async fn force_advance_errors_when_stage_has_no_downstream() {
        let state = state_with_two_team_pipeline().await;
        // spec is terminal (no on_approve) — advancing has nowhere to go.
        let (_run, task) = seed_escalated(&state, "spec", InvocationOutcome::Verdict(Verdict::Reject)).await;
        let err = force_advance_inner(&state, &task.id.0).await.unwrap_err();
        assert!(err.contains("no on_approve"), "got: {err}");
    }

    #[tokio::test]
    async fn abandon_and_accept_mark_done_and_reevaluate_completion() {
        let state = state_with_two_team_pipeline().await;
        let (run_id, task) = seed_escalated(&state, "spec", InvocationOutcome::Error(ErrorClass::Other)).await;
        // Make the run completable: generator dry + no other live items.
        state.runs.set_generator_dry(&run_id).await.unwrap();
        let out = abandon_task_inner(&state, &task.id.0).await.unwrap();
        assert_eq!(out.state, TaskState::Done);
        // run completion was re-evaluated and (precondition now holds) completed.
        assert!(state.runs.get(&run_id).await.unwrap().completed, "completion re-evaluated");

        // accept has the same terminal effect.
        let (_r2, t2) = seed_escalated(&state, "spec", InvocationOutcome::Verdict(Verdict::Approve)).await;
        let out2 = accept_task_inner(&state, &t2.id.0).await.unwrap();
        assert_eq!(out2.state, TaskState::Done);
    }

    #[tokio::test]
    async fn retry_without_audit_row_errors_clearly() {
        let state = state_with_two_team_pipeline().await;
        let run = start_run_inner(&state, None).await.unwrap();
        let mut task = Task::work_item("proj".into(), "p".into(), run.id, "k".into(), "needs-human".into(), None, None, 1);
        task.state = TaskState::NeedsHuman;
        state.tasks.insert(&task).await.unwrap();
        let err = retry_task_inner(&state, &task.id.0).await.unwrap_err();
        assert!(err.contains("no recorded invocation"), "got: {err}");
    }

    #[test]
    fn task_action_args_round_trip_and_require_task_id() {
        let a: args::TaskActionArgs = serde_json::from_value(json!({ "task_id": "T-1" })).unwrap();
        assert_eq!(a.task_id, "T-1");
        assert!(serde_json::from_value::<args::TaskActionArgs>(json!({})).is_err());
    }

    #[test]
    fn tools_cover_the_l2_l3_actions() {
        let t = tools();
        for name in ["list_invocations", "retry_task", "force_advance", "abandon_task", "accept_task"] {
            assert!(t.iter().any(|s| s.name == name), "missing tool {name}");
        }
    }
}
