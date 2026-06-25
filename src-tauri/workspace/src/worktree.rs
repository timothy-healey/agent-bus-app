//! Worktree cleanup for the Workspace context. Workspace owns the project
//! filesystem layout (`paths::project_subdirs` includes `worktrees`), so it
//! owns finding + safely removing leftover git worktrees. The real `git`
//! calls are isolated behind the `WorktreeGit` seam (mirrors the `SpawnFn`
//! seam in `runners/src/claude_cli.rs` and the `KeychainStore` trait in the
//! `secrets` crate) so tests never touch a real repo.
//!
//! IMPORTANT (honest scope): this app does NOT yet create per-task git
//! worktrees — workers run under `--add-dir` scopes (`runners/src/scope.rs`),
//! not isolated worktrees. This module is a self-contained cleanup utility for
//! worktrees that exist under `<project_root>/worktrees/` (created by hand or by
//! a future per-task-worktree feature). Every such worktree is reported as a
//! cleanup candidate because none is tied to an active task today.

use crate::store::ProjectStore;
use agent_bus_core::ProjectId;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// One worktree discovered for a project. The DTO crossing the OHS — no git
/// idiom (no porcelain text, no argv) leaks past this type. Owned solely by
/// Workspace and published through Workspace's own OHS (like `Project`), so it
/// is not a cross-context kernel and does not belong in `agent_bus_core`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeEntry {
    /// Absolute path to the worktree directory.
    pub path: String,
    /// The checked-out HEAD (commit sha) if git reported one, else empty.
    pub head: String,
    /// The branch ref if any (e.g. "refs/heads/feature"), else empty.
    pub branch: String,
    /// True when this worktree is a **cleanup candidate**: it lives under the
    /// project's `worktrees/` subtree and is tied to no active task. Today that
    /// is every worktree under `worktrees/` (per-task creation is absent). NOT
    /// "stale" in the sense of the app having aged it — see the module doc.
    pub stale: bool,
}

/// Parse `git worktree list --porcelain` output into entries, keeping ONLY the
/// worktrees whose path is a descendant of `<project_root>/worktrees/` and
/// marking them `stale = true` (cleanup candidate). The main repo worktree and
/// any worktree outside the project's `worktrees/` subtree are dropped (never
/// offered for removal). Pure: no IO.
pub fn parse_porcelain(porcelain: &str, project_root: &Path) -> Vec<WorktreeEntry> {
    let worktrees_root = project_root.join("worktrees");
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut head = String::new();
    let mut branch = String::new();

    let flush =
        |path: &mut Option<String>, head: &mut String, branch: &mut String, out: &mut Vec<WorktreeEntry>| {
            if let Some(p) = path.take() {
                if is_under(Path::new(&p), &worktrees_root) {
                    out.push(WorktreeEntry {
                        path: p,
                        head: std::mem::take(head),
                        branch: std::mem::take(branch),
                        stale: true,
                    });
                } else {
                    head.clear();
                    branch.clear();
                }
            }
        };

    for line in porcelain.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            // A new block begins — flush the previous one.
            flush(&mut path, &mut head, &mut branch, &mut out);
            path = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            head = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = rest.to_string();
        }
        // "detached", "bare", "locked", blank lines: ignored (branch stays "").
    }
    flush(&mut path, &mut head, &mut branch, &mut out);
    out
}

/// True when `candidate` is `base` itself or a descendant of it, after
/// normalising away `.`/`..` components. Pure: no filesystem access.
fn is_under(candidate: &Path, base: &Path) -> bool {
    normalize(candidate).starts_with(normalize(base))
}

