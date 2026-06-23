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

use conversational_control::catalog::ToolCatalog;
use conversational_control::dispatch::ToolDispatcher;
use conversational_control::engine::CommandEngine;
use conversational_control::store::ConversationStore;
use conversational_control::api::TerminalState;
use agent_bus_core::{ToolCallRequest, ToolCallResult};
use async_trait::async_trait;

const DB_URL: &str = "sqlite:agent_bus.db";

/// Apply schema migrations to the app's own pool, in order, idempotently.
///
/// The applied version lives in SQLite's `PRAGMA user_version` rather than in
/// tauri-plugin-sql's tracking (whose migrations only run when the *frontend*
/// loads the plugin — which this app never does). `raw_sql` runs the
/// multi-statement migration files; migration 004 is a non-idempotent `ALTER`,
/// so the version gate is what keeps re-runs safe.
async fn run_migrations(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    const MIGRATIONS: &[(i64, &str)] = &[
        (1, include_str!("../migrations/001_initial.sql")),
        (2, include_str!("../migrations/002_pipeline_activation.sql")),
        (3, include_str!("../migrations/003_runtime.sql")),
        (4, include_str!("../migrations/004_comments_kind.sql")),
        (5, include_str!("../migrations/005_usage.sql")),
    ];

    let current: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await?;

    for (version, sql) in MIGRATIONS {
        if *version > current {
            sqlx::raw_sql(sql).execute(pool).await?;
            // PRAGMA can't bind parameters; the value is our own trusted i64.
            sqlx::raw_sql(&format!("PRAGMA user_version = {version};"))
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// The concrete dispatcher. Lives at the root — the only module that imports
/// every context (D1/D2). Routes a ToolCallRequest to the owning supplier's
/// logic, reusing the same stores the Tauri commands use.
struct RootDispatcher {
    runtime: Arc<RuntimeState>,
    usage: Arc<usage_telemetry::api::UsageState>,
    app: tauri::AppHandle,
}

/// Concrete revise-bundle reader (Plan 4 vet F1 consumer). Reads the comments
/// table written by Review and flattens rows into Runtime's RevisionNote. Lives
/// at the root because it bridges Runtime's trait + Review's schema without
/// either crate depending on the other.
pub struct SqliteRevisionReader {
    pub pool: sqlx::SqlitePool,
}

#[async_trait]
impl runtime::revision::RevisionBundleReader for SqliteRevisionReader {
    async fn load(&self, task_id: &str) -> runtime::revision::RevisionBundle {
        let rows: Vec<(Option<String>, String, String)> = sqlx::query_as(
            "SELECT anchor_text, note, kind FROM comments WHERE task_id = ? \
             ORDER BY created_at ASC, anchor_offset ASC",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        runtime::revision::RevisionBundle {
            notes: rows
                .into_iter()
                .map(|(anchor_text, note, kind)| runtime::revision::RevisionNote {
                    anchor_text,
                    note,
                    kind,
                })
                .collect(),
        }
    }
}

fn ok(v: serde_json::Value) -> ToolCallResult { ToolCallResult::Ok { result: v } }
fn err(e: impl ToString) -> ToolCallResult { ToolCallResult::Err { error: e.to_string() } }

#[async_trait]
impl ToolDispatcher for RootDispatcher {
    async fn dispatch(&self, req: &ToolCallRequest) -> ToolCallResult {
        use tauri::Emitter;
        let a = &req.args;
        let str_arg = |k: &str| a.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        let result = match req.tool_name.as_str() {
            "inject_topic" => {
                match runtime::api::inject_topic_inner(
                    &self.runtime, str_arg("topic").unwrap_or_default(), str_arg("target_repo"),
                ).await {
                    Ok(task) => { let _ = self.app.emit("task.changed", &task.id.0); serde_json::to_value(task).map(ok).unwrap_or_else(err) }
                    Err(e) => err(e),
                }
            }
            "approve_gate" | "reject_gate" | "revise_gate" => {
                let task_id = str_arg("task_id").unwrap_or_default();
                let verdict = match req.tool_name.as_str() {
                    "approve_gate" => agent_bus_core::Verdict::Approve,
                    "reject_gate" => agent_bus_core::Verdict::Reject,
                    _ => agent_bus_core::Verdict::Revise,
                };
                match runtime::api::apply_gate_verdict_inner(&self.runtime, &task_id, verdict).await {
                    Ok(task) => { let _ = self.app.emit("task.changed", &task.id.0); serde_json::to_value(task).map(ok).unwrap_or_else(err) }
                    Err(e) => err(e),
                }
            }
            "brake_on" => { let s = self.runtime.brake.clone(); s.set_on(str_arg("reason").unwrap_or_else(|| "manual".into())); let _ = self.app.emit("usage.changed", ()); ok(serde_json::to_value(s.state()).unwrap()) }
            "brake_off" => { self.runtime.brake.set_off(); let _ = self.app.emit("usage.changed", ()); ok(serde_json::to_value(self.runtime.brake.state()).unwrap()) }
            "scale_team" => {
                match runtime::api::scale_team_inner(&self.runtime, str_arg("team_id").unwrap_or_default()) {
                    Ok(maxn) => ok(serde_json::json!({ "max": maxn })),
                    Err(e) => err(e),
                }
            }
            "usage_snapshot" => {
                let cfg = usage_telemetry::api::load_config(&self.usage.pool).await;
                let braked = (self.usage.is_braked)();
                match usage_telemetry::snapshot::compute_snapshot(&self.usage.cc, &self.usage.worker, &cfg, braked, now_unix()).await {
                    Ok(snap) => serde_json::to_value(snap).map(ok).unwrap_or_else(err),
                    Err(e) => err(e),
                }
            }
            other => err(format!("tool not dispatchable in v1: {other}")),
        };
        result
    }
}

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
        Migration {
            version: 5,
            description: "usage telemetry — worker_usage_log + cc_usage_log + usage_config",
            sql: include_str!("../migrations/005_usage.sql"),
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
                    .connect_with(
                        sqlx::sqlite::SqliteConnectOptions::new()
                            .filename(&db_path)
                            .create_if_missing(true),
                    )
                    .await
                    .expect("could not open store pool");

                // This pool — not tauri-plugin-sql — owns the schema. The
                // plugin only migrates when the frontend calls Database.load(),
                // which this app never does (all data goes through Rust commands
                // over this pool). So apply migrations here, idempotently.
                run_migrations(&pool)
                    .await
                    .expect("could not run migrations");

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

                // Runtime state — keep an Arc so the terminal dispatcher can reuse the same logic.
                let runtime_state_arc = Arc::new(RuntimeState {
                    tasks: tasks.clone(), brake: brake.clone(), pipeline: pipe.clone(),
                    project_id: project_id.clone(), project_root: project_root.clone(),
                });
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

                // Usage Telemetry (Plan 5). The WorkerUsageStore is the concrete
                // UsageSink (kernel seam); the brake-state callback lets Telemetry
                // read Runtime's brake without depending on the runtime crate.
                use usage_telemetry::cc_log::CcUsageStore;
                use usage_telemetry::worker_log::WorkerUsageStore;
                let cc_store = Arc::new(CcUsageStore::new(pool.clone()));
                let worker_usage = Arc::new(WorkerUsageStore::new(pool.clone()));
                let usage_sink: Arc<dyn agent_bus_core::UsageSink> = worker_usage.clone();
                let usage_state_arc = Arc::new(usage_telemetry::api::UsageState {
                    cc: cc_store.clone(), worker: worker_usage.clone(), pool: pool.clone(),
                    is_braked: Arc::new({ let b = brake.clone(); move || b.is_on() }),
                });
                handle.manage(usage_telemetry::api::UsageState {
                    cc: cc_store.clone(),
                    worker: worker_usage.clone(),
                    pool: pool.clone(),
                    is_braked: Arc::new({ let b = brake.clone(); move || b.is_on() }),
                });

                // Conversational Control (Plan 6). Build the tool catalog by UNIONing every supplier's tools().
                let mut specs = Vec::new();
                specs.extend(pipeline::api::tools());
                specs.extend(runtime::api::tools());
                specs.extend(review::api::tools());
                specs.extend(usage_telemetry::api::tools());
                specs.extend(runners::api::tools());
                specs.extend(workspace::api::tools());
                specs.extend(conversational_control::api::tools());
                let catalog = Arc::new(ToolCatalog::new(specs));
                debug_assert!(catalog.duplicate_names().is_empty(), "tool name collision in catalog");

                let dispatcher: Arc<dyn ToolDispatcher> = Arc::new(RootDispatcher {
                    runtime: runtime_state_arc.clone(),
                    usage: usage_state_arc.clone(),
                    app: handle.clone(),
                });

                let engine: Arc<dyn conversational_control::engine::ConversationEngine> =
                    Arc::new(CommandEngine::new(dispatcher.clone()));

                let convo_store = Arc::new(ConversationStore::new(pool.clone()));

                // 24h-summarisation on launch (D5).
                if !project_id.is_empty() {
                    if let Ok(Some(mut convo)) = convo_store.load(&project_id).await {
                        if conversational_control::summarise::summarise_on_launch(
                            &mut convo, uuid::Uuid::new_v4().to_string(), now_unix(),
                        ) {
                            let _ = convo_store.save(&convo).await;
                        }
                    }
                }

                handle.manage(TerminalState {
                    catalog: catalog.clone(),
                    engine,
                    store: convo_store,
                    project_id: project_id.clone(),
                });

                // Spawn one continuous worker loop per team. Each loop calls
                // process_one_claim and emits task.changed on a settle.
                if !pipe.teams.is_empty() {
                    let revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>> =
                        Some(Arc::new(SqliteRevisionReader { pool: pool.clone() }));
                    spawn_worker_loops(handle.clone(), pipe.clone(), tasks.clone(), brake.clone(), project_root, Some(usage_sink.clone()), revision_reader);
                }

                // Auto-meter sweep (D8/D9). v1 config has auto_meter_enabled=0 so
                // decide() returns NoChange and nothing happens; when v1.1 flips
                // the flag this trips/releases Runtime's brake by reason.
                {
                    let cc = cc_store.clone();
                    let worker = worker_usage.clone();
                    let brake = brake.clone();
                    let pool = pool.clone();
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        use usage_telemetry::api::load_config;
                        use usage_telemetry::brake_policy::{BrakeDecision, AUTO_METER_REASON};
                        use usage_telemetry::snapshot::{auto_brake_decision, compute_snapshot};
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            let cfg = load_config(&pool).await;
                            if !cfg.auto_meter_enabled { continue; }
                            let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
                            let now = now_unix();
                            if let Ok(snap) = compute_snapshot(&cc, &worker, &cfg, brake.is_on(), now).await {
                                match auto_brake_decision(&snap, &cfg, auto_on) {
                                    BrakeDecision::SetOn(reason) => { brake.set_on(reason); let _ = handle.emit("usage.changed", ()); }
                                    BrakeDecision::Release => { brake.set_off(); let _ = handle.emit("usage.changed", ()); }
                                    BrakeDecision::NoChange => {}
                                }
                            }
                        }
                    });
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
            usage_telemetry::api::usage_snapshot,
            usage_telemetry::api::usage_set_budget,
            conversational_control::api::send_message,
            conversational_control::api::get_conversation,
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
    usage_sink: Option<Arc<dyn agent_bus_core::UsageSink>>,
    revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>>,
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
            usage_sink: usage_sink.clone(),
            revision_reader: revision_reader.clone(),
        };
        let handle = handle.clone();
        let team = team.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match process_one_claim(&ctx, &team).await {
                    Ok(runtime::pool::ClaimOutcome::Settled { task_id, .. }) => {
                        let _ = handle.emit("task.changed", task_id);
                        // A settle recorded worker usage; tell the meter to refresh.
                        let _ = handle.emit("usage.changed", ());
                    }
                    Ok(runtime::pool::ClaimOutcome::RateLimited { .. }) => {
                        // Reactive brake (spec) — already the behaviour; reason
                        // surfaces on the meter via brake_state.
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

#[cfg(test)]
mod revision_reader_tests {
    use super::SqliteRevisionReader;
    use runtime::revision::RevisionBundleReader;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn pool_with_comment(task: &str, kind: &str, anchor: Option<&str>, note: &str) -> sqlx::SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        for sql in [
            include_str!("../migrations/001_initial.sql"),
            include_str!("../migrations/003_runtime.sql"),
            include_str!("../migrations/004_comments_kind.sql"),
        ] {
            for stmt in sql.split(';') {
                let s = stmt.trim();
                if !s.is_empty() { sqlx::query(s).execute(&pool).await.unwrap(); }
            }
        }
        sqlx::query("INSERT INTO comments (id, task_id, artifact_path, anchor_text, anchor_offset, note, kind, created_at) VALUES (?,?,?,?,?,?,?,?)")
            .bind("c1").bind(task).bind("artifacts/specs/T-1-v1.md")
            .bind(anchor).bind::<Option<i64>>(None).bind(note).bind(kind).bind(1000i64)
            .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn reads_inline_comment_into_bundle() {
        let pool = pool_with_comment("T-1", "inline", Some("batch key"), "per-row").await;
        let reader = SqliteRevisionReader { pool };
        let bundle = reader.load("T-1").await;
        assert_eq!(bundle.notes.len(), 1);
        assert_eq!(bundle.notes[0].kind, "inline");
        assert_eq!(bundle.notes[0].anchor_text.as_deref(), Some("batch key"));
        assert_eq!(bundle.notes[0].note, "per-row");
    }

    #[tokio::test]
    async fn missing_task_yields_empty_bundle() {
        let pool = pool_with_comment("T-1", "inline", None, "x").await;
        let reader = SqliteRevisionReader { pool };
        assert!(reader.load("T-NOPE").await.notes.is_empty());
    }
}

#[cfg(test)]
mod migration_tests {
    use super::run_migrations;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    // Regression: the GUI boot path opens a *file* pool and the app — not
    // tauri-plugin-sql — must create + migrate the schema. The in-memory store
    // tests never exercised this, so a fresh launch panicked ("unable to open
    // database file") and would then have hit "no such table".
    #[tokio::test]
    async fn fresh_file_is_created_migrated_and_idempotent() {
        let dir = std::env::temp_dir().join("agent_bus_app_migration_test");
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("boot_test.db");
        let _ = std::fs::remove_file(&db); // ensure a truly fresh file

        // create_if_missing(true) is the fix for the code-14 panic.
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db)
                    .create_if_missing(true),
            )
            .await
            .expect("pool opens and creates a missing file");

        run_migrations(&pool).await.expect("first migration run");
        // Second run must be a no-op — migration 004's ALTER would error if
        // re-applied, so this proves the user_version gate works.
        run_migrations(&pool)
            .await
            .expect("second run is idempotent");

        let projects: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='projects'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(projects, 1, "projects table created (migration 001)");

        let kind_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('comments') WHERE name='kind'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(kind_cols, 1, "migration 004 column present exactly once");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 5, "all five migrations recorded");

        let _ = std::fs::remove_file(&db);
    }
}
