//! Scope policy — per-invocation settings.json generation (spec: Worker model
//! → Scope enforcement). Pure construction + a thin filesystem writer. Uses the
//! Workspace path-resolution kernel to turn ${...} patterns into real paths.

use pipeline::model::Scope;
use serde::Serialize;
use std::path::{Path, PathBuf};
use thiserror::Error;
use workspace::paths::{resolve, PathResolveError, PathVars};

/// Tool patterns always denied regardless of team scope (spec: never push,
/// never fetch from inside a worker).
pub const ALWAYS_DENY: &[&str] = &["Bash(git push:*)", "Bash(git fetch:*)"];

#[derive(Debug, Error)]
pub enum ScopeError {
    #[error("path resolution failed: {0}")]
    Resolve(#[from] PathResolveError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// The serialised shape Claude's --settings file expects (subset). Only the
/// permissions block matters for v1 scope enforcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsFile {
    pub permissions: Permissions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Permissions {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// The result of preparing a scope for one invocation: the on-disk settings
/// path + the resolved directories to pass via --add-dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSettings {
    pub settings_path: PathBuf,
    pub add_dirs: Vec<String>,
}

/// Build the SettingsFile (in memory) from a team Scope + resolution vars.
/// `reads`/`writes` patterns are resolved and surfaced as --add-dir entries;
/// the `tools` list becomes permissions.allow; ALWAYS_DENY is appended to deny.
pub fn build_settings(scope: &Scope, vars: &PathVars) -> Result<(SettingsFile, Vec<String>), ScopeError> {
    let mut add_dirs = Vec::new();
    for pat in scope.reads.iter().chain(scope.writes.iter()) {
        add_dirs.push(resolve(pat, vars)?);
    }
    add_dirs.sort();
    add_dirs.dedup();

    let allow = scope.tools.clone();
    let deny = ALWAYS_DENY.iter().map(|s| s.to_string()).collect();

    Ok((SettingsFile { permissions: Permissions { allow, deny } }, add_dirs))
}

/// The directory per-invocation settings files live in: <project>/.agent-bus/runtime/.
pub fn runtime_dir(project_root: &Path) -> PathBuf {
    project_root.join(".agent-bus").join("runtime")
}

/// Prepare a scope for one invocation: build the settings, write it to
/// runtime/<task_id>-<team>-<ts>.settings.json, return the path + add-dirs.
pub fn prepare(
    project_root: &Path,
    team_id: &str,
    task_id: &str,
    ts: i64,
    scope: &Scope,
    vars: &PathVars,
) -> Result<ScopeSettings, ScopeError> {
    let (settings, add_dirs) = build_settings(scope, vars)?;
    let dir = runtime_dir(project_root);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{task_id}-{team_id}-{ts}.settings.json"));
    std::fs::write(&file, serde_json::to_string_pretty(&settings)?)?;
    Ok(ScopeSettings { settings_path: file, add_dirs })
}

/// Delete a settings file (spec: "Deletes the runtime file when the worker
/// settles."). Missing file is not an error.
pub fn cleanup(settings_path: &Path) {
    let _ = std::fs::remove_file(settings_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_scope() -> Scope {
        Scope {
            reads: vec!["${target_repo}".into(), "${project}/artifacts/analyses".into()],
            writes: vec!["${project}/artifacts/analyses".into()],
            tools: vec!["Read".into(), "Write".into(), "Bash(git commit:*)".into()],
        }
    }

    #[test]
    fn build_settings_allows_team_tools_and_always_denies_push_fetch() {
        let vars = PathVars::new("/proj").with_target_repo("/repo");
        let (settings, _dirs) = build_settings(&demo_scope(), &vars).unwrap();
        assert!(settings.permissions.allow.contains(&"Read".to_string()));
        assert!(settings.permissions.allow.contains(&"Bash(git commit:*)".to_string()));
        assert!(settings.permissions.deny.contains(&"Bash(git push:*)".to_string()));
        assert!(settings.permissions.deny.contains(&"Bash(git fetch:*)".to_string()));
    }

    #[test]
    fn build_settings_resolves_add_dirs_and_dedups() {
        let vars = PathVars::new("/proj").with_target_repo("/repo");
        let (_settings, dirs) = build_settings(&demo_scope(), &vars).unwrap();
        // /repo (from reads) + /proj/artifacts/analyses (reads AND writes -> deduped)
        assert_eq!(dirs, vec!["/proj/artifacts/analyses".to_string(), "/repo".to_string()]);
    }

    #[test]
    fn prepare_writes_a_settings_file_then_cleanup_removes_it() {
        let root = std::env::temp_dir().join(format!("abp-scope-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let vars = PathVars::new(&root).with_target_repo("/repo").with_task_id("T-1");

        let ss = prepare(&root, "research", "T-1", 1700, &demo_scope(), &vars).unwrap();
        assert!(ss.settings_path.exists());
        assert!(ss.settings_path.to_string_lossy().contains("T-1-research-1700.settings.json"));

        let body = std::fs::read_to_string(&ss.settings_path).unwrap();
        assert!(body.contains("\"deny\""));
        assert!(body.contains("git push"));

        cleanup(&ss.settings_path);
        assert!(!ss.settings_path.exists());
        cleanup(&ss.settings_path); // idempotent — no panic on missing
    }

    #[test]
    fn runtime_dir_is_under_dot_agent_bus() {
        assert_eq!(
            runtime_dir(Path::new("/p")),
            PathBuf::from("/p/.agent-bus/runtime")
        );
    }
}
