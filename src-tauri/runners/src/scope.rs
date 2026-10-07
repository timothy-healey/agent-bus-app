//! Scope policy: turns a team's effective scope into the per-invocation
//! settings file (`permissions.allow`, `permissions.deny`, `autoMode`) and the
//! tool list the worker argv removes. Pure construction plus a thin
//! filesystem writer. Paths arrive absolute; Runtime resolves them.

use agent_bus_core::ToolGrant;
use serde::Serialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Always allowed. `StructuredOutput` is how a worker returns its result;
/// blocking it would turn every result into a missing Structured output.
pub const ALWAYS_ALLOW: &[&str] = &["StructuredOutput"];

/// Denied by rule for every worker, grant or not.
pub const HARD_LIMITS: &[&str] = &[
    "Bash(git push --force:*)",
    "Bash(git push -f:*)",
    "Bash(git push --force-with-lease:*)",
    "Bash(git push * main)",
    "Bash(git push * master)",
    "Bash(git push *:main)",
    "Bash(git push *:master)",
    "Bash(gh pr merge:*)",
    "Bash(gh repo create:*)",
    "Bash(gh repo delete:*)",
    "Bash(git remote add:*)",
];

/// Denied by rule unless the team holds the Remote git grant.
pub const REMOTE_GIT_DENY: &[&str] = &["Bash(git push:*)", "Bash(git fetch:*)", "Bash(git pull:*)", "Bash(gh:*)"];

/// Tools removed from every worker: none is grantable, and each reaches beyond
/// the files and commands a scope governs (scheduling, remote triggers,
/// notifications, other worktrees, notebooks).
pub const ALWAYS_REMOVED: &[&str] = &[
    "NotebookEdit",
    "CronCreate",
    "CronDelete",
    "CronList",
    "ScheduleWakeup",
    "RemoteTrigger",
    "PushNotification",
    "Workflow",
    "EnterWorktree",
    "ExitWorktree",
    "DesignSync",
    "ShareOnboardingGuide",
    "ReportFindings",
];

/// The tools each grant keeps. A tool listed here is removed unless a grant
/// the team holds keeps it.
const GRANTED_TOOLS: &[(&str, &[&str])] = &[
    ("bash", &["Bash", "Monitor"]),
    ("agent", &["Agent", "ListAgents", "SendMessage"]),
    ("web-fetch", &["WebFetch"]),
    ("web-search", &["WebSearch"]),
];

#[derive(Debug, Error)]
pub enum ScopeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// A team's scope for one invocation, with every path already absolute.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerScope {
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub grants: Vec<ToolGrant>,
    /// `public`, `private` or `internal`; `None` when the lookup failed.
    pub repo_visibility: Option<String>,
    /// Paths denied for editing without being added (the main repo of a task
    /// that runs in a worktree).
    pub deny_paths: Vec<String>,
}

/// The `--settings` file (the subset the worker needs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsFile {
    pub permissions: Permissions,
    #[serde(rename = "autoMode")]
    pub auto_mode: AutoMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Permissions {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// Context for the auto-mode classifier. `$defaults` keeps the built-in
/// environment entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoMode {
    pub environment: Vec<String>,
    /// Prose limits the classifier never lets intent override. Covers what a
    /// deny rule cannot: edits outside the write paths, and shell commands
    /// outside a team's Bash patterns.
    pub hard_deny: Vec<String>,
}

/// What preparing a scope gives the invocation: the settings file, the
/// directories to add and the tools to remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSettings {
    pub settings_path: PathBuf,
    pub add_dirs: Vec<String>,
    pub disallowed_tools: Vec<String>,
}

/// Lists a directory's entries as `(path, is_dir)`. Injected so the deny
/// computation is tested without a filesystem.
pub type ListDir<'a> = &'a dyn Fn(&Path) -> Vec<(PathBuf, bool)>;

/// The real directory lister. A missing or unreadable directory has no entries.
pub fn list_dir_fs(dir: &Path) -> Vec<(PathBuf, bool)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<(PathBuf, bool)> = rd
        .flatten()
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (e.path(), is_dir)
        })
        .collect();
    out.sort();
    out
}

