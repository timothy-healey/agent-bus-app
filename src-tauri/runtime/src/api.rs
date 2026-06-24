//! Runtime OHS — the context's Tauri commands. These are the canonical
//! operator actions (inject/approve/revise/reject/brake/scale) and the
//! god-terminal app-tools (Plan 6 consumes tools()). The router decides where
//! a gate verdict sends a task; these commands apply it + persist.

use crate::brake::{Brake, BrakeState};
use crate::router::route;
use crate::task::{Task, TaskState};
use crate::task_store::TaskStore;
use agent_bus_core::{ToolSpec, Verdict};
use arc_swap::ArcSwap;
use pipeline::model::Pipeline;
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
pub struct RuntimeState {
    pub tasks: Arc<TaskStore>,
    pub brake: Arc<Brake>,
    active: ArcSwap<ActivePipeline>,
}

impl RuntimeState {
    pub fn new(tasks: Arc<TaskStore>, brake: Arc<Brake>, active: ActivePipeline) -> Self {
        Self { tasks, brake, active: ArcSwap::from_pointee(active) }
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
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// The pipeline's entry stage: the first declared team (spec/Plan 2 D5).
fn entry_stage(p: &Pipeline) -> Result<String, String> {
    p.teams.first().map(|t| t.id.clone()).ok_or_else(|| "pipeline has no teams".to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn inject_topic(
    state: tauri::State<'_, Arc<RuntimeState>>,
    topic: String,
    target_repo: Option<String>,
) -> Result<Task, String> {
    inject_topic_inner(state.as_ref(), topic, target_repo).await
}

/// Reusable inner body for inject_topic — callable from the terminal dispatcher
/// at the composition root (Plan 6) without a Tauri State wrapper.
pub async fn inject_topic_inner(
    state: &RuntimeState,
    topic: String,
    target_repo: Option<String>,
) -> Result<Task, String> {
    // Read the current active pipeline/project ONCE (lock-free snapshot) so a
    // concurrent activate never tears the inject mid-build.
    let active = state.active();
    let stage = entry_stage(&active.pipeline)?;
    // A5: default the stored task's target_repo to the project's when the caller
    // supplies none — task overrides project. Reuse the SINGLE precedence fn the
    // worker PathVars build uses, so the rule lives in one place (vet F2).
    let effective = crate::pool::effective_target_repo(
        target_repo.as_deref(),
        active.project_target_repo.as_deref().map(std::path::Path::new),
    )
    .map(|p| p.to_string_lossy().into_owned());
    let task = Task::injected(
        active.project_id.clone(),
        active.pipeline.id.clone(),
        stage,
        topic,
        effective,
        now_unix(),
    );
    state.tasks.insert(&task).await.map_err(|e| e.to_string())?;
    Ok(task)
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

/// Apply an operator verdict at a gate: route to the gate's downstream (approve)
/// or back to the upstream writer (revise) / escalate (reject). For revise we
/// route to the team whose on_approve pointed at this gate.
pub async fn apply_gate_verdict_inner(
    state: &RuntimeState,
    task_id: &str,
    verdict: Verdict,
) -> Result<Task, String> {
    use agent_bus_core::TaskId;
    let active = state.active();
    let mut task = state.tasks.get(&TaskId(task_id.to_string())).await.map_err(|e| e.to_string())?;
    if task.state != TaskState::Gated {
        return Err(format!("task {task_id} is not gated"));
    }
    let now = now_unix();

    match verdict {
        Verdict::Approve => {
            let routed = route(&active.pipeline, &task.current_stage, Verdict::Approve, task.attempts)
                .map_err(|e| format!("{e:?}"))?;
            task.state = routed.next_state;
            task.current_stage = routed.next_stage;
            task.updated_at = now;
        }
        Verdict::Revise => {
            // Send back to the team whose on_approve targets this gate.
            let upstream = active
                .pipeline
                .teams
                .iter()
                .find(|t| t.outputs.on_approve.as_deref() == Some(task.current_stage.as_str()))
                .map(|t| t.id.clone());
            match upstream {
                Some(team_id) if task.attempts < crate::task::MAX_ATTEMPTS => {
                    let _ = task.bump_attempts();
                    task.current_stage = team_id;
                    task.state = TaskState::Queued;
                    task.updated_at = now;
                }
                _ => {
                    task.current_stage = "needs-human".into();
                    task.state = TaskState::NeedsHuman;
                    task.updated_at = now;
                }
            }
        }
        Verdict::Reject => {
            task.current_stage = "needs-human".into();
            task.state = TaskState::NeedsHuman;
            task.updated_at = now;
        }
    }
    state.tasks.update(&task).await.map_err(|e| e.to_string())?;
    Ok(task)
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

/// OHS contract — consumed by Conversational Control (Plan 6).
pub fn tools() -> Vec<ToolSpec> {
    let ctx = "runtime";
    vec![
        ToolSpec {
            name: "inject_topic".into(),
            description: "Inject a new topic into the pipeline's entry team inbox.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "topic": { "type": "string" }, "target_repo": { "type": ["string","null"] } },
                "required": ["topic"]
            }),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "approve_gate".into(),
            description: "Approve a gated task, routing it to the gate's downstream.".into(),
            input_schema: json!({ "type": "object", "properties": { "task_id": { "type": "string" } }, "required": ["task_id"] }),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "reject_gate".into(),
            description: "Reject a gated task (escalates to needs-human).".into(),
            input_schema: json!({ "type": "object", "properties": { "task_id": { "type": "string" } }, "required": ["task_id"] }),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "revise_gate".into(),
            description: "Send a gated task back to its writer for revision.".into(),
            input_schema: json!({ "type": "object", "properties": { "task_id": { "type": "string" } }, "required": ["task_id"] }),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "brake_on".into(),
            description: "Halt new claims (in-flight workers complete).".into(),
            input_schema: json!({ "type": "object", "properties": { "reason": { "type": ["string","null"] } } }),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "brake_off".into(),
            description: "Release the brake.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "scale_team".into(),
            description: "Report a team's configured worker ceiling (v1).".into(),
            input_schema: json!({ "type": "object", "properties": { "team_id": { "type": "string" } }, "required": ["team_id"] }),
            supplier_context: ctx.into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_all_runtime_slug_and_cover_canonical_actions() {
        let t = tools();
        assert!(t.iter().all(|s| s.supplier_context == "runtime"));
        for name in ["inject_topic", "approve_gate", "reject_gate", "revise_gate", "brake_on", "brake_off", "scale_team"] {
            assert!(t.iter().any(|s| s.name == name), "missing tool {name}");
        }
    }

    async fn state_with_project_target_repo(project_default: Option<&str>) -> RuntimeState {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        // tasks table lives in migration 003 (which needs 001's projects table).
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id,name,root_path,created_at,updated_at) VALUES ('proj','n','/p',0,0)")
            .execute(&pool).await.unwrap();
        let pipeline = Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 1,
            defaults: None,
            teams: vec![pipeline::model::Team {
                id: "t1".into(), name: "T1".into(), prompt: "t1.md".into(),
                scope: Default::default(), runner: None,
                outputs: Default::default(), workers: Default::default(),
                role: Default::default(), store: Default::default(),
            }],
            gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
        };
        RuntimeState::new(
            Arc::new(TaskStore::new(pool)),
            Arc::new(Brake::new()),
            ActivePipeline {
                pipeline: Arc::new(pipeline),
                project_id: "proj".into(),
                project_root: "/p".into(),
                project_target_repo: project_default.map(String::from),
            },
        )
    }

    #[tokio::test]
    async fn activate_swaps_active_pipeline_and_inject_reads_new() {
        let state = state_with_project_target_repo(Some("/proj-repo")).await;
        // build a second pipeline with a DIFFERENT entry team + project id
        let p2 = Pipeline {
            id: "p2".into(), name: "P2".into(), description: String::new(), schema_version: 1,
            defaults: None,
            teams: vec![pipeline::model::Team {
                id: "entry2".into(), name: "E2".into(), prompt: "e2.md".into(),
                scope: Default::default(), runner: None,
                outputs: Default::default(), workers: Default::default(),
                role: Default::default(), store: Default::default(),
            }],
            gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
        };
        state.activate_into(ActivePipeline {
            pipeline: Arc::new(p2),
            project_id: "proj".into(),
            project_root: "/p2".into(),
            project_target_repo: Some("/p2-repo".into()),
        });
        let task = inject_topic_inner(&state, "topic".into(), None).await.unwrap();
        // entry stage now the NEW pipeline's first team + the NEW target_repo default
        assert_eq!(task.current_stage, "entry2");
        assert_eq!(task.pipeline, "p2");
        assert_eq!(task.target_repo, Some("/p2-repo".to_string()));
    }

    #[tokio::test]
    async fn inject_defaults_target_repo_to_project_when_unset() {
        let state = state_with_project_target_repo(Some("/proj-repo")).await;
        let task = inject_topic_inner(&state, "topic".into(), None).await.unwrap();
        assert_eq!(task.target_repo, Some("/proj-repo".to_string()));
    }

    #[tokio::test]
    async fn inject_task_target_repo_overrides_project_default() {
        let state = state_with_project_target_repo(Some("/proj-repo")).await;
        let task = inject_topic_inner(&state, "t".into(), Some("/task-repo".into())).await.unwrap();
        assert_eq!(task.target_repo, Some("/task-repo".to_string()));
    }

    #[tokio::test]
    async fn inject_no_project_default_leaves_target_repo_none() {
        let state = state_with_project_target_repo(None).await;
        let task = inject_topic_inner(&state, "t".into(), None).await.unwrap();
        assert_eq!(task.target_repo, None);
    }
}