/// Lexical normalisation: drop `.`, resolve `..` against accumulated
/// components. No symlink resolution (no IO) — sufficient for path-scoping.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// SAFETY GUARD: resolve `path` and confirm it is a descendant of
/// `<project_root>/worktrees/`. Returns the normalised PathBuf on success, or
/// an Err string on any path that would escape the project's worktrees subtree
/// (a different project, a sibling dir like `artifacts/`, or a `..` traversal).
/// `remove_worktree` MUST call this and run NO git command on Err. Pure: no IO.
pub fn worktree_under_root(project_root: &str, path: &str) -> Result<PathBuf, String> {
    let base = normalize(&Path::new(project_root).join("worktrees"));
    let candidate = normalize(Path::new(path));
    if candidate == base {
        // The worktrees/ dir itself is not a worktree — never removable.
        return Err("path is the worktrees root, not a worktree".into());
    }
    if candidate.starts_with(&base) {
        Ok(candidate)
    } else {
        Err("worktree path must live under <project_root>/worktrees/".into())
    }
}

/// The git seam: the ONLY place `git worktree` idiom executes. Production uses
/// `GitCli` (shells out); tests inject a fake. No git type crosses the OHS.
pub trait WorktreeGit: Send + Sync {
    /// `git -C <repo> worktree list --porcelain` → raw stdout.
    fn list_porcelain(&self, repo: &str) -> Result<String, String>;
    /// `git -C <repo> worktree remove <path>`.
    fn remove(&self, repo: &str, path: &str) -> Result<(), String>;
    /// `git -C <repo> worktree add -b <branch> <path> <base_ref>`.
    fn add(&self, repo: &str, path: &str, branch: &str, base_ref: &str) -> Result<(), String>;
    /// Reset a worktree to its branch baseline:
    /// `git -C <worktree_path> reset --hard` then `git -C <worktree_path> clean -fd`
    /// (no remote, so no `@{upstream}`).
    fn reset(&self, worktree_path: &str) -> Result<(), String>;
}

/// Production git runner: shells out to the system `git`. Constructed at the
/// composition root; never used in tests.
pub struct GitCli;

