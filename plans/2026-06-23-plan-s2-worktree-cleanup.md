# S2 — Worktree-cleanup prompts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the operator a safe, path-scoped way to find and remove leftover git worktrees under a project's `worktrees/` directory, surfaced as a maintenance affordance in Settings.

**Architecture:** Worktree management belongs to the **Workspace** context — Workspace already owns the project filesystem layout (`paths::project_subdirs()` lists `worktrees`, `resolve_under_root` guards path escapes). We add two Workspace OHS commands — `list_worktrees` / `remove_worktree` — that run real `git worktree list --porcelain` / `git worktree remove` behind an **injected runner seam** (a `WorktreeGit` trait, mirroring the `SpawnFn` seam in `runners/src/claude_cli.rs`), so tests use a fake and never mutate the actual repo. The git idiom (porcelain parsing, `git worktree` argv) is sealed inside Workspace; the OHS returns a plain `WorktreeEntry` DTO. The Settings "projects" section gains per-project maintenance: list stale worktrees and remove with a confirm prompt.

**Tech Stack:** Rust (Workspace crate, sqlx/tauri already present), TypeScript/React (Settings UI, vitest).

---

## Decisions

- **Does per-task git worktree CREATION exist? NO.** Verified by grep across `runtime`, `runners`, `workspace`, `app`: there is **no** `git worktree add` / `git worktree remove` / `git worktree list` call anywhere. Workers run under `--add-dir` **scopes** (`runners/src/scope.rs` `prepare`/`cleanup`), not isolated git worktrees. The `worktrees/` name appears only as:
  - a project sub-dir created empty at scaffold (`paths::project_subdirs()` includes `"worktrees"`),
  - prose in a seed-template prompt body (`pipeline/src/seed_template.rs`),
  - comments that *anticipate* a future worker-commit-in-worktree path (`workspace/src/git_config.rs`, `runtime/src/api.rs`).
  So per-task worktree creation is a **separate, currently-absent feature**. This plan does **not** fabricate it.
- **Scope of S2 (chosen, recommended option):** a self-contained **worktree-cleanup utility + prompt**. We detect git worktrees that actually exist on disk by asking git (`git worktree list --porcelain`) **and** by listing stray directories under the project's `worktrees/`, classify which are tied to no active task (here: all of them, since creation is absent — see staleness note), and offer safe removal. We deliver something real and safe today; when per-task worktree creation lands, this cleanup surface already exists to retire its leftovers.
- **Cleanup-candidate definition (honest — vet F1):** because nothing in this app creates per-task worktrees, every worktree we find under `worktrees/` is *by definition* not owned by any in-flight task this app tracks. We therefore present **all** worktrees discovered under the project's `worktrees/` dir as **cleanup candidates**, labelling them "not tied to an active task". We do **not** call them "stale" as if the app aged them, and we do **not** invent a task↔worktree association that does not exist. The `WorktreeEntry.stale` boolean (kept for forward-compatibility) carries the single honest rule — *is it under `worktrees/` and tied to no active task? → it's a cleanup candidate* — and when per-task creation lands the same command can subtract live-task worktrees. (Vet F1: register a "Worktree cleanup" entry in `DOMAIN.md` → Workspace; see Task 9.)
- **Safety / path-scoping (hard rule):** `remove_worktree(path)` **must reject any path that does not resolve to a descendant of `<project_root>/worktrees/`**. We reuse the `resolve_under_root` escape-guard discipline and additionally require the resolved path to live under the `worktrees/` subtree. A path outside that subtree returns an error and runs **no** git command. This is enforced in pure code (`worktree_under_root`) and unit-tested independently of git.
- **Git seam (testability + ACL):** real `git` execution is isolated behind a `WorktreeGit` trait with two methods (`list_porcelain(repo)`, `remove(repo, path)`). Production impl shells out to `git`; tests inject a fake. No `std::process::Command` / git argv / porcelain text crosses the OHS — `list_worktrees` returns `Vec<WorktreeEntry>` and `remove_worktree` returns `Result<(), String>`.
- **Where the UI fits:** the Settings "projects" section (`SettingsView.tsx`). Each project row gains a "worktrees" expander that lists discovered worktrees with a per-entry "remove" + a confirm prompt. This is the maintenance area the operator already uses to remove projects.
- **Auto-decisions:** every fork resolved with the recommended option (Workspace ownership; trait seam; `worktrees/`-subtree scoping; `list_worktrees`/`remove_worktree` naming; Settings/projects surface).

