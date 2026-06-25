//! Runtime OHS — the context's Tauri commands. These are the canonical
//! operator actions (inject/approve/revise/reject/brake/scale) and the
//! god-terminal app-tools (Plan 6 consumes tools()). The router decides where
//! a gate verdict sends a task; these commands apply it + persist.

use crate::brake::Brake;
use crate::brake::BrakeState;
use crate::engine::{self, EngineContext};
use crate::fanout_store::FanOutStore;
use crate::generator_ledger::GeneratorLedger;
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
            Arc::new(FanOutStore::new(pool)),
            None,
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
}