impl WorktreeGit for GitCli {
    fn list_porcelain(&self, repo: &str) -> Result<String, String> {
        let out = std::process::Command::new("git")
            .args(["-C", repo, "worktree", "list", "--porcelain"])
            .output()
            .map_err(|e| format!("git worktree list failed to spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git worktree list failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn remove(&self, repo: &str, path: &str) -> Result<(), String> {
        let out = std::process::Command::new("git")
            .args(["-C", repo, "worktree", "remove", path])
            .output()
            .map_err(|e| format!("git worktree remove failed to spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git worktree remove failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }

    fn add(&self, repo: &str, path: &str, branch: &str, base_ref: &str) -> Result<(), String> {
        let out = std::process::Command::new("git")
            .args(["-C", repo, "worktree", "add", "-b", branch, path, base_ref])
            .output()
            .map_err(|e| format!("git worktree add failed to spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }

    fn reset(&self, worktree_path: &str) -> Result<(), String> {
        let reset = std::process::Command::new("git")
            .args(["-C", worktree_path, "reset", "--hard"])
            .output()
            .map_err(|e| format!("git reset failed to spawn: {e}"))?;
        if !reset.status.success() {
            return Err(format!(
                "git reset --hard failed: {}",
                String::from_utf8_lossy(&reset.stderr)
            ));
        }
        let clean = std::process::Command::new("git")
            .args(["-C", worktree_path, "clean", "-fd"])
            .output()
            .map_err(|e| format!("git clean failed to spawn: {e}"))?;
        if !clean.status.success() {
            return Err(format!(
                "git clean -fd failed: {}",
                String::from_utf8_lossy(&clean.stderr)
            ));
        }
        Ok(())
    }
}

/// List the project's cleanup-candidate worktrees. Pure orchestration over the
/// seam: ask git, parse + filter to the `worktrees/` subtree. Testable with a
/// fake git.
pub fn list_worktrees_inner(
    git: &dyn WorktreeGit,
    project_root: &str,
) -> Result<Vec<WorktreeEntry>, String> {
    let porcelain = git.list_porcelain(project_root)?;
    Ok(parse_porcelain(&porcelain, Path::new(project_root)))
}

/// Remove one worktree after the path-scoping guard passes. Runs NO git command
/// on a rejected path (the guard returns Err first). Testable with a fake git.
pub fn remove_worktree_inner(
    git: &dyn WorktreeGit,
    project_root: &str,
    path: &str,
) -> Result<(), String> {
    let resolved = worktree_under_root(project_root, path)?;
    git.remove(project_root, &resolved.to_string_lossy())
}

/// Create one worktree after the path-scoping guard passes (same guard as
/// `remove_worktree_inner`). Runs NO git command on a rejected path. The branch
/// is created at `base_ref` (the target repo's current HEAD, passed by the
/// caller). Testable with a fake git.
pub fn add_worktree_inner(
    git: &dyn WorktreeGit,
    project_root: &str,
    path: &str,
    branch: &str,
    base_ref: &str,
) -> Result<(), String> {
    let resolved = worktree_under_root(project_root, path)?;
    git.add(project_root, &resolved.to_string_lossy(), branch, base_ref)
}

/// Reset one worktree to its branch baseline after the path-scoping guard
/// passes. Runs NO git command on a rejected path. Testable with a fake git.
pub fn reset_worktree_inner(
    git: &dyn WorktreeGit,
    project_root: &str,
    path: &str,
) -> Result<(), String> {
    let resolved = worktree_under_root(project_root, path)?;
    git.reset(&resolved.to_string_lossy())
}

/// The scaffolded subdirs the wizard creates under a project root, in the order
/// this cleanup removes them. `worktrees` is removed LAST (after its git
/// worktrees are torn down via the seam). Mirrors `paths::project_subdirs` but
/// fixes the removal order (`worktrees` last) and is the destructive vocabulary,
/// kept local to the cleanup so it can never drift into "wipe the whole root".
const SCAFFOLDED_SUBDIRS: &[&str] = &[
    "prompts",
    "pipelines",
    "artifacts",
    ".agent-bus",
    "worktrees",
];

/// Outcome of [`cleanup_project_files_inner`]: a human-readable note plus the
/// subdirs actually removed. `skipped_for_safety` is true when the target-repo
/// guard tripped and NO file deletion happened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileCleanup {
    /// Scaffolded subdirs that were removed (existed and were deleted).
    pub removed: Vec<String>,
    /// Worktree paths torn down via the git seam.
    pub worktrees_removed: Vec<String>,
    /// True when the safety guard skipped ALL file deletion (target repo == root
    /// or nested under it). `note` explains why.
    pub skipped_for_safety: bool,
    /// Non-fatal notes (guard explanation, per-worktree remove failures, etc.).
    pub note: String,
}

/// SAFETY GUARD: is the bound `target_repo` the project root itself, or nested
/// under it? If so, removing the scaffolded subdirs would risk the real repo, so
/// the caller MUST skip file deletion entirely. Lexical (no IO). `None`/empty
/// target_repo → never unsafe.
fn target_repo_collides_with_root(root_path: &str, target_repo: Option<&str>) -> bool {
    let Some(repo) = target_repo.filter(|s| !s.trim().is_empty()) else {
        return false;
    };
    let root = normalize(Path::new(root_path));
    let repo = normalize(Path::new(repo));
    // Collision when repo IS the root, or repo is a descendant of the root
    // (deleting root's subdirs could nuke the repo). `starts_with` covers both
    // (a path starts_with itself).
    repo.starts_with(&root)
}

/// Remove a project's on-disk scaffolding with hard safety guards. Given the
/// project's `root_path`, its (optional) bound `target_repo`, and the
/// `WorktreeGit` seam, this:
///   1. Lists the project's cleanup-candidate worktrees (under
///      `<root>/worktrees/`) and `git worktree remove`s each via the seam,
///      reusing the same path-scoping guard (`worktree_under_root`). Tolerant of
///      none and of per-worktree failures (recorded in the note, never fatal).
///   2. Removes ONLY the named scaffolded subdirs that resolve strictly under
///      `root_path` (`prompts/ pipelines/ artifacts/ .agent-bus/ worktrees/`).
///      Missing dirs are fine. It NEVER removes `root_path` itself (operator
///      chose "scaffolded pieces", not the whole root).
///
/// HARD SAFETY GUARDS:
///   * NEVER touches `target_repo` or anything under it.
///   * If `target_repo == root_path` or is nested under `root_path`, SKIPs ALL
///     file deletion (returns `skipped_for_safety = true` with a clear note)
///     rather than risk the real repo. Worktree teardown is also skipped in that
///     case (a worktree under such a root could be inside the repo).
///   * Only ever deletes the named subdirs that resolve strictly under
///     `root_path` (a subdir whose normalised path is not under the root is
///     skipped — defence in depth, can't happen for literal names).
pub fn cleanup_project_files_inner(
    git: &dyn WorktreeGit,
    root_path: &str,
    target_repo: Option<&str>,
) -> FileCleanup {
    let mut out = FileCleanup::default();

    // GUARD: refuse entirely if the bound repo is the root or under it.
    if target_repo_collides_with_root(root_path, target_repo) {
        out.skipped_for_safety = true;
        out.note = format!(
            "skipped file cleanup: target_repo ({}) is the project root or nested under it; \
             refusing to risk the real repo",
            target_repo.unwrap_or_default()
        );
        return out;
    }

    let root = normalize(Path::new(root_path));

    // 1. Tear down git worktrees via the seam (path-scoped, tolerant).
    match list_worktrees_inner(git, root_path) {
        Ok(entries) => {
            for entry in entries {
                match remove_worktree_inner(git, root_path, &entry.path) {
                    Ok(()) => out.worktrees_removed.push(entry.path),
                    Err(e) => {
                        out.note
                            .push_str(&format!("worktree remove failed for {}: {e}; ", entry.path));
                    }
                }
            }
        }
        Err(e) => {
            // No worktrees / not a git repo / git missing: tolerant — record and
            // carry on to the directory cleanup.
            out.note.push_str(&format!("worktree list skipped: {e}; "));
        }
    }

    // 2. Remove only the named scaffolded subdirs, each re-checked to resolve
    // strictly under the root (defence in depth; never the root itself).
    for name in SCAFFOLDED_SUBDIRS {
        let dir = normalize(&root.join(name));
        if dir == root || !dir.starts_with(&root) {
            // Would not be strictly under the root — refuse (cannot happen for
            // the literal names, but the guard is cheap and explicit).
            continue;
        }
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => out.removed.push(name.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => { /* tolerant */ }
            Err(e) => {
                out.note
                    .push_str(&format!("failed to remove {}: {e}; ", dir.display()));
            }
        }
    }

    out
}

/// State for the worktree commands: holds the project store (to resolve a
/// project's root) and the injected git runner.
pub struct WorktreeState {
    pub store: Arc<ProjectStore>,
    pub git: Arc<dyn WorktreeGit>,
}

async fn project_root(store: &ProjectStore, project_id: &str) -> Result<String, String> {
    let project = store
        .get(&ProjectId(project_id.to_string()))
        .await
        .map_err(|e| e.to_string())?;
    Ok(project.root_path.to_string_lossy().into_owned())
}

/// OHS: list a project's cleanup-candidate git worktrees (those under
/// `<project_root>/worktrees/`). Read-only.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_worktrees(
    state: tauri::State<'_, WorktreeState>,
    project_id: String,
) -> Result<Vec<WorktreeEntry>, String> {
    let root = project_root(&state.store, &project_id).await?;
    list_worktrees_inner(state.git.as_ref(), &root)
}

/// OHS: remove one git worktree. The path MUST resolve under the project's
/// `worktrees/` subtree (guarded) or this errors and runs no git command.
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_worktree(
    state: tauri::State<'_, WorktreeState>,
    project_id: String,
    path: String,
) -> Result<(), String> {
    let root = project_root(&state.store, &project_id).await?;
    remove_worktree_inner(state.git.as_ref(), &root, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn root() -> PathBuf {
        PathBuf::from("/home/u/proj")
    }

    #[test]
    fn keeps_only_worktrees_under_the_worktrees_subtree() {
        let porcelain = "\
worktree /home/u/proj
HEAD aaaa1111
branch refs/heads/main

worktree /home/u/proj/worktrees/T-040
HEAD bbbb2222
branch refs/heads/task-40

worktree /home/u/elsewhere/wt
HEAD cccc3333
branch refs/heads/other
";
        let got = parse_porcelain(porcelain, &root());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/home/u/proj/worktrees/T-040");
        assert_eq!(got[0].head, "bbbb2222");
        assert_eq!(got[0].branch, "refs/heads/task-40");
        assert!(got[0].stale);
    }

    #[test]
    fn handles_detached_worktree_without_branch() {
        let porcelain = "\
worktree /home/u/proj/worktrees/detached
HEAD dddd4444
detached
";
        let got = parse_porcelain(porcelain, &root());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].branch, "");
        assert_eq!(got[0].head, "dddd4444");
        assert!(got[0].stale);
    }

    #[test]
    fn empty_output_yields_no_entries() {
        assert!(parse_porcelain("", &root()).is_empty());
    }

    #[test]
    fn under_root_accepts_a_worktree_inside_the_subtree() {
        let ok = worktree_under_root("/home/u/proj", "/home/u/proj/worktrees/T-1");
        assert_eq!(ok.unwrap(), PathBuf::from("/home/u/proj/worktrees/T-1"));
    }

    #[test]
    fn under_root_rejects_a_path_outside_worktrees() {
        assert!(worktree_under_root("/home/u/proj", "/home/u/proj/artifacts/x").is_err());
    }

    #[test]
    fn under_root_rejects_a_traversal_escape() {
        assert!(worktree_under_root("/home/u/proj", "/home/u/proj/worktrees/../../etc").is_err());
    }

    #[test]
    fn under_root_rejects_a_path_in_a_different_project() {
        assert!(worktree_under_root("/home/u/proj", "/home/u/other/worktrees/T-1").is_err());
    }

    #[test]
    fn under_root_rejects_the_worktrees_root_itself() {
        assert!(worktree_under_root("/home/u/proj", "/home/u/proj/worktrees").is_err());
    }

    struct FakeGit {
        porcelain: String,
        removed: Mutex<Vec<(String, String)>>, // (repo, path)
        added: Mutex<Vec<(String, String, String, String)>>, // (repo, path, branch, base)
        reset_paths: Mutex<Vec<String>>,
        fail_remove: bool,
    }
    impl FakeGit {
        fn new(porcelain: &str) -> Self {
            Self {
                porcelain: porcelain.into(),
                removed: Mutex::new(vec![]),
                added: Mutex::new(vec![]),
                reset_paths: Mutex::new(vec![]),
                fail_remove: false,
            }
        }
    }
    impl WorktreeGit for FakeGit {
        fn list_porcelain(&self, _repo: &str) -> Result<String, String> {
            Ok(self.porcelain.clone())
        }
        fn remove(&self, repo: &str, path: &str) -> Result<(), String> {
            if self.fail_remove {
                return Err("git worktree remove failed".into());
            }
            self.removed.lock().unwrap().push((repo.into(), path.into()));
            Ok(())
        }
        fn add(&self, repo: &str, path: &str, branch: &str, base: &str) -> Result<(), String> {
            self.added.lock().unwrap().push((repo.into(), path.into(), branch.into(), base.into()));
            Ok(())
        }
        fn reset(&self, worktree_path: &str) -> Result<(), String> {
            self.reset_paths.lock().unwrap().push(worktree_path.into());
            Ok(())
        }
    }

    #[test]
    fn add_worktree_inner_runs_git_for_an_in_subtree_path() {
        let git = FakeGit::new("");
        add_worktree_inner(
            &git,
            "/home/u/proj",
            "/home/u/proj/worktrees/R-1/alpha",
            "agent-bus/R-1/alpha",
            "HEAD",
        )
        .unwrap();
        let calls = git.added.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            (
                "/home/u/proj".into(),
                "/home/u/proj/worktrees/R-1/alpha".into(),
                "agent-bus/R-1/alpha".into(),
                "HEAD".into()
            )
        );
    }

    #[test]
    fn add_worktree_inner_rejects_an_escape_and_runs_no_git() {
        let git = FakeGit::new("");
        let err = add_worktree_inner(
            &git,
            "/home/u/proj",
            "/home/u/proj/artifacts/x",
            "agent-bus/x",
            "HEAD",
        );
        assert!(err.is_err());
        assert!(git.added.lock().unwrap().is_empty(), "no git on a rejected path");
    }

    #[test]
    fn reset_worktree_inner_runs_git_for_an_in_subtree_path() {
        let git = FakeGit::new("");
        reset_worktree_inner(&git, "/home/u/proj", "/home/u/proj/worktrees/R-1/alpha").unwrap();
        let calls = git.reset_paths.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], "/home/u/proj/worktrees/R-1/alpha");
    }

    #[test]
    fn reset_worktree_inner_rejects_an_escape_and_runs_no_git() {
        let git = FakeGit::new("");
        let err = reset_worktree_inner(&git, "/home/u/proj", "/home/u/proj/worktrees/../../etc");
        assert!(err.is_err());
        assert!(git.reset_paths.lock().unwrap().is_empty(), "no git on a rejected path");
    }

    #[test]
    fn list_worktrees_inner_filters_to_the_subtree() {
        let git = FakeGit::new("\
worktree /home/u/proj
HEAD a

worktree /home/u/proj/worktrees/T-1
HEAD b
branch refs/heads/t1
");
        let got = list_worktrees_inner(&git, "/home/u/proj").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/home/u/proj/worktrees/T-1");
    }

    #[test]
    fn remove_worktree_inner_runs_git_for_an_in_subtree_path() {
        let git = FakeGit::new("");
        remove_worktree_inner(&git, "/home/u/proj", "/home/u/proj/worktrees/T-1").unwrap();
        let calls = git.removed.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            ("/home/u/proj".into(), "/home/u/proj/worktrees/T-1".into())
        );
    }

    #[test]
    fn remove_worktree_inner_rejects_an_escape_and_runs_no_git() {
        let git = FakeGit::new("");
        let err = remove_worktree_inner(&git, "/home/u/proj", "/home/u/proj/artifacts/x");
        assert!(err.is_err());
        assert!(
            git.removed.lock().unwrap().is_empty(),
            "no git command on a rejected path"
        );
    }

    // --- on-disk cleanup (project delete) -------------------------------------

    /// Build a project root with every scaffolded subdir + a sentinel file in
    /// each, and a SEPARATE target_repo dir with its own sentinel. Returns
    /// (root, target_repo).
    fn scaffolded_project() -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("abp-clean-{}", uuid::Uuid::new_v4()));
        let root = base.join("project");
        let repo = base.join("target-repo");
        for name in SCAFFOLDED_SUBDIRS {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("sentinel.txt"), b"x").unwrap();
        }
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("sentinel.txt"), b"real-repo").unwrap();
        (root, repo)
    }

    #[test]
    fn cleanup_removes_scaffolded_subdirs_and_leaves_target_repo_untouched() {
        let (root, repo) = scaffolded_project();
        let git = FakeGit::new(""); // no worktrees reported
        let out = cleanup_project_files_inner(
            &git,
            root.to_str().unwrap(),
            Some(repo.to_str().unwrap()),
        );

        assert!(!out.skipped_for_safety, "should not skip: repo is separate");
        // Every scaffolded subdir is gone.
        for name in SCAFFOLDED_SUBDIRS {
            assert!(!root.join(name).exists(), "{name} should be removed");
        }
        // The root itself survives (we delete pieces, not the whole root).
        assert!(root.exists(), "root_path itself must not be removed");
        // The target repo + its sentinel are completely untouched.
        assert!(repo.exists(), "target_repo must survive");
        assert!(repo.join("sentinel.txt").exists(), "target_repo contents must survive");
        assert_eq!(
            std::fs::read_to_string(repo.join("sentinel.txt")).unwrap(),
            "real-repo"
        );

        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn cleanup_skips_file_deletion_when_target_repo_equals_root() {
        let (root, _repo) = scaffolded_project();
        let git = FakeGit::new("");
        // target_repo == root_path: the hard guard must skip ALL file deletion.
        let out = cleanup_project_files_inner(
            &git,
            root.to_str().unwrap(),
            Some(root.to_str().unwrap()),
        );

        assert!(out.skipped_for_safety, "guard must trip when repo == root");
        assert!(out.removed.is_empty(), "nothing should be removed");
        // The root's scaffolded contents all survive untouched.
        for name in SCAFFOLDED_SUBDIRS {
            assert!(root.join(name).join("sentinel.txt").exists(), "{name} sentinel must survive");
        }
        // No git worktree teardown happened either (guard returns early).
        assert!(git.removed.lock().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn cleanup_skips_file_deletion_when_target_repo_nested_under_root() {
        let (root, _repo) = scaffolded_project();
        let nested = root.join("nested-repo");
        let git = FakeGit::new("");
        let out = cleanup_project_files_inner(
            &git,
            root.to_str().unwrap(),
            Some(nested.to_str().unwrap()),
        );
        assert!(out.skipped_for_safety, "guard must trip when repo is under root");
        for name in SCAFFOLDED_SUBDIRS {
            assert!(root.join(name).exists(), "{name} must survive the skip");
        }
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn cleanup_is_tolerant_of_missing_subdirs_and_no_target_repo() {
        let base = std::env::temp_dir().join(format!("abp-clean-{}", uuid::Uuid::new_v4()));
        let root = base.join("project");
        // Only create ONE of the scaffolded subdirs; the rest are missing.
        std::fs::create_dir_all(root.join("artifacts")).unwrap();
        let git = FakeGit::new("");
        let out = cleanup_project_files_inner(&git, root.to_str().unwrap(), None);
        assert!(!out.skipped_for_safety);
        assert_eq!(out.removed, vec!["artifacts".to_string()]);
        assert!(!root.join("artifacts").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_tears_down_worktrees_via_the_seam() {
        let (root, repo) = scaffolded_project();
        let wt = format!("{}/worktrees/T-1", root.to_string_lossy());
        let porcelain = format!("worktree {wt}\nHEAD aaaa\nbranch refs/heads/t1\n");
        let git = FakeGit::new(&porcelain);
        let out = cleanup_project_files_inner(
            &git,
            root.to_str().unwrap(),
            Some(repo.to_str().unwrap()),
        );
        // The worktree under <root>/worktrees/ was removed via the git seam.
        let calls = git.removed.lock().unwrap();
        assert_eq!(calls.len(), 1, "expected one git worktree remove");
        assert_eq!(calls[0].1, normalize(Path::new(&wt)).to_string_lossy());
        assert_eq!(out.worktrees_removed, vec![wt]);
        drop(calls);
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn target_repo_collision_guard_is_lexical() {
        assert!(target_repo_collides_with_root("/p", Some("/p")));
        assert!(target_repo_collides_with_root("/p", Some("/p/repo")));
        assert!(target_repo_collides_with_root("/p", Some("/p/./repo")));
        assert!(!target_repo_collides_with_root("/p", Some("/other")));
        assert!(!target_repo_collides_with_root("/p", None));
        assert!(!target_repo_collides_with_root("/p", Some("   ")));
    }

    #[test]
    fn worktree_entry_wire_contract_matches_ts() {
        use std::collections::BTreeSet;
        let e = WorktreeEntry {
            path: "/p/worktrees/T-1".into(),
            head: "abc".into(),
            branch: "refs/heads/t1".into(),
            stale: true,
        };
        let v = serde_json::to_value(&e).unwrap();
        let keys: BTreeSet<String> = v.as_object().unwrap().keys().cloned().collect();
        let want: BTreeSet<String> = ["path", "head", "branch", "stale"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(keys, want);
        assert!(v["path"].is_string());
        assert!(v["head"].is_string());
        assert!(v["branch"].is_string());
        assert!(v["stale"].is_boolean());
    }
}
