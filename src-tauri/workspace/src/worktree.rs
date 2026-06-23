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
        fail_remove: bool,
    }
    impl FakeGit {
        fn new(porcelain: &str) -> Self {
            Self {
                porcelain: porcelain.into(),
                removed: Mutex::new(vec![]),
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