/// Every tool the worker argv removes: the always-removed set plus each
/// grantable tool no held grant keeps. Read, Glob, Grep, Skill, Edit, Write
/// and `StructuredOutput` are never listed.
pub fn disallowed_tools(grants: &[ToolGrant]) -> Vec<String> {
    let mut keep: Vec<&str> = Vec::new();
    for g in grants {
        match g {
            ToolGrant::Bash => keep.extend(["Bash", "Monitor"]),
            ToolGrant::BashPattern(_) => keep.push("Bash"),
            ToolGrant::Agent => keep.extend(["Agent", "ListAgents", "SendMessage"]),
            ToolGrant::WebFetch => keep.push("WebFetch"),
            ToolGrant::WebSearch => keep.push("WebSearch"),
            ToolGrant::RemoteGit => {}
        }
    }
    let mut out: Vec<String> = Vec::new();
    for (_, tools) in GRANTED_TOOLS {
        for t in *tools {
            if !keep.contains(t) && !out.iter().any(|o| o == t) {
                out.push(t.to_string());
            }
        }
    }
    out.extend(ALWAYS_REMOVED.iter().map(|t| t.to_string()));
    out
}

/// The directories to pass as `--add-dir`: every read and write path, sorted
/// and deduped.
pub fn add_dirs(scope: &WorkerScope) -> Vec<String> {
    let mut dirs: Vec<String> = scope.reads.iter().chain(scope.writes.iter()).cloned().collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// `Edit(//abs/**)`-style rule for a path (`//` marks an absolute path).
fn path_rule(tool: &str, path: &Path, recursive: bool) -> String {
    let p = path.to_string_lossy();
    let p = p.trim_end_matches('/');
    if recursive {
        format!("{tool}(/{p}/**)")
    } else {
        format!("{tool}(/{p})")
    }
}

/// The paths under `read` that must be denied for editing. A read inside a
/// write is not denied (the more specific write wins). A read containing a
/// write cannot be denied wholesale, because a deny rule beats any allow, so
/// it is descended: every existing entry on the way that does not lead to a
/// write path is denied. Entries created later are not covered.
fn read_denies(read: &Path, writes: &[PathBuf], list_dir: ListDir) -> Vec<(PathBuf, bool)> {
    if writes.iter().any(|w| read.starts_with(w)) {
        return vec![];
    }
    if !writes.iter().any(|w| w.starts_with(read)) {
        return vec![(read.to_path_buf(), true)];
    }
    let mut out = Vec::new();
    for (entry, is_dir) in list_dir(read) {
        if writes.iter().any(|w| entry.starts_with(w)) {
            continue;
        }
        if writes.iter().any(|w| w.starts_with(&entry)) {
            if is_dir {
                out.extend(read_denies(&entry, writes, list_dir));
            }
            continue;
        }
        out.push((entry, is_dir));
    }
    out
}

/// Build the settings file for a scope.
/// - allow: `Bash` for the Bash grant, `Bash(<pattern>)` per pattern, the web
///   tools and `Agent` when granted, and `StructuredOutput`;
/// - deny: the hard limits, the remote-git commands without the grant, and
///   `Edit`/`Write` on every read path not covered by a write path;
/// - `autoMode.environment`: `$defaults` plus the repo visibility when known.
pub fn build_settings(scope: &WorkerScope, list_dir: ListDir) -> SettingsFile {
    let mut allow: Vec<String> = Vec::new();
    for g in &scope.grants {
        let rule = match g {
            ToolGrant::Bash => "Bash".to_string(),
            ToolGrant::BashPattern(p) => format!("Bash({p})"),
            ToolGrant::Agent => "Agent".to_string(),
            ToolGrant::WebFetch => "WebFetch".to_string(),
            ToolGrant::WebSearch => "WebSearch".to_string(),
            ToolGrant::RemoteGit => continue,
        };
        if !allow.contains(&rule) {
            allow.push(rule);
        }
    }
    for t in ALWAYS_ALLOW {
        if !allow.iter().any(|a| a == t) {
            allow.push(t.to_string());
        }
    }

    let mut deny: Vec<String> = HARD_LIMITS.iter().map(|s| s.to_string()).collect();
    if !scope.grants.contains(&ToolGrant::RemoteGit) {
        deny.extend(REMOTE_GIT_DENY.iter().map(|s| s.to_string()));
    }
    let writes: Vec<PathBuf> = scope.writes.iter().map(PathBuf::from).collect();
    let mut reads: Vec<PathBuf> = scope.reads.iter().chain(scope.deny_paths.iter()).map(PathBuf::from).collect();
    reads.sort();
    reads.dedup();
    let mut denied: Vec<(PathBuf, bool)> = Vec::new();
    for r in &reads {
        for d in read_denies(r, &writes, list_dir) {
            if !denied.iter().any(|(p, _)| d.0.starts_with(p)) {
                denied.push(d);
            }
        }
    }
    for (path, recursive) in &denied {
        deny.push(path_rule("Edit", path, *recursive));
        deny.push(path_rule("Write", path, *recursive));
    }

    let mut environment = vec!["$defaults".to_string()];
    if let Some(v) = &scope.repo_visibility {
        environment.push(format!("Repository visibility: {v}"));
    }

    // In auto mode an edit outside the working directories goes to the
    // classifier rather than being refused, so the write paths are stated as
    // a hard limit.
    let mut hard_deny = vec!["$defaults".to_string()];
    if !scope.writes.is_empty() {
        hard_deny.push(format!(
            "Writing outside the work area: never create, edit, move or delete files outside these directories, by any tool or shell command: {}. Git's own bookkeeping for the work (commits, branches, pushes the task allows) is not a file edit.",
            scope.writes.join(", ")
        ));
    }
    let patterns: Vec<&str> = scope
        .grants
        .iter()
        .filter_map(|g| match g {
            ToolGrant::BashPattern(p) => Some(p.as_str()),
            _ => None,
        })
        .collect();
    if !patterns.is_empty() && !scope.grants.contains(&ToolGrant::Bash) {
        hard_deny.push(format!(
            "Unlisted shell commands: never run a shell command unless it matches one of these patterns: {}",
            patterns.join(", ")
        ));
    }

    SettingsFile { permissions: Permissions { allow, deny }, auto_mode: AutoMode { environment, hard_deny } }
}

/// The directory per-invocation settings files live in: <project>/.agent-bus/runtime/.
pub fn runtime_dir(project_root: &Path) -> PathBuf {
    project_root.join(".agent-bus").join("runtime")
}

/// Prepare a scope for one invocation: build the settings, write it to
/// runtime/<task_id>-<team>-<ts>.settings.json, and return its path with the
/// add-dirs and the tools to remove.
pub fn prepare(
    project_root: &Path,
    team_id: &str,
    task_id: &str,
    ts: i64,
    scope: &WorkerScope,
) -> Result<ScopeSettings, ScopeError> {
    let settings = build_settings(scope, &list_dir_fs);
    let dir = runtime_dir(project_root);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{task_id}-{team_id}-{ts}.settings.json"));
    std::fs::write(&file, serde_json::to_string_pretty(&settings)?)?;
    Ok(ScopeSettings { settings_path: file, add_dirs: add_dirs(scope), disallowed_tools: disallowed_tools(&scope.grants) })
}

/// Delete a settings file once the worker settles. A missing file is not an error.
pub fn cleanup(settings_path: &Path) {
    let _ = std::fs::remove_file(settings_path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::ToolGrant::*;

    fn no_fs(_: &Path) -> Vec<(PathBuf, bool)> {
        vec![]
    }

    fn scope(reads: &[&str], writes: &[&str], grants: Vec<ToolGrant>) -> WorkerScope {
        WorkerScope {
            reads: reads.iter().map(|s| s.to_string()).collect(),
            writes: writes.iter().map(|s| s.to_string()).collect(),
            grants,
            ..WorkerScope::default()
        }
    }

    #[test]
    fn a_deny_path_is_denied_for_editing_but_not_added() {
        let mut s = scope(&[], &["/wt/alpha"], vec![]);
        s.deny_paths = vec!["/repo".into()];
        let d = deny_of(&s, &no_fs);
        assert!(d.contains(&"Edit(//repo/**)".to_string()), "{d:?}");
        assert!(d.contains(&"Write(//repo/**)".to_string()), "{d:?}");
        assert_eq!(add_dirs(&s), vec!["/wt/alpha".to_string()]);
    }

    #[test]
    fn auto_mode_hard_denies_edits_outside_the_write_paths() {
        let s = scope(&["/repo"], &["/art/me", "/wt"], vec![]);
        let hd = build_settings(&s, &no_fs).auto_mode.hard_deny;
        assert_eq!(hd[0], "$defaults");
        assert!(hd.iter().any(|r| r.contains("/art/me") && r.contains("/wt") && r.contains("outside")), "{hd:?}");
    }

    #[test]
    fn bash_patterns_alone_hard_deny_every_other_shell_command() {
        let s = scope(&[], &["/w"], vec![BashPattern("git diff:*".into()), BashPattern("git log:*".into())]);
        let hd = build_settings(&s, &no_fs).auto_mode.hard_deny;
        assert!(hd.iter().any(|r| r.contains("git diff:*") && r.contains("git log:*") && r.contains("shell")), "{hd:?}");
        let full = scope(&[], &["/w"], vec![Bash, BashPattern("git diff:*".into())]);
        let hd = build_settings(&full, &no_fs).auto_mode.hard_deny;
        assert!(!hd.iter().any(|r| r.contains("git diff:*")), "{hd:?}");
    }

    fn deny_of(s: &WorkerScope, list: ListDir) -> Vec<String> {
        build_settings(s, list).permissions.deny
    }

    #[test]
    fn a_read_path_is_denied_for_edit_and_write() {
        let d = deny_of(&scope(&["/repo"], &["/art/me"], vec![]), &no_fs);
        assert!(d.contains(&"Edit(//repo/**)".to_string()), "{d:?}");
        assert!(d.contains(&"Write(//repo/**)".to_string()), "{d:?}");
    }

    #[test]
    fn a_write_path_is_not_denied() {
        let d = deny_of(&scope(&[], &["/art/me"], vec![]), &no_fs);
        assert!(!d.iter().any(|r| r.contains("/art/me")), "{d:?}");
    }

    #[test]
    fn a_read_inside_a_write_is_not_denied() {
        let d = deny_of(&scope(&["/repo/docs"], &["/repo"], vec![]), &no_fs);
        assert!(!d.iter().any(|r| r.starts_with("Edit(") || r.starts_with("Write(")), "{d:?}");
    }

    #[test]
    fn a_read_containing_a_write_denies_only_what_does_not_lead_to_it() {
        let list = |p: &Path| -> Vec<(PathBuf, bool)> {
            match p.to_str().unwrap() {
                "/art" => vec![("/art/f.md".into(), false), ("/art/me".into(), true), ("/art/other".into(), true)],
                _ => vec![],
            }
        };
        let d = deny_of(&scope(&["/art"], &["/art/me"], vec![]), &list);
        assert!(d.contains(&"Edit(//art/other/**)".to_string()), "{d:?}");
        assert!(d.contains(&"Write(//art/other/**)".to_string()), "{d:?}");
        assert!(d.contains(&"Edit(//art/f.md)".to_string()), "{d:?}");
        assert!(!d.iter().any(|r| r.contains("/art/me")), "{d:?}");
        assert!(!d.contains(&"Edit(//art/**)".to_string()), "{d:?}");
    }

    #[test]
    fn a_nested_write_is_reached_through_its_ancestors() {
        let list = |p: &Path| -> Vec<(PathBuf, bool)> {
            match p.to_str().unwrap() {
                "/r" => vec![("/r/a".into(), true), ("/r/b".into(), true)],
                "/r/a" => vec![("/r/a/w".into(), true), ("/r/a/x".into(), true)],
                _ => vec![],
            }
        };
        let d = deny_of(&scope(&["/r"], &["/r/a/w"], vec![]), &list);
        assert!(d.contains(&"Edit(//r/b/**)".to_string()), "{d:?}");
        assert!(d.contains(&"Edit(//r/a/x/**)".to_string()), "{d:?}");
        assert!(!d.iter().any(|r| r.contains("/r/a/w") || r == "Edit(//r/a/**)"), "{d:?}");
    }

    #[test]
    fn the_hard_limits_are_always_denied() {
        for grants in [vec![], vec![Bash, RemoteGit]] {
            let d = deny_of(&scope(&[], &[], grants), &no_fs);
            for h in HARD_LIMITS {
                assert!(d.contains(&h.to_string()), "{h} missing from {d:?}");
            }
        }
    }

    #[test]
    fn remote_git_is_denied_only_without_the_grant() {
        let without = deny_of(&scope(&[], &[], vec![Bash]), &no_fs);
        for r in REMOTE_GIT_DENY {
            assert!(without.contains(&r.to_string()), "{r}");
        }
        let with = deny_of(&scope(&[], &[], vec![Bash, RemoteGit]), &no_fs);
        for r in REMOTE_GIT_DENY {
            assert!(!with.contains(&r.to_string()), "{r}");
        }
    }

    #[test]
    fn grants_become_allow_rules() {
        let s = scope(&[], &[], vec![BashPattern("git diff:*".into()), WebFetch, RemoteGit]);
        let allow = build_settings(&s, &no_fs).permissions.allow;
        assert_eq!(allow, vec!["Bash(git diff:*)".to_string(), "WebFetch".into(), "StructuredOutput".into()]);
        let bash = build_settings(&scope(&[], &[], vec![Bash, Agent]), &no_fs).permissions.allow;
        assert_eq!(bash, vec!["Bash".to_string(), "Agent".into(), "StructuredOutput".into()]);
    }

    #[test]
    fn auto_mode_environment_keeps_the_defaults_and_adds_visibility_when_known() {
        let mut s = scope(&[], &[], vec![]);
        assert_eq!(build_settings(&s, &no_fs).auto_mode.environment, vec!["$defaults".to_string()]);
        s.repo_visibility = Some("public".into());
        assert_eq!(
            build_settings(&s, &no_fs).auto_mode.environment,
            vec!["$defaults".to_string(), "Repository visibility: public".into()]
        );
    }

    #[test]
    fn settings_serialise_with_the_cli_key_names() {
        let v = serde_json::to_value(build_settings(&scope(&[], &[], vec![]), &no_fs)).unwrap();
        assert!(v["permissions"]["allow"].is_array());
        assert!(v["permissions"]["deny"].is_array());
        assert_eq!(v["autoMode"]["environment"][0], "$defaults");
    }

    #[test]
    fn no_grants_removes_every_grantable_tool() {
        let d = disallowed_tools(&[]);
        for t in ["Bash", "Monitor", "Agent", "ListAgents", "SendMessage", "WebFetch", "WebSearch"] {
            assert!(d.contains(&t.to_string()), "{t}");
        }
        for t in ALWAYS_REMOVED {
            assert!(d.contains(&t.to_string()), "{t}");
        }
    }

    #[test]
    fn the_always_on_tools_are_never_removed() {
        for grants in [vec![], vec![Bash, Agent, WebFetch, WebSearch, RemoteGit]] {
            let d = disallowed_tools(&grants);
            for t in ["Read", "Glob", "Grep", "Skill", "Edit", "Write", "StructuredOutput", "ToolSearch"] {
                assert!(!d.contains(&t.to_string()), "{t} removed with {grants:?}");
            }
        }
    }

    #[test]
    fn each_grant_keeps_its_tools() {
        let all = disallowed_tools(&[Bash, Agent, WebFetch, WebSearch]);
        assert_eq!(all, ALWAYS_REMOVED.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let pattern = disallowed_tools(&[BashPattern("git diff:*".into())]);
        assert!(!pattern.contains(&"Bash".to_string()));
        assert!(pattern.contains(&"Monitor".to_string()));
        let remote = disallowed_tools(&[RemoteGit]);
        assert!(remote.contains(&"Bash".to_string()));
    }

    #[test]
    fn add_dirs_are_reads_and_writes_deduped() {
        let s = scope(&["/repo", "/art"], &["/art", "/w"], vec![]);
        assert_eq!(add_dirs(&s), vec!["/art".to_string(), "/repo".into(), "/w".into()]);
    }

    #[test]
    fn prepare_writes_a_settings_file_then_cleanup_removes_it() {
        let root = std::env::temp_dir().join(format!("abp-scope-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let s = scope(&["/repo"], &["/art"], vec![Bash]);

        let ss = prepare(&root, "research", "T-1", 1700, &s).unwrap();
        assert!(ss.settings_path.exists());
        assert!(ss.settings_path.to_string_lossy().contains("T-1-research-1700.settings.json"));
        assert_eq!(ss.add_dirs, vec!["/art".to_string(), "/repo".into()]);
        assert_eq!(ss.disallowed_tools, disallowed_tools(&[Bash]));

        let body: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&ss.settings_path).unwrap()).unwrap();
        assert!(body["permissions"]["deny"].as_array().unwrap().iter().any(|r| r == "Edit(//repo/**)"));
        assert_eq!(body["autoMode"]["environment"][0], "$defaults");

        cleanup(&ss.settings_path);
        assert!(!ss.settings_path.exists());
        cleanup(&ss.settings_path);
    }

    #[test]
    fn runtime_dir_is_under_dot_agent_bus() {
        assert_eq!(runtime_dir(Path::new("/p")), PathBuf::from("/p/.agent-bus/runtime"));
    }
}
