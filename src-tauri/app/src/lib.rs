use std::sync::Arc;
use tauri::{Emitter, Manager};
use tauri_plugin_sql::{Migration, MigrationKind};
use workspace::{api::WorkspaceState, store::ProjectStore};

use runners::claude_cli::ClaudeCliRunner;
use runtime::api::RuntimeState;
use runtime::brake::Brake;
use runtime::pool::{process_one_claim, PoolContext};
use runtime::task_store::TaskStore;
use pipeline::model::{Pipeline, Team};

const DB_URL: &str = "sqlite:agent_bus.db";

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Resolve the active project + pipeline at startup. v1: the newest project
/// (projects[0] in the frontend ordering) and its first pipeline file. Returns
/// an empty placeholder pipeline when none exists so the app still boots.
async fn load_active(
    project_store: &ProjectStore,
) -> (String, String, Pipeline) {
    let empty = Pipeline {
        id: String::new(), name: String::new(), description: String::new(),
        schema_version: pipeline::model::SCHEMA_VERSION,
        teams: vec![], gates: vec![], escalations: vec![],
    };
    let Ok(projects) = project_store.list().await else { return (String::new(), String::new(), empty); };
    let Some(project) = projects.into_iter().next() else { return (String::new(), String::new(), empty); };
    let root = project.root_path.to_string_lossy().into_owned();
    let store = pipeline::store::PipelineStore::new(&root);
    let pipe = store
        .list_ids()
        .ok()
        .and_then(|ids| ids.into_iter().next())
        .and_then(|id| store.load(&id).ok())
        .unwrap_or(empty);
    (project.id.0, root, pipe)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let migrations = vec![
        Migration {
            version: 1,
            description: "initial — projects + conversations",
            sql: include_str!("../migrations/001_initial.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "pipeline activation index",
            sql: include_str!("../migrations/002_pipeline_activation.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 3,
            description: "runtime — tasks/workers/comments",
            sql: include_str!("../migrations/003_runtime.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 4,
            description: "review — comments.kind column",
            sql: include_str!("../migrations/004_comments_kind.sql"),
            kind: MigrationKind::Up,
        },
    ];

    tauri::Builder::default()
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(DB_URL, migrations)
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let data_dir = handle.path().app_data_dir().expect("no app data dir");
                std::fs::create_dir_all(&data_dir).ok();
                let db_path = data_dir.join("agent_bus.db");

                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .connect(&format!("sqlite://{}", db_path.display()))
                    .await
                    .expect("could not open store pool");

                // Workspace state (Plan 1).
                let project_store = Arc::new(ProjectStore::new(pool.clone()));
                handle.manage(WorkspaceState { store: project_store.clone() });

                // Runtime state (Plan 3).
                let (project_id, project_root, pipe) = load_active(&project_store).await;
                let pipe = Arc::new(pipe);
                let tasks = Arc::new(TaskStore::new(pool.clone()));
                let brake = Arc::new(Brake::new());

                // F4 crash recovery: release any tasks stuck in `running`.
                let _ = tasks.release_orphaned_running(now_unix()).await;

                handle.manage(RuntimeState {
                    tasks: tasks.clone(),
                    brake: brake.clone(),
                    pipeline: pipe.clone(),
                    project_id: project_id.clone(),
                    project_root: project_root.clone(),
                });

                // Review state (Plan 4).
                let review_state = review::api::ReviewState {
                    comments: std::sync::Arc::new(review::store::CommentStore::new(pool.clone())),
                };
                handle.manage(review_state);

                // Spawn one continuous worker loop per team. Each loop calls
                // process_one_claim and emits task.changed on a settle.
                if !pipe.teams.is_empty() {
                    spawn_worker_loops(handle.clone(), pipe.clone(), tasks.clone(), brake.clone(), project_root);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace::api::workspace_create_project,
            workspace::api::workspace_list_projects,
            workspace::api::workspace_get_project,
            workspace::api::workspace_set_active_pipeline,
            workspace::api::read_artifact,
            pipeline::api::pipeline_list_templates,
            pipeline::api::pipeline_list,
            pipeline::api::pipeline_load,
            pipeline::api::pipeline_instantiate_template,
            runtime::api::inject_topic,
            runtime::api::approve_gate,
            runtime::api::reject_gate,
            runtime::api::revise_gate,
            runtime::api::list_tasks,
            runtime::api::brake_on,
            runtime::api::brake_off,
            runtime::api::brake_state,
            runtime::api::scale_team,
            review::api::add_comment,
            review::api::list_comments,
            review::api::delete_comment,
            review::api::record_verdict,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Spawn a polling worker loop per team. v1 runs one loop per team (concurrent
/// workers per team is v1.1). Each iteration runs process_one_claim; on a
/// settle it emits a `task.changed` event the frontend listens for.
fn spawn_worker_loops(
    handle: tauri::AppHandle,
    pipeline: Arc<Pipeline>,
    tasks: Arc<TaskStore>,
    brake: Arc<Brake>,
    project_root: String,
) {
    let runner: Arc<dyn runners::output::Runner> = Arc::new(ClaudeCliRunner::new());
    for team in pipeline.teams.clone() {
        let ctx = PoolContext {
            pipeline: pipeline.clone(),
            runner: runner.clone(),
            tasks: tasks.clone(),
            brake: brake.clone(),
            project_root: std::path::PathBuf::from(&project_root),
            read_prompt: Arc::new({
                let root = project_root.clone();
                move |t: &Team| {
                    std::fs::read_to_string(std::path::Path::new(&root).join(&t.prompt))
                        .unwrap_or_default()
                }
            }),
        };
        let handle = handle.clone();
        let team = team.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match process_one_claim(&ctx, &team).await {
                    Ok(runtime::pool::ClaimOutcome::Settled { task_id, .. }) => {
                        let _ = handle.emit("task.changed", task_id);
                    }
                    Ok(runtime::pool::ClaimOutcome::RateLimited { .. }) => {
                        ctx.brake.set_on("rate-limit");
                        let _ = handle.emit("task.changed", "rate-limited");
                    }
                    _ => {}
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }
}
