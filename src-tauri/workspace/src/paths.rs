//! Path-resolution kernel — the second deliberate shared kernel (DOMAIN.md).
//!
//! Every context substitutes the same variables when turning a YAML path
//! pattern (e.g. "${target_repo}/docs/critique-*.md") into a concrete path.
//! This module owns the variable vocabulary and the substitution algorithm.
//! It is pure: no I/O, no filesystem access — it only rewrites strings.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// The resolution context: concrete values for the path variables that apply
/// to a single substitution. `target_repo` and `task_id` are optional because
/// some patterns (and some call sites) don't reference them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathVars {
    /// `${project}` and its migration alias `${agent_bus}` — the project root.
    pub project_root: PathBuf,
    /// `${target_repo}` — the repo a task targets (per-task in Runtime).
    pub target_repo: Option<PathBuf>,
    /// `${task_id}` — the in-flight task id.
    pub task_id: Option<String>,
}

impl PathVars {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            target_repo: None,
            task_id: None,
        }
    }

    pub fn with_target_repo(mut self, repo: impl Into<PathBuf>) -> Self {
        self.target_repo = Some(repo.into());
        self
    }

    pub fn with_task_id(mut self, id: impl Into<String>) -> Self {
        self.task_id = Some(id.into());
        self
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathResolveError {
    #[error("unknown path variable: ${{{0}}}")]
    UnknownVariable(String),
    #[error("path references ${{{0}}} but no value was supplied")]
    MissingValue(String),
}

/// Substitute `${...}` tokens in `pattern` against `vars`. Returns the
/// rewritten string. Recognised tokens: `project`, `agent_bus` (alias for
/// project), `target_repo`, `task_id`. Any other token is an error. A
/// recognised-but-unsupplied optional token (target_repo / task_id) is a
/// MissingValue error rather than a silent passthrough.
pub fn resolve(pattern: &str, vars: &PathVars) -> Result<String, PathResolveError> {
    let mut out = String::with_capacity(pattern.len());
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let close = pattern[i + 2..]
                .find('}')
                .map(|rel| i + 2 + rel)
                .ok_or_else(|| PathResolveError::UnknownVariable(pattern[i + 2..].to_string()))?;
            let name = &pattern[i + 2..close];
            let value: String = match name {
                "project" | "agent_bus" => vars.project_root.to_string_lossy().into_owned(),
                "target_repo" => vars
                    .target_repo
                    .as_ref()
                    .ok_or_else(|| PathResolveError::MissingValue("target_repo".into()))?
                    .to_string_lossy()
                    .into_owned(),
                "task_id" => vars
                    .task_id
                    .clone()
                    .ok_or_else(|| PathResolveError::MissingValue("task_id".into()))?,
                other => return Err(PathResolveError::UnknownVariable(other.to_string())),
            };
            out.push_str(&value);
            i = close + 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

/// Convenience: resolve a pattern and return it as a PathBuf.
pub fn resolve_path(pattern: &str, vars: &PathVars) -> Result<PathBuf, PathResolveError> {
    resolve(pattern, vars).map(PathBuf::from)
}

/// The canonical project sub-directories the wizard creates (spec → Data model
/// → Filesystem layout). Exposed here so Workspace owns the layout vocabulary.
pub fn project_subdirs() -> &'static [&'static str] {
    &["pipelines", "prompts", "artifacts", "worktrees", ".agent-bus"]
}

/// The directory pipeline YAML files live in, relative to a project root.
pub fn pipelines_dir(project_root: &Path) -> PathBuf {
    project_root.join("pipelines")
}

/// The directory per-team prompt markdown files live in, relative to a project
/// root. Workspace owns this layout vocabulary (vet F1).
pub fn prompts_dir(project_root: &Path) -> PathBuf {
    project_root.join("prompts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_project_and_alias() {
        let vars = PathVars::new("/home/u/proj");
        assert_eq!(resolve("${project}/artifacts", &vars).unwrap(), "/home/u/proj/artifacts");
        assert_eq!(resolve("${agent_bus}/x", &vars).unwrap(), "/home/u/proj/x");
    }

    #[test]
    fn resolves_target_repo_and_task_id() {
        let vars = PathVars::new("/p")
            .with_target_repo("/repo")
            .with_task_id("T-040");
        assert_eq!(
            resolve("${target_repo}/docs/${task_id}.md", &vars).unwrap(),
            "/repo/docs/T-040.md"
        );
    }

    #[test]
    fn leaves_non_variable_text_untouched() {
        let vars = PathVars::new("/p");
        assert_eq!(resolve("plain/path/no/vars.md", &vars).unwrap(), "plain/path/no/vars.md");
    }

    #[test]
    fn unknown_variable_is_an_error() {
        let vars = PathVars::new("/p");
        assert_eq!(
            resolve("${bogus}/x", &vars),
            Err(PathResolveError::UnknownVariable("bogus".into()))
        );
    }

    #[test]
    fn missing_optional_value_is_an_error() {
        let vars = PathVars::new("/p"); // no target_repo
        assert_eq!(
            resolve("${target_repo}/x", &vars),
            Err(PathResolveError::MissingValue("target_repo".into()))
        );
    }

    #[test]
    fn resolve_path_returns_pathbuf() {
        let vars = PathVars::new("/p");
        assert_eq!(resolve_path("${project}/a", &vars).unwrap(), PathBuf::from("/p/a"));
    }

    #[test]
    fn pipelines_dir_is_project_root_join_pipelines() {
        assert_eq!(pipelines_dir(Path::new("/p")), PathBuf::from("/p/pipelines"));
    }

    #[test]
    fn prompts_dir_is_project_root_join_prompts() {
        assert_eq!(prompts_dir(Path::new("/p")), PathBuf::from("/p/prompts"));
    }
}
