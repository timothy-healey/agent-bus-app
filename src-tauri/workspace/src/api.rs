//! Tauri commands published by the Workspace context — the context's
//! Open Host Service surface.

use crate::paths::project_subdirs;
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

/// Expand a leading `~` or `~/…` in a user-supplied path to the home dir.
/// `home` is injected for testability; pure. Only the current-user `~` is
/// handled (not `~otheruser`); non-tilde paths are returned unchanged. An empty
/// home leaves the input untouched (no silent rewrite to a wrong root).
pub fn expand_tilde(input: &str, home: &str) -> String {
    if home.is_empty() {
        return input.to_string();
    }
    if input == "~" {
        home.to_string()
    } else if let Some(rest) = input.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else {
        input.to_string()
    }
}

fn home_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_create_project(
    state: tauri::State<'_, WorkspaceState>,
    name: String,
    root_path: String,
    target_repo: Option<String>,
) -> Result<Project, String> {
    // Expand a leading ~ so a root like "~/DDD-effort" resolves to the home dir
    // instead of being stored as a literal "~" path (which scattered files under
    // the app's cwd).
    let home = home_dir();
    let root = expand_tilde(&root_path, &home);
    let mut project = Project::new(name, PathBuf::from(root), now_unix());
    // A5: the target repo is tilde-expanded with the same discipline as root.
    project.target_repo = target_repo
        .filter(|s| !s.trim().is_empty())
        .map(|s| expand_tilde(&s, &home));
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

/// OHS command (A5): set (or clear) a project's target repo. Tilde-expanded
/// (same discipline as root_path). Binds `${target_repo}` for all teams' scope
/// resolution and defaults the inject target. Empty/blank input clears it.
#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_set_target_repo(
    state: tauri::State<'_, WorkspaceState>,
    id: String,
    target_repo: Option<String>,
) -> Result<(), String> {
    let home = home_dir();
    let expanded = target_repo
        .filter(|s| !s.trim().is_empty())
        .map(|s| expand_tilde(&s, &home));
    state
        .store
        .set_target_repo(&ProjectId(id), expanded.as_deref(), now_unix())
        .await
        .map_err(|e| e.to_string())
}

/// OHS command (A4): set a project's extra skill sources (`.claude` roots beyond
/// the always-on global ~/.claude). Each path is tilde-expanded with the same
/// discipline as root_path; blank entries are dropped. An empty list clears the
/// config (= global only). The skills crate never learns the Project type — these
/// roots are resolved at the composition root before the scanner sees them.
#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_set_skill_sources(
    state: tauri::State<'_, WorkspaceState>,
    id: String,
    sources: Vec<String>,
) -> Result<(), String> {
    let home = home_dir();
    let expanded: Vec<String> = sources
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| expand_tilde(&s, &home))
        .collect();
    state
        .store
        .set_skill_sources(&ProjectId(id), &expanded, now_unix())
        .await
        .map_err(|e| e.to_string())
}

