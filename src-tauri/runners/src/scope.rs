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

/// Tool patterns that imply the worker needs outbound network access. When none
/// of the team's tools match, the generated sandbox profile denies network.
const NETWORK_TOOLS: &[&str] = &["WebFetch", "WebSearch"];

/// System paths a confined `claude` subprocess must still read to start and run
/// (dynamic linker, shared libs, system frameworks, devices). Deliberately
/// read-only.
const SYSTEM_READ_SUBPATHS: &[&str] = &[
    "/usr/lib",
    "/usr/bin",
    "/usr/share",
    "/System",
    "/Library",
    "/private/var",
    "/dev",
    "/etc",
    "/bin",
    "/sbin",
];

/// Escape a path for inclusion in an SBPL double-quoted string literal.
fn sbpl_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// **EXPERIMENTAL · macOS-only · STRUCTURAL-ONLY.** Generate a macOS
/// `sandbox-exec` (SBPL) profile string from a team Scope.
///
/// This is a *pure* projection of the same Scope that `build_settings` projects
/// to `settings.json` — a deny-by-default profile that allows reads/writes only
/// on the team's resolved scope paths (plus system essentials needed to run
/// `claude`), and grants network only when the team holds a network-capable tool.
///
/// IMPORTANT HONESTY CAVEAT: `sandbox-exec` is Apple-DEPRECATED (still functional
/// on current macOS) and this confinement is **unverified** in CI — only this
/// string generation is unit-tested. Wrapping a real subprocess in
/// `sandbox-exec -p <this>` is opt-in/experimental and is NOT a proven security
/// boundary. The SBPL idiom is sealed inside the Runners ACL.
pub fn sandbox_profile(scope: &Scope, vars: &PathVars) -> Result<String, ScopeError> {
    let mut reads: Vec<String> = Vec::new();
    for pat in scope.reads.iter() {
        reads.push(resolve(pat, vars)?);
    }
    let mut writes: Vec<String> = Vec::new();
    for pat in scope.writes.iter() {
        writes.push(resolve(pat, vars)?);
    }
    // Writable dirs are also readable.
    reads.extend(writes.iter().cloned());
    reads.sort();
    reads.dedup();
    writes.sort();
    writes.dedup();

    let net_allowed = scope
        .tools
        .iter()
        .any(|t| NETWORK_TOOLS.iter().any(|n| t.starts_with(n)));

    let mut p = String::new();
    p.push_str("(version 1)\n");
    p.push_str(";; EXPERIMENTAL macOS sandbox-exec profile — generated from team Scope.\n");
    p.push_str(";; sandbox-exec is Apple-deprecated; this is opt-in and NOT a proven boundary.\n");
    p.push_str("(deny default)\n");
    p.push_str("(allow process-exec)\n");
    p.push_str("(allow process-fork)\n");
    p.push_str("(allow sysctl-read)\n");
    p.push_str("(allow mach-lookup)\n");

    // System essentials: read-only.
    p.push_str("(allow file-read*\n");
    for sp in SYSTEM_READ_SUBPATHS {
        p.push_str(&format!("  (subpath \"{}\")\n", sbpl_escape(sp)));
    }
    // Scope read dirs (includes write dirs, which are also readable).
    for d in &reads {
        p.push_str(&format!("  (subpath \"{}\")\n", sbpl_escape(d)));
    }
    p.push_str(")\n");

    // Scope write dirs + tmp.
    p.push_str("(allow file-write*\n");
    p.push_str("  (subpath \"/private/tmp\")\n");
    p.push_str("  (subpath \"/private/var/folders\")\n");
    for d in &writes {
        p.push_str(&format!("  (subpath \"{}\")\n", sbpl_escape(d)));
    }
    p.push_str(")\n");

    if net_allowed {
        p.push_str("(allow network*)\n");
    } else {
        p.push_str("(deny network*)\n");
    }

    Ok(p)
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
    fn sandbox_profile_denies_by_default_and_allows_scope_paths() {
        let vars = PathVars::new("/proj").with_target_repo("/repo");
        let p = sandbox_profile(&demo_scope(), &vars).unwrap();
        // deny-by-default header
        assert!(p.starts_with("(version 1)"));
        assert!(p.contains("(deny default)"));
        // process exec/fork allowed (claude must run)
        assert!(p.contains("(allow process-exec)"));
        assert!(p.contains("(allow process-fork)"));
        // read access to the resolved read dirs
        assert!(p.contains("(allow file-read*"));
        assert!(p.contains("(subpath \"/repo\")"));
        assert!(p.contains("(subpath \"/proj/artifacts/analyses\")"));
        // write access only to the resolved write dirs
        assert!(p.contains("(allow file-write*"));
        // system essentials are readable
        assert!(p.contains("(subpath \"/usr/lib\")"));
        assert!(p.contains("(subpath \"/System\")"));
    }

    #[test]
    fn sandbox_profile_denies_network_unless_a_network_tool_is_present() {
        let vars = PathVars::new("/proj").with_target_repo("/repo");
        // demo_scope() has Read/Write/Bash(git commit) — no network tool
        let p = sandbox_profile(&demo_scope(), &vars).unwrap();
        assert!(p.contains("(deny network*)"));
        assert!(!p.contains("(allow network*)"));

        let mut net = demo_scope();
        net.tools.push("WebFetch".into());
        let p2 = sandbox_profile(&net, &vars).unwrap();
        assert!(p2.contains("(allow network*)"));
        assert!(!p2.contains("(deny network*)"));
    }

    #[test]
    fn sandbox_profile_escapes_quotes_and_backslashes_in_paths() {
        let scope = Scope {
            reads: vec!["${project}/a\"b".into()],
            writes: vec![],
            tools: vec![],
        };
        let vars = PathVars::new("/proj");
        let p = sandbox_profile(&scope, &vars).unwrap();
        // an embedded quote must be backslash-escaped inside the SBPL string literal
        assert!(p.contains("/proj/a\\\"b"));
    }

    #[test]
    fn runtime_dir_is_under_dot_agent_bus() {
        assert_eq!(
            runtime_dir(Path::new("/p")),
            PathBuf::from("/p/.agent-bus/runtime")
        );
    }
}