---

## File Structure

- **Create** `src-tauri/workspace/src/worktree.rs` — the `WorktreeEntry` DTO, the `WorktreeGit` seam trait + real `GitCli` impl, the pure `parse_porcelain` + `worktree_under_root` helpers, and the OHS commands `list_worktrees` / `remove_worktree`. One responsibility: worktree discovery + safe removal for a project.
- **Modify** `src-tauri/workspace/src/lib.rs` — `pub mod worktree;`.
- **Modify** `src-tauri/workspace/src/api.rs` — add the two new commands to `tools()` (OHS contract list).
- **Modify** `src-tauri/app/src/lib.rs` — register `list_worktrees` / `remove_worktree` in `generate_handler!` and construct the `WorktreeState` with the real `GitCli` runner.
- **Modify** `src/ipc/workspace.ts` — `WorktreeEntry` type + `listWorktrees` / `removeWorktree` IPC wrappers.
- **Modify** `src/components/SettingsView.tsx` — per-project worktrees expander + remove-with-confirm.
- **Modify** `src/App.tsx` — pass `listWorktrees` / `removeWorktree` props into `SettingsView`.
- **Create** `src/components/SettingsView.test.tsx` — vitest for the worktrees expander (list, confirm, remove).

---

## Task 1: Worktree DTO + porcelain parser (pure)

**Files:**
- Create: `src-tauri/workspace/src/worktree.rs`
- Modify: `src-tauri/workspace/src/lib.rs`

- [ ] **Step 1: Add the module to lib.rs**

In `src-tauri/workspace/src/lib.rs`, after `pub mod paths;` add:

```rust
pub mod worktree;
```

- [ ] **Step 2: Write the failing test for the porcelain parser**

Create `src-tauri/workspace/src/worktree.rs` with the DTO, a `parse_porcelain` stub, and tests:

```rust
//! Worktree cleanup for the Workspace context. Workspace owns the project
//! filesystem layout (`paths::project_subdirs` includes `worktrees`), so it
//! owns finding + safely removing leftover git worktrees. The real `git`
//! calls are isolated behind the `WorktreeGit` seam (mirrors the `SpawnFn`
//! seam in `runners/src/claude_cli.rs`) so tests never touch a real repo.
//!
//! IMPORTANT (honest scope): this app does NOT yet create per-task git
//! worktrees — workers run under `--add-dir` scopes, not isolated worktrees.
//! This module is a self-contained cleanup utility for worktrees that exist
//! under `<project_root>/worktrees/` (e.g. created by hand or by a future
//! per-task-worktree feature). Every such worktree is reported as a cleanup
//! candidate because none is tied to an active task today.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// One worktree discovered for a project. The DTO crossing the OHS — no git
/// idiom (no porcelain text, no argv) leaks past this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeEntry {
    /// Absolute path to the worktree directory.
    pub path: String,
    /// The checked-out HEAD (commit sha) if git reported one, else empty.
    pub head: String,
    /// The branch ref if any (e.g. "refs/heads/feature"), else empty.
    pub branch: String,
    /// True when this worktree is a cleanup candidate: it lives under the
    /// project's `worktrees/` subtree and is tied to no active task. Today
    /// that is every worktree under `worktrees/` (creation is absent).
    pub stale: bool,
}

/// Parse `git worktree list --porcelain` output into entries, keeping ONLY the
/// worktrees whose path is a descendant of `<project_root>/worktrees/` and
/// marking them `stale = true`. The main repo worktree and any worktree
/// outside the project's `worktrees/` subtree are dropped (never offered for
/// removal). Pure: no IO.
pub fn parse_porcelain(porcelain: &str, project_root: &Path) -> Vec<WorktreeEntry> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from("/home/u/proj")
    }

    #[test]
    fn keeps_only_worktrees_under_the_worktrees_subtree() {
        // Block 1 = the main repo (dropped). Block 2 = under worktrees/ (kept).
        // Block 3 = a worktree elsewhere (dropped — never offered for removal).
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
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p workspace worktree::tests`
Expected: FAIL — `unimplemented!()` panics / `not yet implemented`.