/// OHS command: remove a project. Deletes the project's entire DB subtree (the
/// FK-safe cascade in `ProjectStore::remove`, including the project's worker
/// rows) AND its on-disk scaffolding (prompts/ pipelines/ artifacts/ .agent-bus/
/// worktrees/, plus tearing down any git worktrees via the WorktreeGit seam).
/// The bound `target_repo` is NEVER touched — and if it is the project root or
/// nested under it, file cleanup is skipped entirely (guard in
/// `cleanup_project_files_inner`).
///
/// Order matters: read the project's paths BEFORE the DB delete (the row is gone
/// after), then run the DB cascade, then the files. File cleanup is best-effort:
/// the DB row is already gone, so a file error is surfaced in the log but does
/// NOT fail the command (that would leave the operator unable to retry a delete
/// of an already-deleted project).
#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_remove_project(
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
    worktrees: tauri::State<'_, crate::worktree::WorktreeState>,
    id: String,
) -> Result<(), String> {
    let project_id = ProjectId(id);

    // 1. Read paths BEFORE deleting (the row is gone afterwards).
    let project = state.store.get(&project_id).await.map_err(|e| match e {
        ProjectStoreError::NotFound(_) => "not_found".to_string(),
        other => other.to_string(),
    })?;
    let root_path = project.root_path.to_string_lossy().into_owned();
    let target_repo = project.target_repo.clone();

    // 2. DB cascade (authoritative; the operator's delete succeeds or fails here).
    state.store.remove(&project_id).await.map_err(|e| match e {
        ProjectStoreError::NotFound(_) => "not_found".to_string(),
        other => other.to_string(),
    })?;

    // 3. On-disk cleanup — best-effort. The DB row is already gone, so a file
    // error must not fail the command (it would block re-deleting). Surface via
    // the log; the guarded cleanup never touches target_repo.
    let cleanup = crate::worktree::cleanup_project_files_inner(
        worktrees.git.as_ref(),
        &root_path,
        target_repo.as_deref(),
    );
    if cleanup.skipped_for_safety || !cleanup.note.is_empty() {
        eprintln!(
            "workspace_remove_project: file cleanup for {root_path}: removed={:?} worktrees={:?} skipped_for_safety={} note={}",
            cleanup.removed, cleanup.worktrees_removed, cleanup.skipped_for_safety, cleanup.note
        );
    }

    // 4. App-owned artifact data dir cleanup (LF26): remove
    // <app_data>/projects/<id> (artifacts live there). Best-effort; never
    // touches target_repo.
    use tauri::Manager;
    if let Ok(app_data) = app.path().app_data_dir() {
        remove_app_data_project_dir(&app_data, &project_id.0);
    }

    Ok(())
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

/// The app-owned data dir for a project: `<app_data>/projects/<project_id>`
/// (parent of the artifacts dir). Removed on project delete. PURE.
pub fn app_data_project_dir(app_data: &Path, project_id: &str) -> PathBuf {
    app_data.join("projects").join(project_id)
}

/// Best-effort removal of the app-owned project data dir (artifacts live here).
/// Never touches `target_repo`. A missing dir is fine.
pub fn remove_app_data_project_dir(app_data: &Path, project_id: &str) {
    let dir = app_data_project_dir(app_data, project_id);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("workspace_remove_project: app-data cleanup for {project_id} failed: {e}");
        }
    }
}

/// The absolute, app-owned artifact base for `project_id` under the Tauri
/// app-data dir: `<app_data>/projects/<project_id>/artifacts` (LF26). The single
/// place this layout is spelled so the engine, the reader, and the delete cascade
/// agree. PURE.
pub fn artifact_base_for(app_data: &Path, project_id: &str) -> PathBuf {
    app_data.join("projects").join(project_id).join("artifacts")
}

/// Resolve `path` against the artifact `base`, allowing an ABSOLUTE path only
/// when it is inside `base`, or a relative path resolved under `base`. Rejects
/// anything that escapes the base (the new escape guard for the app-owned base,
/// replacing `resolve_under_root`'s reject-all-absolutes for artifacts). PURE.
pub fn resolve_under_base(base: &Path, path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    let candidate = if p.is_absolute() {
        // `starts_with` is lexical and does NOT collapse `..`, so an absolute
        // path like `<base>/../../../../etc/passwd` would otherwise pass the
        // guard below. Reject any `..` component first (mirrors the relative
        // branch's escape rule).
        if p.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err("artifact path may not escape the artifact base".into());
        }
        p.to_path_buf()
    } else {
        let mut out = base.to_path_buf();
        for comp in p.components() {
            match comp {
                Component::Normal(c) => out.push(c),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err("artifact path may not escape the artifact base".into());
                }
            }
        }
        out
    };
    if !candidate.starts_with(base) {
        return Err("artifact path is outside the project artifact base".into());
    }
    Ok(candidate)
}

