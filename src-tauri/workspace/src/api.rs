//! Tauri commands published by the Workspace context — the context's
//! Open Host Service surface.

use crate::project::Project;
use crate::store::{ProjectStore, ProjectStoreError};
use agent_bus_core::{ProjectId, ToolSpec};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Shared state held by Tauri's state manager.
pub struct WorkspaceState {
    pub store: Arc<ProjectStore>,
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

#[tauri::command]
pub async fn workspace_create_project(
    state: tauri::State<'_, WorkspaceState>,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    let project = Project::new(name, PathBuf::from(root_path), now_unix());
    state.store.insert(&project).await.map_err(|e| e.to_string())?;
    Ok(project)
}

#[tauri::command]
pub async fn workspace_list_projects(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<Project>, String> {
    state.store.list().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_get_project(
    state: tauri::State<'_, WorkspaceState>,
    id: String,
) -> Result<Project, String> {
    state.store.get(&ProjectId(id)).await.map_err(|e| match e {
        ProjectStoreError::NotFound(_) => "not_found".to_string(),
        other => other.to_string(),
    })
}

#[tauri::command]
pub async fn workspace_set_active_pipeline(
    state: tauri::State<'_, WorkspaceState>,
    id: String,
    pipeline_id: Option<String>,
) -> Result<(), String> {
    use agent_bus_core::PipelineId;
    let pid = pipeline_id.map(PipelineId);
    state
        .store
        .set_active_pipeline(&ProjectId(id), pid.as_ref(), now_unix())
        .await
        .map_err(|e| e.to_string())
}

/// OHS contract: the union of these is what Conversational Control will
/// expose to the god terminal in Plan 6.
pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "workspace_create_project".into(),
            description: "Create a new project in the workspace.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "root_path": { "type": "string" }
                },
                "required": ["name", "root_path"]
            }),
            supplier_context: "workspace".into(),
        },
        ToolSpec {
            name: "workspace_list_projects".into(),
            description: "List all known projects, newest first.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            supplier_context: "workspace".into(),
        },
        ToolSpec {
            name: "workspace_set_active_pipeline".into(),
            description: "Set (or clear) a project's active pipeline.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "pipeline_id": { "type": ["string", "null"] }
                },
                "required": ["id"]
            }),
            supplier_context: "workspace".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_publishes_workspace_named_tools() {
        let t = tools();
        assert!(t.iter().all(|s| s.supplier_context == "workspace"));
        assert!(t.iter().any(|s| s.name == "workspace_create_project"));
        assert!(t.iter().any(|s| s.name == "workspace_list_projects"));
    }
}
