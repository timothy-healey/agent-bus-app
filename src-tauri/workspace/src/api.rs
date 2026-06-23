//! Tauri commands published by the Workspace context — the context's
//! Open Host Service surface.

use crate::project::Project;
use crate::store::{ProjectStore, ProjectStoreError};
use agent_bus_core::{ProjectId, ToolSpec};
use serde_json::json;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Shared state held by Tauri's state manager.
pub struct WorkspaceState {
    pub store: Arc<ProjectStore>,
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_create_project(
    state: tauri::State<'_, WorkspaceState>,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    let project = Project::new(name, PathBuf::from(root_path), now_unix());
    state.store.insert(&project).await.map_err(|e| e.to_string())?;
    Ok(project)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_list_projects(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<Project>, String> {
    state.store.list().await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_get_project(
    state: tauri::State<'_, WorkspaceState>,
    id: String,
) -> Result<Project, String> {
    state.store.get(&ProjectId(id)).await.map_err(|e| match e {
        ProjectStoreError::NotFound(_) => "not_found".to_string(),
        other => other.to_string(),
    })
}

#[tauri::command(rename_all = "snake_case")]
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

/// Resolve `rel_path` against `root`, guaranteeing the result stays inside
/// `root`. Rejects absolute paths and any `..` that would escape the root.
/// Pure (no IO) so it is unit-testable; the command does the read.
pub fn resolve_under_root(root: &str, rel_path: &str) -> Result<PathBuf, String> {
    let rel = Path::new(rel_path);
    if rel.is_absolute() {
        return Err("artifact path must be relative to the project root".into());
    }
    let mut out = PathBuf::from(root);
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("artifact path may not escape the project root".into());
            }
        }
    }
    Ok(out)
}

/// Read an artifact file (spec/plan/critique markdown) by a project-root-relative
/// path. The path is constrained to the project root — Review's reading surface,
/// published as a Workspace OHS command.
#[tauri::command(rename_all = "snake_case")]
pub async fn read_artifact(
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    path: String,
) -> Result<String, String> {
    let project = state
        .store
        .get(&ProjectId(project_id))
        .await
        .map_err(|e| e.to_string())?;
    let root = project.root_path.to_string_lossy();
    let full = resolve_under_root(&root, &path)?;
    std::fs::read_to_string(&full).map_err(|e| e.to_string())
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
        ToolSpec {
            name: "read_artifact".into(),
            description: "Read an artifact file (markdown) by a path relative to the project root.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["project_id", "path"]
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

    #[test]
    fn resolve_under_root_accepts_a_relative_artifact_path() {
        let root = std::env::temp_dir();
        let got = resolve_under_root(root.to_str().unwrap(), "artifacts/specs/T-1-v1.md").unwrap();
        assert!(got.starts_with(&root));
        assert!(got.ends_with("artifacts/specs/T-1-v1.md"));
    }

    #[test]
    fn resolve_under_root_rejects_parent_traversal() {
        let root = std::env::temp_dir();
        assert!(resolve_under_root(root.to_str().unwrap(), "../../etc/passwd").is_err());
    }

    #[test]
    fn resolve_under_root_rejects_absolute_path() {
        let root = std::env::temp_dir();
        assert!(resolve_under_root(root.to_str().unwrap(), "/etc/passwd").is_err());
    }

    #[test]
    fn tools_publishes_read_artifact_under_workspace() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "read_artifact" && s.supplier_context == "workspace"));
    }
}