/// Read an artifact file (spec/plan/critique markdown) by path. Artifacts live at
/// the absolute, app-owned base `<app_data>/projects/<id>/artifacts` (LF26); the
/// path (absolute under the base, or relative to it) is escape-guarded to that
/// base. Review's reading surface, published as a Workspace OHS command.
#[tauri::command(rename_all = "snake_case")]
pub async fn read_artifact(
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    path: String,
) -> Result<String, String> {
    use tauri::Manager;
    // Confirm the project exists (keeps the not-found behaviour).
    let _ = state
        .store
        .get(&ProjectId(project_id.clone()))
        .await
        .map_err(|e| e.to_string())?;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let base = artifact_base_for(&app_data, &project_id);
    let full = resolve_under_base(&base, &path)?;
    std::fs::read_to_string(&full).map_err(|e| e.to_string())
}

/// Inner write logic (testable without a Tauri State wrapper). Resolves the
/// project root from the stored Project (already ~-expanded at create — D6; do
/// NOT re-expand), creates the canonical sub-dirs, and writes the pipeline YAML +
/// each prompt file. Every relative path is escape-guarded with
/// resolve_under_root (the same guard read_artifact uses).
pub async fn write_project_pipeline_inner(
    state: &WorkspaceState,
    project_id: String,
    yaml_rel_path: String,
    pipeline_yaml: String,
    prompts: Vec<(String, String)>,
) -> Result<(), String> {
    let project = state
        .store
        .get(&ProjectId(project_id))
        .await
        .map_err(|e| e.to_string())?;
    let root = project.root_path.to_string_lossy().into_owned();

    // Create the canonical project sub-dirs (Workspace owns the layout).
    for sub in project_subdirs() {
        std::fs::create_dir_all(Path::new(&root).join(sub)).map_err(|e| e.to_string())?;
    }

    // Write the pipeline YAML (path-scoped).
    let yaml_path = resolve_under_root(&root, &yaml_rel_path)?;
    if let Some(parent) = yaml_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&yaml_path, pipeline_yaml).map_err(|e| e.to_string())?;

    // Write each prompt file (path-scoped).
    for (rel, body) in prompts {
        let path = resolve_under_root(&root, &rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, body).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// OHS command: write a project's pipeline YAML + per-team prompt files
/// (vet F1 — Workspace owns the bytes-to-disk; Pipeline Authoring serializes).
#[tauri::command(rename_all = "snake_case")]
pub async fn write_project_pipeline(
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    yaml_rel_path: String,
    pipeline_yaml: String,
    prompts: Vec<(String, String)>,
) -> Result<(), String> {
    write_project_pipeline_inner(&state, project_id, yaml_rel_path, pipeline_yaml, prompts).await
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
                    "root_path": { "type": "string" },
                    "target_repo": { "type": ["string", "null"] }
                },
                "required": ["name", "root_path"]
            }),
            supplier_context: "workspace".into(),
        },
        ToolSpec {
            name: "workspace_set_target_repo".into(),
            description: "Set (or clear) a project's target repo (binds ${target_repo}).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "target_repo": { "type": ["string", "null"] }
                },
                "required": ["id"]
            }),
            supplier_context: "workspace".into(),
        },
        ToolSpec {
            name: "workspace_set_skill_sources".into(),
            description: "Set a project's extra skill sources (.claude roots beyond global).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "sources": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["id", "sources"]
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
            name: "workspace_remove_project".into(),
            description: "Remove a project from the workspace registry (does not delete files).".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
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
        ToolSpec {
            name: "write_project_pipeline".into(),
            description: "Write a project's pipeline YAML + per-team prompt files under the project root.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "yaml_rel_path": { "type": "string" },
                    "pipeline_yaml": { "type": "string" },
                    "prompts": { "type": "array" }
                },
                "required": ["project_id", "yaml_rel_path", "pipeline_yaml", "prompts"]
            }),
            supplier_context: "workspace".into(),
        },
        ToolSpec {
            name: "list_worktrees".into(),
            description: "List a project's cleanup-candidate git worktrees (under worktrees/).".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "project_id": { "type": "string" } },
                "required": ["project_id"]
            }),
            supplier_context: "workspace".into(),
        },
        ToolSpec {
            name: "remove_worktree".into(),
            description: "Remove a git worktree (path must live under the project's worktrees/).".into(),
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
        ToolSpec {
            name: "list_dir".into(),
            description: "List a directory's immediate children (dirs-first) for the in-app file/folder picker.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            supplier_context: "workspace".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;
    use crate::store::ProjectStore;
    use std::sync::Arc as StdArc;

    #[test]
    fn artifact_base_for_project_is_app_data_projects_id_artifacts() {
        let base = super::artifact_base_for(std::path::Path::new("/data"), "proj-7");
        assert_eq!(base, std::path::PathBuf::from("/data/projects/proj-7/artifacts"));
    }

    #[test]
    fn resolve_under_base_accepts_absolute_inside_and_rejects_outside() {
        let base = std::path::PathBuf::from("/data/projects/p/artifacts");
        let ok = super::resolve_under_base(&base, "/data/projects/p/artifacts/spec/k-v1.md").unwrap();
        assert_eq!(ok, std::path::PathBuf::from("/data/projects/p/artifacts/spec/k-v1.md"));
        assert!(super::resolve_under_base(&base, "/etc/passwd").is_err());
        // a relative path is still accepted, resolved under the base
        let rel = super::resolve_under_base(&base, "spec/k-v1.md").unwrap();
        assert_eq!(rel, std::path::PathBuf::from("/data/projects/p/artifacts/spec/k-v1.md"));
    }

    #[test]
    fn resolve_under_base_rejects_absolute_with_parent_dir_escape() {
        // An absolute path that lexically starts under the base but climbs out
        // via `..` must be rejected — `starts_with` does NOT collapse `..`.
        let base = std::path::PathBuf::from("/data/projects/p/artifacts");
        assert!(super::resolve_under_base(
            &base,
            "/data/projects/p/artifacts/../../../../etc/passwd"
        )
        .is_err());
    }

    #[test]
    fn app_data_project_dir_is_under_projects_id() {
        let d = super::app_data_project_dir(std::path::Path::new("/data"), "p9");
        assert_eq!(d, std::path::PathBuf::from("/data/projects/p9"));
    }

    #[test]
    fn remove_app_data_project_dir_is_best_effort_and_removes_the_tree() {
        let tmp = std::env::temp_dir().join(format!("abtest-{}", uuid::Uuid::new_v4()));
        let proj = super::app_data_project_dir(&tmp, "p1");
        std::fs::create_dir_all(proj.join("artifacts/spec")).unwrap();
        std::fs::write(proj.join("artifacts/spec/k-v1.md"), b"x").unwrap();
        super::remove_app_data_project_dir(&tmp, "p1");
        assert!(!proj.exists());
        // calling again on a missing dir does not panic
        super::remove_app_data_project_dir(&tmp, "p1");
    }

    async fn state_with_project(root: &std::path::Path) -> (WorkspaceState, String) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/010_project_target_repo.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/011_skill_sources.sql")).execute(&pool).await.unwrap();
        let store = StdArc::new(ProjectStore::new(pool));
        let project = Project::new("Demo".into(), root.to_path_buf(), 0);
        store.insert(&project).await.unwrap();
        (WorkspaceState { store }, project.id.0)
    }

    #[tokio::test]
    async fn write_project_pipeline_writes_yaml_and_prompts_under_root() {
        let root = std::env::temp_dir().join(format!("abp-wpp-{}", uuid::Uuid::new_v4()));
        let (state, project_id) = state_with_project(&root).await;
        write_project_pipeline_inner(
            &state,
            project_id,
            "pipelines/demo.yaml".into(),
            "id: demo\nname: Demo\n".into(),
            vec![("prompts/research.md".into(), "investigate".into())],
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(root.join("pipelines/demo.yaml")).unwrap(), "id: demo\nname: Demo\n");
        assert_eq!(std::fs::read_to_string(root.join("prompts/research.md")).unwrap(), "investigate");
        // the canonical subdirs were created
        assert!(root.join("artifacts").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn write_project_pipeline_rejects_a_path_escape() {
        let root = std::env::temp_dir().join(format!("abp-wpp-{}", uuid::Uuid::new_v4()));
        let (state, project_id) = state_with_project(&root).await;
        let err = write_project_pipeline_inner(
            &state,
            project_id,
            "../escape.yaml".into(),
            "x".into(),
            vec![],
        )
        .await
        .unwrap_err();
        assert!(err.contains("escape") || err.contains("relative"));
        let _ = std::fs::remove_dir_all(&root);
    }

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
    fn tools_publishes_set_target_repo_under_workspace() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "workspace_set_target_repo" && s.supplier_context == "workspace"));
    }

    #[tokio::test]
    async fn set_target_repo_expands_and_persists() {
        let root = std::env::temp_dir().join(format!("abp-tr-{}", uuid::Uuid::new_v4()));
        let (state, project_id) = state_with_project(&root).await;
        let expanded = expand_tilde("~/repo", "/Users/tim");
        state.store.set_target_repo(&ProjectId(project_id.clone()), Some(&expanded), 0).await.unwrap();
        let got = state.store.get(&ProjectId(project_id)).await.unwrap();
        assert_eq!(got.target_repo, Some("/Users/tim/repo".to_string()));
    }

    #[test]
    fn tools_publishes_set_skill_sources_under_workspace() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "workspace_set_skill_sources" && s.supplier_context == "workspace"));
    }

    #[tokio::test]
    async fn set_skill_sources_expands_and_persists() {
        let root = std::env::temp_dir().join(format!("abp-ss-{}", uuid::Uuid::new_v4()));
        let (state, project_id) = state_with_project(&root).await;
        let home = "/Users/tim";
        let expanded: Vec<String> = ["~/a/.claude", "  ", "/abs/.claude"]
            .iter()
            .filter(|s| !s.trim().is_empty())
            .map(|s| expand_tilde(s, home))
            .collect();
        state.store.set_skill_sources(&ProjectId(project_id.clone()), &expanded, 0).await.unwrap();
        let got = state.store.get(&ProjectId(project_id)).await.unwrap();
        assert_eq!(got.skill_sources, vec!["/Users/tim/a/.claude".to_string(), "/abs/.claude".into()]);
    }

    #[test]
    fn tools_publishes_read_artifact_under_workspace() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "read_artifact" && s.supplier_context == "workspace"));
    }

    #[test]
    fn tools_publishes_worktree_commands_under_workspace() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "list_worktrees" && s.supplier_context == "workspace"));
        assert!(t.iter().any(|s| s.name == "remove_worktree" && s.supplier_context == "workspace"));
    }

    #[test]
    fn tools_publishes_list_dir_under_workspace() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "list_dir" && s.supplier_context == "workspace"));
    }

    #[test]
    fn expand_tilde_expands_leading_home() {
        assert_eq!(expand_tilde("~/DDD-effort", "/Users/tim"), "/Users/tim/DDD-effort");
        assert_eq!(expand_tilde("~", "/Users/tim"), "/Users/tim");
        // a trailing slash on home doesn't double up
        assert_eq!(expand_tilde("~/a/b", "/Users/tim/"), "/Users/tim/a/b");
    }

    #[test]
    fn expand_tilde_leaves_other_paths_unchanged() {
        assert_eq!(expand_tilde("/abs/path", "/Users/tim"), "/abs/path");
        assert_eq!(expand_tilde("relative/x", "/Users/tim"), "relative/x");
        // only the current-user ~ is handled
        assert_eq!(expand_tilde("~otheruser/x", "/Users/tim"), "~otheruser/x");
        // empty home: leave untouched rather than rewrite to a wrong root
        assert_eq!(expand_tilde("~/x", ""), "~/x");
    }
}