- [ ] **Step 4: Implement `parse_porcelain`**

Replace the `parse_porcelain` stub body:

```rust
pub fn parse_porcelain(porcelain: &str, project_root: &Path) -> Vec<WorktreeEntry> {
    let worktrees_root = project_root.join("worktrees");
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut head = String::new();
    let mut branch = String::new();

    let mut flush = |path: &mut Option<String>, head: &mut String, branch: &mut String, out: &mut Vec<WorktreeEntry>| {
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
    let c = normalize(candidate);
    let b = normalize(base);
    c.starts_with(&b)
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
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p workspace worktree::tests`
Expected: PASS (3 tests).

- [ ] **Step 6: Add a serde wire-contract test for `WorktreeEntry` (vet F2)**

Mirror `workspace/src/contract_tests.rs` — lock the JSON key set/casing against the
TS interface so a drift fails `cargo test`. Add to the `tests` module in
`worktree.rs`:

```rust
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
        let want: BTreeSet<String> =
            ["path", "head", "branch", "stale"].iter().map(|s| s.to_string()).collect();
        assert_eq!(keys, want);
        assert!(v["path"].is_string());
        assert!(v["head"].is_string());
        assert!(v["branch"].is_string());
        assert!(v["stale"].is_boolean());
    }
```

Run: `cd src-tauri && cargo test -p workspace worktree::tests`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/workspace/src/worktree.rs src-tauri/workspace/src/lib.rs
git commit -m "feat(workspace): worktree porcelain parser + DTO, scoped to worktrees/ subtree"
```

---

## Task 2: Path-scoping guard for removal (pure)

**Files:**
- Modify: `src-tauri/workspace/src/worktree.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `worktree.rs`:

```rust
    #[test]
    fn under_root_accepts_a_worktree_inside_the_subtree() {
        let ok = worktree_under_root("/home/u/proj", "/home/u/proj/worktrees/T-1");
        assert_eq!(ok.unwrap(), PathBuf::from("/home/u/proj/worktrees/T-1"));
    }

    #[test]
    fn under_root_rejects_a_path_outside_worktrees() {
        // Inside the project root but NOT under worktrees/ — must be rejected.
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p workspace worktree::tests::under_root`
Expected: FAIL — `worktree_under_root` not found.

- [ ] **Step 3: Implement `worktree_under_root`**

Add to `worktree.rs` (above the `tests` module):

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p workspace worktree::tests`
Expected: PASS (all worktree tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/workspace/src/worktree.rs
git commit -m "feat(workspace): worktree removal path-scoping guard (rejects escapes)"
```

---

## Task 3: Git seam trait + fake, list/remove commands

**Files:**
- Modify: `src-tauri/workspace/src/worktree.rs`

- [ ] **Step 1: Write the failing test for the seam-driven commands**

Add to `worktree.rs` (the seam + fake live in the module; commands are tested via inner fns that take the trait, not the Tauri `State`):

```rust
    use std::sync::Mutex;

    /// Fake git: returns canned porcelain for list, records remove() calls.
    struct FakeGit {
        porcelain: String,
        removed: Mutex<Vec<(String, String)>>, // (repo, path)
        fail_remove: bool,
    }
    impl FakeGit {
        fn new(porcelain: &str) -> Self {
            Self { porcelain: porcelain.into(), removed: Mutex::new(vec![]), fail_remove: false }
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
        assert_eq!(calls[0], ("/home/u/proj".into(), "/home/u/proj/worktrees/T-1".into()));
    }

    #[test]
    fn remove_worktree_inner_rejects_an_escape_and_runs_no_git() {
        let git = FakeGit::new("");
        let err = remove_worktree_inner(&git, "/home/u/proj", "/home/u/proj/artifacts/x");
        assert!(err.is_err());
        assert!(git.removed.lock().unwrap().is_empty(), "no git command on a rejected path");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p workspace worktree::tests`
Expected: FAIL — `WorktreeGit`, `list_worktrees_inner`, `remove_worktree_inner` not found.

- [ ] **Step 3: Implement the seam, the real impl, and the inner command fns**

