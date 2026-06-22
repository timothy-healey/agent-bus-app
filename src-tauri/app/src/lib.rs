use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_sql::{Migration, MigrationKind};
use workspace::{api::WorkspaceState, store::ProjectStore};

const DB_URL: &str = "sqlite:agent_bus.db";

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
                // Resolve the DB path the plugin uses and open our own pool
                // against it. We can't share the plugin's pool directly, so
                // we open a sibling connection — sqlite handles concurrency.
                let data_dir = handle
                    .path()
                    .app_data_dir()
                    .expect("no app data dir");
                std::fs::create_dir_all(&data_dir).ok();
                let db_path = data_dir.join("agent_bus.db");

                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .connect(&format!("sqlite://{}", db_path.display()))
                    .await
                    .expect("could not open project store pool");

                let store = Arc::new(ProjectStore::new(pool));
                handle.manage(WorkspaceState { store });
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace::api::workspace_create_project,
            workspace::api::workspace_list_projects,
            workspace::api::workspace_get_project,
            workspace::api::workspace_set_active_pipeline,
            pipeline::api::pipeline_list_templates,
            pipeline::api::pipeline_list,
            pipeline::api::pipeline_load,
            pipeline::api::pipeline_instantiate_template,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