Add to `worktree.rs` (above the `tests` module):

```rust
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
pub fn list_worktrees_inner(git: &dyn WorktreeGit, project_root: &str) -> Result<Vec<WorktreeEntry>, String> {
    let porcelain = git.list_porcelain(project_root)?;
    Ok(parse_porcelain(&porcelain, Path::new(project_root)))
}

/// Remove one worktree after the path-scoping guard passes. Runs NO git command
/// on a rejected path (the guard returns Err first). Testable with a fake git.
pub fn remove_worktree_inner(git: &dyn WorktreeGit, project_root: &str, path: &str) -> Result<(), String> {
    let resolved = worktree_under_root(project_root, path)?;
    git.remove(project_root, &resolved.to_string_lossy())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p workspace worktree::tests`
Expected: PASS (all worktree tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/workspace/src/worktree.rs
git commit -m "feat(workspace): WorktreeGit seam + GitCli impl + list/remove inner commands"
```

---

## Task 4: Tauri OHS commands + state

**Files:**
- Modify: `src-tauri/workspace/src/worktree.rs`
- Modify: `src-tauri/workspace/src/api.rs`

- [ ] **Step 1: Add the Tauri command wrappers + state to `worktree.rs`**

Append to `worktree.rs` (before `#[cfg(test)]`):

```rust
use crate::api::WorkspaceState;
use agent_bus_core::ProjectId;
use std::sync::Arc;

/// State for the worktree commands: holds the project store (to resolve a
/// project's root) and the injected git runner.
pub struct WorktreeState {
    pub store: Arc<crate::store::ProjectStore>,
    pub git: Arc<dyn WorktreeGit>,
}

async fn project_root(store: &crate::store::ProjectStore, project_id: &str) -> Result<String, String> {
    let project = store.get(&ProjectId(project_id.to_string())).await.map_err(|e| e.to_string())?;
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
```

Note: the `WorkspaceState` import is not actually needed; remove the `use crate::api::WorkspaceState;` line if clippy flags it unused. Keep only the imports the code uses.

- [ ] **Step 2: Add the commands to the OHS `tools()` contract in `api.rs`**

In `src-tauri/workspace/src/api.rs`, inside the `vec![ ... ]` returned by `tools()`, after the `read_artifact` ToolSpec (locate the closing `},` of the last entry) add:

```rust
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
```

- [ ] **Step 3: Add a `tools()` assertion test in `api.rs`**

In the `tests` module of `api.rs`, add:

```rust
    #[test]
    fn tools_publishes_worktree_commands_under_workspace() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "list_worktrees" && s.supplier_context == "workspace"));
        assert!(t.iter().any(|s| s.name == "remove_worktree" && s.supplier_context == "workspace"));
    }
```

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test -p workspace`
Expected: PASS (worktree tests + new tools assertion + existing tests green).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/workspace/src/worktree.rs src-tauri/workspace/src/api.rs
git commit -m "feat(workspace): list_worktrees/remove_worktree OHS commands + tools() contract"
```

---

## Task 5: Wire commands at the composition root

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Construct and manage `WorktreeState`**

In `src-tauri/app/src/lib.rs`, near the existing `handle.manage(WorkspaceState { store: project_store.clone() });` line (~894), add:

```rust
                handle.manage(workspace::worktree::WorktreeState {
                    store: project_store.clone(),
                    git: std::sync::Arc::new(workspace::worktree::GitCli),
                });
```

(Adjust `std::sync::Arc` to whatever Arc alias the file already uses — the file imports `use std::sync::Arc as StdArc;` in tests; in the setup block use the fully-qualified `std::sync::Arc` or the crate's existing import.)

- [ ] **Step 2: Register the commands in `generate_handler!`**

In the `tauri::generate_handler![ ... ]` list, after `workspace::api::workspace_remove_project,` add:

```rust
            workspace::worktree::list_worktrees,
            workspace::worktree::remove_worktree,
```

- [ ] **Step 3: Build to verify wiring compiles**

Run: `cd src-tauri && cargo check --workspace`
Expected: clean (no errors).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): register + wire worktree cleanup commands with real GitCli"
```

---

## Task 6: Frontend IPC

**Files:**
- Modify: `src/ipc/workspace.ts`

- [ ] **Step 1: Add the type + wrappers**

In `src/ipc/workspace.ts`, after the `removeProject` function add:

```ts
export interface WorktreeEntry {
  path: string;
  head: string;
  branch: string;
  stale: boolean;
}

export async function listWorktrees(projectId: string): Promise<WorktreeEntry[]> {
  return await invoke<WorktreeEntry[]>("list_worktrees", { project_id: projectId });
}

export async function removeWorktree(projectId: string, path: string): Promise<void> {
  await invoke<void>("remove_worktree", { project_id: projectId, path });
}
```

- [ ] **Step 2: Typecheck**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun run build`
Expected: build succeeds (or at minimum no TS error from this file; full build runs in Task 9).

- [ ] **Step 3: Commit**

```bash
git add src/ipc/workspace.ts
git commit -m "feat(ipc): listWorktrees/removeWorktree wrappers + WorktreeEntry type"
```

---

## Task 7: Settings worktrees expander UI

**Files:**
- Modify: `src/components/SettingsView.tsx`

- [ ] **Step 1: Extend the props**

In `SettingsView.tsx`, add to `SettingsViewProps` (after `onRemoveProject`):

```ts
  onListWorktrees: (projectId: string) => Promise<WorktreeEntry[]>;
  onRemoveWorktree: (projectId: string, path: string) => Promise<void>;
```

And update the import on line 3:

```ts
import type { GitConfig, Project, WorktreeEntry } from "../ipc/workspace";
```

- [ ] **Step 2: Destructure the new props**

In the `const { ... } = props;` block add `onListWorktrees, onRemoveWorktree,`.

- [ ] **Step 3: Add a per-project worktrees sub-component**

Add this component above `export function SettingsView` in the same file:

```tsx
function ProjectWorktrees(props: {
  project: Project;
  onList: (projectId: string) => Promise<WorktreeEntry[]>;
  onRemove: (projectId: string, path: string) => Promise<void>;
}) {
  const { project, onList, onRemove } = props;
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<WorktreeEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmPath, setConfirmPath] = useState<string | null>(null);

  async function load() {
    setBusy(true);
    setError(null);
    try {
      setEntries(await onList(project.id));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function toggle() {
    const next = !open;
    setOpen(next);
    if (next && entries === null) await load();
  }

  async function remove(path: string) {
    setBusy(true);
    setError(null);
    try {
      await onRemove(project.id, path);
      setConfirmPath(null);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  function basename(p: string): string {
    const parts = p.split("/").filter(Boolean);
    return parts[parts.length - 1] ?? p;
  }

  return (
    <div style={{ marginTop: 6 }}>
      <button
        onClick={toggle}
        aria-expanded={open}
        style={{ background: "none", border: "none", color: "var(--text-3)", fontSize: 11, cursor: "pointer", padding: 0 }}
      >
        {open ? "▾" : "▸"} worktrees
      </button>
      {open && (
        <div style={{ marginTop: 6, paddingLeft: 14 }}>
          {busy && entries === null && (
            <div style={{ fontSize: 11, color: "var(--text-3)" }}>loading…</div>
          )}
          {error && <div style={{ fontSize: 11, color: "var(--danger, #d66)" }}>{error}</div>}
          {entries !== null && entries.length === 0 && (
            <div style={{ fontSize: 11, color: "var(--text-3)" }}>no worktrees to clean up.</div>
          )}
          {entries?.map((w) => (
            <div key={w.path} style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 0" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 12, color: "var(--text)" }}>{basename(w.path)}</div>
                <div style={{ fontSize: 11, color: "var(--text-3)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {w.branch || w.head || w.path} · not tied to an active task
                </div>
              </div>
              {confirmPath === w.path ? (
                <>
                  <Button disabled={busy} onClick={() => remove(w.path)}>confirm remove</Button>
                  <Button disabled={busy} onClick={() => setConfirmPath(null)}>cancel</Button>
                </>
              ) : (
                <Button disabled={busy} onClick={() => setConfirmPath(w.path)}>remove</Button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Render it inside each project row**

In the `projects.map((p) => ( ... ))` block, change the row so the worktrees expander sits under the name/path. Replace the existing row's outer `<div key={p.id} ...>` content so the expander renders after the name+remove flex row:

```tsx
        {projects.map((p) => (
          <div key={p.id} style={{ padding: "6px 0", borderBottom: "1px solid var(--border)" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 12, color: "var(--text)" }}>
                  {p.name}{p.id === activeProjectId ? " (active)" : ""}
                </div>
                <div style={{ fontSize: 11, color: "var(--text-3)" }}>{p.root_path}</div>
              </div>
              <Button onClick={() => onRemoveProject(p.id)}>remove</Button>
            </div>
            <ProjectWorktrees project={p} onList={onListWorktrees} onRemove={onRemoveWorktree} />
          </div>
        ))}
```

- [ ] **Step 5: Build to typecheck**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun run build`
Expected: build succeeds.

- [ ] **Step 6: Commit**

```bash
git add src/components/SettingsView.tsx
git commit -m "feat(settings): per-project worktrees expander with remove + confirm prompt"
```

---

## Task 8: Frontend tests + App wiring

**Files:**
- Create: `src/components/SettingsView.test.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: Write the failing component test**

Create `src/components/SettingsView.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { SettingsView } from "./SettingsView";
import type { Project, WorktreeEntry } from "../ipc/workspace";

const project: Project = {
  id: "p1",
  name: "Proj",
  root_path: "/home/u/proj",
  active_pipeline_id: null,
  created_at: 1,
  updated_at: 1,
};

function baseProps(overrides: Partial<React.ComponentProps<typeof SettingsView>> = {}) {
  return {
    usage: null,
    onSetBudget: vi.fn(),
    onSetAutoMeter: vi.fn(),
    apiKeyPresent: false,
    onSetApiKey: vi.fn(),
    onClearApiKey: vi.fn(),
    gitConfig: { author_name: "", author_email: "" },
    onSaveGitConfig: vi.fn(),
    projects: [project],
    activeProjectId: null,
    onRemoveProject: vi.fn(),
    onListWorktrees: vi.fn(),
    onRemoveWorktree: vi.fn(),
    ...overrides,
  } as React.ComponentProps<typeof SettingsView>;
}

describe("SettingsView worktrees", () => {
  it("lists worktrees when the expander is opened", async () => {
    const entries: WorktreeEntry[] = [
      { path: "/home/u/proj/worktrees/T-1", head: "abc", branch: "refs/heads/t1", stale: true },
    ];
    const onListWorktrees = vi.fn().mockResolvedValue(entries);
    render(<SettingsView {...baseProps({ onListWorktrees })} />);

    fireEvent.click(screen.getByRole("button", { name: /worktrees/i }));
    await waitFor(() => expect(onListWorktrees).toHaveBeenCalledWith("p1"));
    expect(await screen.findByText("T-1")).toBeInTheDocument();
  });

  it("requires confirm before removing and calls onRemoveWorktree", async () => {
    const entries: WorktreeEntry[] = [
      { path: "/home/u/proj/worktrees/T-1", head: "abc", branch: "", stale: true },
    ];
    const onListWorktrees = vi.fn().mockResolvedValue(entries);
    const onRemoveWorktree = vi.fn().mockResolvedValue(undefined);
    render(<SettingsView {...baseProps({ onListWorktrees, onRemoveWorktree })} />);

    fireEvent.click(screen.getByRole("button", { name: /worktrees/i }));
    await screen.findByText("T-1");

    fireEvent.click(screen.getByRole("button", { name: "remove" }));
    expect(onRemoveWorktree).not.toHaveBeenCalled(); // not yet — needs confirm
    fireEvent.click(screen.getByRole("button", { name: "confirm remove" }));
    await waitFor(() =>
      expect(onRemoveWorktree).toHaveBeenCalledWith("p1", "/home/u/proj/worktrees/T-1"),
    );
  });

  it("shows the empty state when there are no worktrees", async () => {
    const onListWorktrees = vi.fn().mockResolvedValue([]);
    render(<SettingsView {...baseProps({ onListWorktrees })} />);
    fireEvent.click(screen.getByRole("button", { name: /worktrees/i }));
    expect(await screen.findByText(/no worktrees to clean up/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run src/components/SettingsView.test.tsx`
Expected: FAIL — `onListWorktrees`/`onRemoveWorktree` props not yet passed by some callers, or (if Task 7 done) PASS for the component but App.tsx still missing props. The component test itself should pass once Task 7 is in; this step proves the test exercises the new UI.

If it fails because the "remove" button name is ambiguous (the project also has a "remove" button), scope the query: use `within()` on the worktrees row. Adjust the test to:

```tsx
    const row = (await screen.findByText("T-1")).closest("div")!.parentElement!;
    fireEvent.click(within(row).getByRole("button", { name: "remove" }));
```

and add `within` to the import from `@testing-library/react`. Re-run.

- [ ] **Step 3: Verify the test passes**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run src/components/SettingsView.test.tsx`
Expected: PASS (3 tests).

- [ ] **Step 4: Wire the props in App.tsx**

In `src/App.tsx`, add `listWorktrees, removeWorktree` to the existing import from `./ipc/workspace`, then in the `<SettingsView ... />` block add (after `onRemoveProject`):

```tsx
            onListWorktrees={(id) => listWorktrees(id)}
            onRemoveWorktree={async (id, path) => { await removeWorktree(id, path); }}
```

- [ ] **Step 5: Run the full vitest + build to verify App still compiles**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run && /opt/homebrew/bin/bun run build`
Expected: all vitest green, build succeeds.

- [ ] **Step 6: Commit**

```bash
git add src/components/SettingsView.test.tsx src/App.tsx
git commit -m "test(settings): worktrees expander tests; wire App props"
```

---

## Task 9: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Rust tests**

Run: `cd src-tauri && cargo test --workspace`
Expected: all green (new worktree tests included; existing suite unchanged).

- [ ] **Step 2: Check + clippy**

Run: `cd src-tauri && cargo check --workspace && cargo clippy --workspace -- -D warnings`
Expected: clean (no warnings — fix any clippy nits, e.g. unused imports, `to_string_lossy` patterns).

- [ ] **Step 3: Frontend**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run && /opt/homebrew/bin/bun run build`
Expected: all vitest green, build succeeds.

- [ ] **Step 4: Register the worktree-cleanup language in DOMAIN.md (vet F1 + F3)**

In `DOMAIN.md` → `### Workspace`, after the "Git author identity" entry, add:

```markdown
- **Worktree cleanup** — a self-contained utility (S2) to find + safely remove leftover git worktrees under a project's `worktrees/`. `list_worktrees` / `remove_worktree` (Workspace OHS) run real `git worktree list --porcelain` / `git worktree remove` behind the **`WorktreeGit` seam** (real `GitCli` + a fake in tests — the same ACL-seam discipline as `SpawnFn`/`KeychainStore`; no git idiom crosses the OHS). `remove_worktree` is **path-scoped**: it rejects any path not resolving under `<project_root>/worktrees/` and runs no git command on a rejected path. **Honest scope:** per-task worktree *creation* is NOT implemented — workers run under `--add-dir` **scopes** (`runners/src/scope.rs`), not isolated worktrees — so every worktree discovered under `worktrees/` is a **cleanup candidate** (tied to no active task), never called "stale". When per-task creation lands, the same command can subtract live-task worktrees.
```

- [ ] **Step 5: Commit DOMAIN.md + any clippy/lint fixes**

```bash
git add -A
git commit -m "docs(domain): register Worktree cleanup language (S2 vet F1/F3); clippy fixes"
```

---

## Self-Review notes

- **Spec coverage:** detection of leftover worktrees (Task 1+3 `list_worktrees`), safe removal (Task 2+3 `remove_worktree` guarded), cleanup prompt UI (Task 7+8 confirm flow), git behind a seam/fake (Task 3), path-scoping never escapes `worktrees/` (Task 2). Honesty about creation-vs-cleanup recorded in `## Decisions`.
- **Types consistent:** `WorktreeEntry` { path, head, branch, stale } identical Rust↔TS; `WorktreeGit` methods `list_porcelain`/`remove`; inner fns `list_worktrees_inner`/`remove_worktree_inner`; commands `list_worktrees`/`remove_worktree`.
- **No real-repo mutation in tests:** every Rust test uses `FakeGit`; the real `GitCli` is only constructed at the composition root (Task 5) and never exercised by tests.
