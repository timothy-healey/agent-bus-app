# App-managed Project Home Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** New projects live in `<app_data>/projects/<id>/` and every worktree git command runs in the target repo, so the target repo only gains `agent-bus/<run>/<item>` branches.

**Architecture:** The Workspace worktree functions separate *which repo git runs in* (`repo`) from *which subtree paths must stay inside* (`project_root`). The app's `GitCliWorktreeProvider` passes the effective target repo as `repo`. Project creation makes `root` optional (`None` → app-data dir), requires a git-repo target in that case, and the wizard hides the root picker behind "Custom location".

**Tech Stack:** Rust (Tauri 2, sqlx/SQLite, tokio), React 18 + TypeScript, vitest + Testing Library.

**Spec:** `docs/superpowers/specs/2026-10-05-app-managed-project-home-design.md`

## Global Constraints

- The path guard never loosens: every worktree path must resolve under `<project_root>/worktrees/` (`worktree_under_root`), or no git command runs.
- Default project root: `app_data_project_dir(app_data, id)` = `<app_data>/projects/<id>`.
- Branch name format stays `agent-bus/<run_id>/<item_key>`; base ref stays `"HEAD"`.
- Error strings, verbatim: `a target repo is required when the project lives in app data`, `target repo is not a git repository: <path>`, `target repo must be an absolute path: <path>`.
- No data migration; existing projects keep their stored `root_path`.
- `agent-bus/…` branches are never deleted by the app.
- Commands: Rust `cargo test --manifest-path src-tauri/Cargo.toml --workspace`; frontend `npx vitest run`; types `npx tsc --noEmit`. Run from the repo root `Efforts/agent-bus-app`.
- Code comments state the rule, never when or who decided it.

## Review Focus

1. **Target typed with `~`** (e.g. `~/code/app`): expanded to an absolute path *before* the git-repo check and stored expanded. Pinned in Task 4.
2. **Relative target** (e.g. `./app` or `app`): rejected with `target repo must be an absolute path`, not resolved against the app's cwd. Pinned in Task 4.
3. **Whitespace-only target with no custom root**: treated as missing, so the "target repo is required" error, and nothing written. Pinned in Task 4.
4. **Custom location opened, root typed, then closed again**: the root is ignored (`null` sent) and the target becomes required again. Pinned in Task 5.
5. **Only the target repo typed, then Cancel**: counts as a dirty draft, so the discard confirm appears. Pinned in Task 5.

---

### Task 1: Worktree functions take the repo separately from the project root

**Files:**
- Modify: `src-tauri/workspace/src/worktree.rs` (`list_worktrees_inner`, `remove_worktree_inner`, `add_worktree_inner`, `cleanup_project_files_inner`, `project_root` helper, `list_worktrees`/`remove_worktree` OHS commands, tests module)
- Modify: `src-tauri/app/src/lib.rs:159-165` (the one external call to `add_worktree_inner`)

**Interfaces:**
- Produces:
  - `pub fn list_worktrees_inner(git: &dyn WorktreeGit, repo: &str, project_root: &str) -> Result<Vec<WorktreeEntry>, String>`
  - `pub fn remove_worktree_inner(git: &dyn WorktreeGit, repo: &str, project_root: &str, path: &str) -> Result<(), String>`
  - `pub fn add_worktree_inner(git: &dyn WorktreeGit, repo: &str, project_root: &str, path: &str, branch: &str, base_ref: &str) -> Result<(), String>`
  - `cleanup_project_files_inner(git, root_path, target_repo)`: signature unchanged; teardown runs git in `target_repo` (non-empty) else `root_path`.

- [ ] **Step 1: Make the test fake record which repo `list` ran in**

In the `tests` module of `src-tauri/workspace/src/worktree.rs`, replace the `FakeGit` struct, its `new`, and its `list_porcelain` impl:

```rust
    struct FakeGit {
        porcelain: String,
        listed: Mutex<Vec<String>>, // repo
        removed: Mutex<Vec<(String, String)>>, // (repo, path)
        added: Mutex<Vec<(String, String, String, String)>>, // (repo, path, branch, base)
        reset_paths: Mutex<Vec<String>>,
        fail_remove: bool,
    }
    impl FakeGit {
        fn new(porcelain: &str) -> Self {
            Self {
                porcelain: porcelain.into(),
                listed: Mutex::new(vec![]),
                removed: Mutex::new(vec![]),
                added: Mutex::new(vec![]),
                reset_paths: Mutex::new(vec![]),
                fail_remove: false,
            }
        }
    }
    impl WorktreeGit for FakeGit {
        fn list_porcelain(&self, repo: &str) -> Result<String, String> {
            self.listed.lock().unwrap().push(repo.into());
            Ok(self.porcelain.clone())
        }
```

(The `remove`, `add`, `reset` impls below it are unchanged.)

- [ ] **Step 2: Write the failing tests**

Add to the `tests` module:

```rust
    #[test]
    fn add_worktree_inner_runs_git_in_the_repo_not_the_root() {
        let git = FakeGit::new("");
        add_worktree_inner(
            &git,
            "/home/u/code",
            "/home/u/proj",
            "/home/u/proj/worktrees/R-1/alpha",
            "agent-bus/R-1/alpha",
            "HEAD",
        )
        .unwrap();
        let calls = git.added.lock().unwrap();
        assert_eq!(calls[0].0, "/home/u/code");
        assert_eq!(calls[0].1, "/home/u/proj/worktrees/R-1/alpha");
    }

    #[test]
    fn add_worktree_inner_guards_on_the_root_even_when_the_path_is_inside_the_repo() {
        let git = FakeGit::new("");
        let err = add_worktree_inner(
            &git,
            "/home/u/code",
            "/home/u/proj",
            "/home/u/code/worktrees/R-1/alpha",
            "agent-bus/R-1/alpha",
            "HEAD",
        );
        assert!(err.is_err());
        assert!(git.added.lock().unwrap().is_empty(), "no git on a rejected path");
    }

    #[test]
    fn list_worktrees_inner_queries_the_repo_and_filters_to_the_root() {
        let git = FakeGit::new("\
worktree /home/u/code
HEAD a

worktree /home/u/proj/worktrees/T-1
HEAD b
branch refs/heads/t1
");
        let got = list_worktrees_inner(&git, "/home/u/code", "/home/u/proj").unwrap();
        assert_eq!(*git.listed.lock().unwrap(), vec!["/home/u/code".to_string()]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/home/u/proj/worktrees/T-1");
    }

    #[test]
    fn remove_worktree_inner_runs_git_in_the_repo() {
        let git = FakeGit::new("");
        remove_worktree_inner(&git, "/home/u/code", "/home/u/proj", "/home/u/proj/worktrees/T-1").unwrap();
        let calls = git.removed.lock().unwrap();
        assert_eq!(calls[0], ("/home/u/code".into(), "/home/u/proj/worktrees/T-1".into()));
    }

    #[test]
    fn cleanup_tears_down_worktrees_in_the_target_repo() {
        let (root, repo) = scaffolded_project();
        let wt = format!("{}/worktrees/T-1", root.to_string_lossy());
        let porcelain = format!("worktree {wt}\nHEAD aaaa\nbranch refs/heads/t1\n");
        let git = FakeGit::new(&porcelain);
        let out = cleanup_project_files_inner(&git, root.to_str().unwrap(), Some(repo.to_str().unwrap()));
        assert_eq!(*git.listed.lock().unwrap(), vec![repo.to_string_lossy().into_owned()]);
        let calls = git.removed.lock().unwrap();
        assert_eq!(calls[0].0, repo.to_string_lossy());
        assert_eq!(out.worktrees_removed, vec![wt]);
        drop(calls);
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p workspace worktree`
Expected: compile errors `this function takes 5 arguments but 6 arguments were supplied` (add), `takes 2 arguments but 3` (list), `takes 3 arguments but 4` (remove). That is the RED for the signature change.

- [ ] **Step 4: Implement the split**

Replace the three `*_inner` functions:

```rust
/// List the project's cleanup-candidate worktrees. Git runs in `repo` (the repo
/// the worktrees belong to); entries are filtered to `<project_root>/worktrees/`.
/// Testable with a fake git.
pub fn list_worktrees_inner(
    git: &dyn WorktreeGit,
    repo: &str,
    project_root: &str,
) -> Result<Vec<WorktreeEntry>, String> {
    let porcelain = git.list_porcelain(repo)?;
    Ok(parse_porcelain(&porcelain, Path::new(project_root)))
}

/// Remove one worktree after the path-scoping guard passes. The guard checks
/// `project_root`; git runs in `repo`. Runs NO git command on a rejected path.
pub fn remove_worktree_inner(
    git: &dyn WorktreeGit,
    repo: &str,
    project_root: &str,
    path: &str,
) -> Result<(), String> {
    let resolved = worktree_under_root(project_root, path)?;
    git.remove(repo, &resolved.to_string_lossy())
}

/// Create one worktree after the path-scoping guard passes. The guard checks
/// `project_root`; git runs in `repo` and the branch is created at `base_ref`
/// of that repo. Runs NO git command on a rejected path.
pub fn add_worktree_inner(
    git: &dyn WorktreeGit,
    repo: &str,
    project_root: &str,
    path: &str,
    branch: &str,
    base_ref: &str,
) -> Result<(), String> {
    let resolved = worktree_under_root(project_root, path)?;
    git.add(repo, &resolved.to_string_lossy(), branch, base_ref)
}
```

In `cleanup_project_files_inner`, directly after the collision-guard `if` block returns, add:

```rust
    // Worktrees are registered in the target repo when there is one.
    let repo = target_repo.filter(|s| !s.trim().is_empty()).unwrap_or(root_path);
```

and change the teardown calls to `list_worktrees_inner(git, repo, root_path)` and `remove_worktree_inner(git, repo, root_path, &entry.path)`.

Replace the `project_root` helper and the two OHS commands:

```rust
/// A project's `(repo, root)`: git runs in `repo` (the target repo, else the
/// root); paths are guarded against `root`.
async fn project_repo_and_root(store: &ProjectStore, project_id: &str) -> Result<(String, String), String> {
    let project = store
        .get(&ProjectId(project_id.to_string()))
        .await
        .map_err(|e| e.to_string())?;
    let root = project.root_path.to_string_lossy().into_owned();
    let repo = project
        .target_repo
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| root.clone());
    Ok((repo, root))
}

/// OHS: list a project's cleanup-candidate git worktrees (those under
/// `<project_root>/worktrees/`). Read-only.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_worktrees(
    state: tauri::State<'_, WorktreeState>,
    project_id: String,
) -> Result<Vec<WorktreeEntry>, String> {
    let (repo, root) = project_repo_and_root(&state.store, &project_id).await?;
    list_worktrees_inner(state.git.as_ref(), &repo, &root)
}

/// OHS: remove one git worktree. The path MUST resolve under the project's
/// `worktrees/` subtree (guarded) or this errors and runs no git command.
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_worktree(
    state: tauri::State<'_, WorktreeState>,
    project_id: String,
    path: String,
) -> Result<(), String> {
    let (repo, root) = project_repo_and_root(&state.store, &project_id).await?;
    remove_worktree_inner(state.git.as_ref(), &repo, &root, &path)
}
```

Update the existing tests to the new arity, passing the root as the repo so their assertions hold unchanged:
- `add_worktree_inner_runs_git_for_an_in_subtree_path` and `add_worktree_inner_rejects_an_escape_and_runs_no_git`: insert `"/home/u/proj",` as the new second argument.
- `list_worktrees_inner_filters_to_the_subtree`: `list_worktrees_inner(&git, "/home/u/proj", "/home/u/proj")`.
- `remove_worktree_inner_runs_git_for_an_in_subtree_path` and `remove_worktree_inner_rejects_an_escape_and_runs_no_git`: insert `"/home/u/proj",` as the new second argument.

In `src-tauri/app/src/lib.rs`, `GitCliWorktreeProvider::ensure`, keep today's behaviour for now (Task 3 changes it):

```rust
        workspace::worktree::add_worktree_inner(
            self.git.as_ref(),
            &self.project_root,
            &self.project_root,
            &path,
            &branch,
            "HEAD",
        )?;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
Expected: all pass (the five new tests included), no new warnings.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/workspace/src/worktree.rs src-tauri/app/src/lib.rs
git commit -m "refactor(workspace): run worktree git in the repo, guard on the project root"
```

---

### Task 2: `is_repo` on the git seam + `resolve_project_root`

**Files:**
- Modify: `src-tauri/workspace/src/worktree.rs` (`WorktreeGit` trait, `GitCli`, test `FakeGit`)
- Modify: `src-tauri/workspace/src/api.rs` (new `resolve_project_root`, tests)
- Modify: `src-tauri/app/src/lib.rs` (the `FakeGit` in `worktree_provider_tests`)

**Interfaces:**
- Consumes: Task 1's worktree functions (unchanged here).
- Produces:
  - `WorktreeGit::is_repo(&self, path: &str) -> bool`
  - `pub fn resolve_project_root(app_data: &Path, project_id: &str, root: Option<&str>, home: &str) -> PathBuf` in `workspace::api`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/workspace/src/worktree.rs` `tests` module:

```rust
    #[test]
    fn git_cli_is_repo_is_true_for_an_initialised_repo_and_false_for_a_plain_dir() {
        let base = std::env::temp_dir().join(format!("abp-isrepo-{}", uuid::Uuid::new_v4()));
        let repo = base.join("repo");
        let plain = base.join("plain");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        let init = std::process::Command::new("git").args(["-C", repo.to_str().unwrap(), "init", "-q"]).status().unwrap();
        assert!(init.success());
        assert!(GitCli.is_repo(repo.to_str().unwrap()));
        assert!(!GitCli.is_repo(plain.to_str().unwrap()));
        assert!(!GitCli.is_repo(base.join("missing").to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&base);
    }
```

In `src-tauri/workspace/src/api.rs`, add to its `#[cfg(test)] mod tests` (create one at the end of the file with `use super::*;` if none exists):

```rust
    #[test]
    fn resolve_project_root_defaults_to_the_app_data_project_dir() {
        let got = resolve_project_root(Path::new("/data"), "proj-1", None, "/home/u");
        assert_eq!(got, PathBuf::from("/data/projects/proj-1"));
    }

    #[test]
    fn resolve_project_root_treats_a_blank_root_as_none() {
        let got = resolve_project_root(Path::new("/data"), "proj-1", Some("   "), "/home/u");
        assert_eq!(got, PathBuf::from("/data/projects/proj-1"));
    }

    #[test]
    fn resolve_project_root_expands_a_tilde_root() {
        let got = resolve_project_root(Path::new("/data"), "proj-1", Some("~/work/x"), "/home/u");
        assert_eq!(got, PathBuf::from("/home/u/work/x"));
    }

    #[test]
    fn resolve_project_root_keeps_an_absolute_root() {
        let got = resolve_project_root(Path::new("/data"), "proj-1", Some("/srv/p"), "/home/u");
        assert_eq!(got, PathBuf::from("/srv/p"));
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p workspace`
Expected: compile errors `no method named is_repo found for struct GitCli` and `cannot find function resolve_project_root`.

- [ ] **Step 3: Implement**

Add to the `WorktreeGit` trait in `worktree.rs`, after `reset`:

```rust
    /// True when `path` is inside a git work tree (`git -C <path> rev-parse --git-dir`).
    fn is_repo(&self, path: &str) -> bool;
```

Add to `impl WorktreeGit for GitCli`:

```rust
    fn is_repo(&self, path: &str) -> bool {
        std::process::Command::new("git")
            .args(["-C", path, "rev-parse", "--git-dir"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
```

Add to the test `FakeGit` impl in `worktree.rs` and to the `FakeGit` impl in `src-tauri/app/src/lib.rs` `worktree_provider_tests`:

```rust
        fn is_repo(&self, _p: &str) -> bool { true }
```

Add to `src-tauri/workspace/src/api.rs`, next to `app_data_project_dir`:

```rust
/// Where a project's root lives: a non-blank `root` (tilde-expanded) when the
/// operator chose a custom location, otherwise the app-owned
/// `<app_data>/projects/<project_id>` dir. PURE.
pub fn resolve_project_root(app_data: &Path, project_id: &str, root: Option<&str>, home: &str) -> PathBuf {
    match root.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => PathBuf::from(expand_tilde(r, home)),
        None => app_data_project_dir(app_data, project_id),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/workspace/src/worktree.rs src-tauri/workspace/src/api.rs src-tauri/app/src/lib.rs
git commit -m "feat(workspace): is_repo on the git seam + resolve_project_root"
```

---

### Task 3: Worktrees are created in the target repo

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (`GitCliWorktreeProvider` doc comment + `ensure`, `worktree_provider_tests`)

**Interfaces:**
- Consumes: `add_worktree_inner(git, repo, project_root, path, branch, base_ref)` from Task 1; `WorktreeGit::is_repo` from Task 2 (fakes only).
- Produces: `GitCliWorktreeProvider::ensure(run_id, item_key, target_repo)` runs git in `target_repo`, or in the project root when `target_repo` is blank.

- [ ] **Step 1: Write the failing tests**

In `worktree_provider_tests`, change the existing test's repo assertion from `assert_eq!(calls[0].0, "/proj");` to:

```rust
        assert_eq!(calls[0].0, "/repo");
```

and add:

```rust
    #[test]
    fn git_cli_worktree_provider_falls_back_to_the_root_without_a_target() {
        use std::sync::Mutex;
        struct FakeGit { added: Mutex<Vec<(String, String, String, String)>> }
        impl workspace::worktree::WorktreeGit for FakeGit {
            fn list_porcelain(&self, _r: &str) -> Result<String, String> { Ok(String::new()) }
            fn remove(&self, _r: &str, _p: &str) -> Result<(), String> { Ok(()) }
            fn add(&self, repo: &str, path: &str, branch: &str, base: &str) -> Result<(), String> {
                self.added.lock().unwrap().push((repo.into(), path.into(), branch.into(), base.into()));
                Ok(())
            }
            fn reset(&self, _p: &str) -> Result<(), String> { Ok(()) }
            fn is_repo(&self, _p: &str) -> bool { true }
        }
        let git = std::sync::Arc::new(FakeGit { added: Mutex::new(vec![]) });
        let provider = GitCliWorktreeProvider::new(git.clone(), "/proj".into());
        let path = runtime::engine::WorktreeProvider::ensure(&provider, "R-1", "alpha", "  ").unwrap();
        assert_eq!(path, "/proj/worktrees/R-1/alpha");
        assert_eq!(git.added.lock().unwrap()[0].0, "/proj");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p app worktree_provider`
Expected: `git_cli_worktree_provider_ensure_derives_path_and_branch` FAILS with `left: "/proj"` / `right: "/repo"`; the fallback test passes (today's behaviour).

- [ ] **Step 3: Implement**

Replace the doc comment above `pub struct GitCliWorktreeProvider` with:

```rust
/// The app's git-backed worktree provider (worktree isolation). Implements the
/// git-unaware `runtime::engine::WorktreeProvider` over `workspace`'s
/// `WorktreeGit` seam. Lives at the composition root so `runtime` never learns
/// `git`. Worktree directories live under `<project_root>/worktrees/`; git runs
/// in the item's target repo so the branch comes off that repo's HEAD.
```

Replace `ensure`:

```rust
    fn ensure(&self, run_id: &str, item_key: &str, target_repo: &str) -> Result<String, String> {
        let path = self.worktree_path_for(run_id, item_key);
        // Idempotent: a worktree dir already present (resume / re-claim) is reused.
        if std::path::Path::new(&path).is_dir() {
            return Ok(path);
        }
        let branch = format!("agent-bus/{run_id}/{item_key}");
        // Git runs in the target repo; a blank target means the root is the repo.
        let repo = if target_repo.trim().is_empty() { self.project_root.as_str() } else { target_repo };
        workspace::worktree::add_worktree_inner(
            self.git.as_ref(),
            repo,
            &self.project_root,
            &path,
            &branch,
            "HEAD",
        )?;
        Ok(path)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): create worktrees in the item's target repo"
```

---

### Task 4: Project creation defaults the root to app data

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (`create_project_from_draft_inner`, `create_project_from_draft` command, the tests module containing `workspace_state` / `complete_draft`)

**Interfaces:**
- Consumes: `workspace::api::resolve_project_root` and `WorktreeGit::is_repo` (Task 2).
- Produces:
  - `pub async fn create_project_from_draft_inner(ws: &workspace::api::WorkspaceState, git: &dyn workspace::worktree::WorktreeGit, app_data: &std::path::Path, name: String, root: Option<String>, draft: DraftPipeline, target_repo: Option<String>) -> Result<Project, String>`
  - Tauri command `create_project_from_draft(name: String, root: Option<String>, draft, target_repo: Option<String>)`: the frontend sends `root: null` for app-managed.

- [ ] **Step 1: Add test helpers and update the existing call sites**

In the tests module (next to `complete_draft`), add:

```rust
    /// A git seam whose only job here is answering `is_repo`.
    struct RepoCheck(bool);
    impl workspace::worktree::WorktreeGit for RepoCheck {
        fn list_porcelain(&self, _r: &str) -> Result<String, String> { Ok(String::new()) }
        fn remove(&self, _r: &str, _p: &str) -> Result<(), String> { Ok(()) }
        fn add(&self, _r: &str, _p: &str, _b: &str, _base: &str) -> Result<(), String> { Ok(()) }
        fn reset(&self, _p: &str) -> Result<(), String> { Ok(()) }
        fn is_repo(&self, _p: &str) -> bool { self.0 }
    }

    fn temp_app_data() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("abp-appdata-{}", uuid::Uuid::new_v4()))
    }
```

Update the five existing calls (`create_from_an_invalid_draft_writes_nothing_and_errors`, `create_from_a_valid_draft_writes_files_and_activates`, `save_pipeline_edits_overwrites_yaml_and_prompts`, `save_pipeline_edits_rejects_an_invalid_draft_without_writing`, `pipeline_to_draft_inner_reads_prompt_bodies_back`) from

```rust
create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), <draft>, None)
```

to

```rust
create_project_from_draft_inner(&ws, &RepoCheck(true), &temp_app_data(), "Demo".into(), Some(root.to_string_lossy().into()), <draft>, None)
```

keeping each call's `<draft>` argument as it is.

- [ ] **Step 2: Write the failing tests**

```rust
    #[tokio::test]
    async fn create_without_a_root_lives_in_app_data() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let app_data = temp_app_data();
        let target = std::env::temp_dir().join("abp-target-repo").to_string_lossy().into_owned();
        let project = create_project_from_draft_inner(
            &ws, &RepoCheck(true), &app_data, "Demo".into(), None, complete_draft("demo"), Some(target.clone()),
        ).await.unwrap();
        let expected_root = app_data.join("projects").join(&project.id.0);
        assert_eq!(project.root_path, expected_root);
        assert!(expected_root.join("pipelines/demo.yaml").exists());
        assert_eq!(project.target_repo.as_deref(), Some(target.as_str()));
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[tokio::test]
    async fn create_without_a_root_or_target_errors_and_writes_nothing() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let app_data = temp_app_data();
        let err = create_project_from_draft_inner(
            &ws, &RepoCheck(true), &app_data, "Demo".into(), None, complete_draft("demo"), None,
        ).await.unwrap_err();
        assert_eq!(err, "a target repo is required when the project lives in app data");
        assert!(ws.store.list().await.unwrap().is_empty());
        assert!(!app_data.exists());
    }

    #[tokio::test]
    async fn create_treats_a_whitespace_target_as_missing() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let err = create_project_from_draft_inner(
            &ws, &RepoCheck(true), &temp_app_data(), "Demo".into(), None, complete_draft("demo"), Some("   ".into()),
        ).await.unwrap_err();
        assert_eq!(err, "a target repo is required when the project lives in app data");
        assert!(ws.store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_rejects_a_target_that_is_not_a_git_repo() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let err = create_project_from_draft_inner(
            &ws, &RepoCheck(false), &temp_app_data(), "Demo".into(), None, complete_draft("demo"), Some("/srv/not-a-repo".into()),
        ).await.unwrap_err();
        assert_eq!(err, "target repo is not a git repository: /srv/not-a-repo");
        assert!(ws.store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_rejects_a_relative_target() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let err = create_project_from_draft_inner(
            &ws, &RepoCheck(true), &temp_app_data(), "Demo".into(), None, complete_draft("demo"), Some("./app".into()),
        ).await.unwrap_err();
        assert_eq!(err, "target repo must be an absolute path: ./app");
        assert!(ws.store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_expands_a_tilde_target_before_checking_it() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let app_data = temp_app_data();
        let home = std::env::var("HOME").unwrap();
        let project = create_project_from_draft_inner(
            &ws, &RepoCheck(true), &app_data, "Demo".into(), None, complete_draft("demo"), Some("~/code/app".into()),
        ).await.unwrap();
        assert_eq!(project.target_repo, Some(format!("{home}/code/app")));
        let _ = std::fs::remove_dir_all(&app_data);
    }
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p app create_`
Expected: compile error `this function takes 5 arguments but 7 arguments were supplied`.

- [ ] **Step 4: Implement**

Replace `create_project_from_draft_inner` (keep the doc comment above it, then append one line to that doc comment: `/// A blank \`root\` puts the project in app data and requires a git-repo target.`):

```rust
pub async fn create_project_from_draft_inner(
    ws: &workspace::api::WorkspaceState,
    git: &dyn workspace::worktree::WorktreeGit,
    app_data: &std::path::Path,
    name: String,
    root: Option<String>,
    draft: DraftPipeline,
    target_repo: Option<String>,
) -> Result<Project, String> {
    // 1. HARD validate + serialize (shared gate; nothing is written when invalid).
    let (yaml_rel, yaml, prompts) = pipeline::draft::prepare_pipeline_write(&draft)?;
    let pipeline = draft.to_pipeline();

    // 2. Location: a custom root, or app data with a git-repo target.
    let home = std::env::var("HOME").unwrap_or_default();
    let custom_root = root.filter(|s| !s.trim().is_empty());
    let target = target_repo
        .filter(|s| !s.trim().is_empty())
        .map(|s| workspace::api::expand_tilde(s.trim(), &home));
    if custom_root.is_none() && target.is_none() {
        return Err("a target repo is required when the project lives in app data".into());
    }
    if let Some(t) = &target {
        if !std::path::Path::new(t).is_absolute() {
            return Err(format!("target repo must be an absolute path: {t}"));
        }
        if !git.is_repo(t) {
            return Err(format!("target repo is not a git repository: {t}"));
        }
    }

    // 3. Create the project row; the id exists before the root is resolved.
    let mut project = Project::new(name, std::path::PathBuf::new(), now_unix());
    project.root_path =
        workspace::api::resolve_project_root(app_data, &project.id.0, custom_root.as_deref(), &home);
    project.target_repo = target;
    ws.store.insert(&project).await.map_err(|e| e.to_string())?;

    // 4. Write the YAML + prompt files (Workspace owns the bytes-to-disk).
    workspace::api::write_project_pipeline_inner(
        ws, project.id.0.clone(), yaml_rel, yaml, prompts,
    )
    .await?;

    // 5. Activate.
    ws.store
        .set_active_pipeline(&project.id, Some(&agent_bus_core::PipelineId(pipeline.id.clone())), now_unix())
        .await
        .map_err(|e| e.to_string())?;

    // Return the project with the active pipeline reflected.
    ws.store.get(&project.id).await.map_err(|e| e.to_string())
}
```

Replace the command:

```rust
#[tauri::command(rename_all = "snake_case")]
async fn create_project_from_draft(
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
    activator: tauri::State<'_, Arc<pipeline_activator::PipelineActivator>>,
    name: String,
    root: Option<String>,
    draft: DraftPipeline,
    target_repo: Option<String>,
) -> Result<Project, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let project = create_project_from_draft_inner(
        &state, &workspace::worktree::GitCli, &app_data, name, root, draft, target_repo,
    )
    .await?;
    activator.activate(&project.id.0).await?;
    Ok(project)
}
```

(`tauri::Manager` is already imported at the top of `lib.rs`.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
Expected: all pass, including the six new `create_*` tests and the five updated ones.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): new projects live in app data and need a git-repo target"
```

---

### Task 5: Wizard asks for the target repo; root moves behind "Custom location"

**Files:**
- Modify: `src/ipc/pipeline.ts:232-239` (`createProjectFromDraft`)
- Modify: `src/wizard/NewProjectWizard.tsx` (state, `dirty`, `create`, Basics JSX, Generate/template guards, `ReviewStep` props)
- Modify: `src/wizard/ReviewStep.tsx` (props + location line)
- Test: `src/wizard/NewProjectWizard.test.tsx`, `src/wizard/ReviewStep.test.tsx`

**Interfaces:**
- Consumes: the command contract from Task 4 (`root: null` means app-managed).
- Produces:
  - `createProjectFromDraft(name: string, root: string | null, draft: DraftPipeline, targetRepo?: string | null)`
  - `ReviewStepProps.basics: { name: string; root: string; description: string; targetRepo: string }`, where an empty `root` means app-managed.

- [ ] **Step 1: Update the test helpers to the new Basics**

In `src/wizard/NewProjectWizard.test.tsx`, replace `openToReview` with:

```ts
// Open the wizard, fill basics, generate, and advance through to the review step.
// Default location is app-managed with a target repo of "/repo".
async function openToReview(opts: { targetRepo?: string; customRoot?: string } = {}) {
  kickoffMock.mockResolvedValueOnce(draftWithTeam());
  const onCreated = vi.fn();
  render(<NewProjectWizard open={true} onClose={() => {}} onCreated={onCreated} />);
  fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
  if (opts.customRoot !== undefined) {
    fireEvent.click(screen.getByRole("button", { name: /custom location/i }));
    fireEvent.change(screen.getByRole("textbox", { name: "Root path" }), { target: { value: opts.customRoot } });
  }
  const target = opts.targetRepo ?? (opts.customRoot === undefined ? "/repo" : undefined);
  if (target !== undefined) {
    fireEvent.change(screen.getByRole("textbox", { name: /target repo/i }), { target: { value: target } });
  }
  fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "a research flow" } });
  fireEvent.click(screen.getByRole("button", { name: /generate/i }));
  // generate advances to the Canvas step (its palette is the tell)
  await screen.findByRole("button", { name: /add team/i });
  fireEvent.click(screen.getByRole("button", { name: /continue/i })); // canvas -> review
  return { onCreated };
}
```

In the five tests that fill the root directly (lines 67, 85, 182, 208, 235 today), replace each occurrence of

```ts
fireEvent.change(screen.getByRole("textbox", { name: "Root path" }), { target: { value: "/p" } });
```

with

```ts
fireEvent.change(screen.getByRole("textbox", { name: /target repo/i }), { target: { value: "/repo" } });
```

Replace the two existing Create tests:

```ts
  it("footer Create sends root null + the target repo for an app-managed project", async () => {
    const created = { id: "proj-x", name: "Demo", root_path: "/data/projects/proj-x", target_repo: "/repo", active_pipeline_id: "demo", created_at: 0, updated_at: 0 };
    createMock.mockResolvedValueOnce(created);
    const { onCreated } = await openToReview();
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));
    await waitFor(() => expect(createMock).toHaveBeenCalledWith("Demo", null, expect.objectContaining({ name: "Demo" }), "/repo"));
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(created));
  });

  it("footer Create passes a custom root through with an optional target (A5)", async () => {
    const created = { id: "proj-x", name: "Demo", root_path: "/p", target_repo: null, active_pipeline_id: "demo", created_at: 0, updated_at: 0 };
    createMock.mockResolvedValueOnce(created);
    await openToReview({ customRoot: "/p" });
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));
    await waitFor(() => expect(createMock).toHaveBeenCalledWith("Demo", "/p", expect.objectContaining({ name: "Demo" }), null));
  });
```

In `src/wizard/ReviewStep.test.tsx`, change the three `basics={{ name: "Demo", root: "/p", description: "" }}` props to `basics={{ name: "Demo", root: "/p", description: "", targetRepo: "" }}`.

- [ ] **Step 2: Write the failing tests**

Add to `NewProjectWizard.test.tsx` inside `describe("NewProjectWizard", …)`:

```ts
  it("Generate needs a target repo by default and shows no root picker", () => {
    render(<NewProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
    fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "x" } });
    expect(screen.queryByRole("textbox", { name: "Root path" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /generate/i })).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox", { name: /target repo/i }), { target: { value: "/repo" } });
    expect(screen.getByRole("button", { name: /generate/i })).toBeEnabled();
  });

  it("Custom location reveals the root picker and makes the target optional", () => {
    render(<NewProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
    fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: /custom location/i }));
    fireEvent.change(screen.getByRole("textbox", { name: "Root path" }), { target: { value: "/p" } });
    expect(screen.getByRole("button", { name: /generate/i })).toBeEnabled();
  });

  it("closing Custom location ignores the typed root and needs a target again", async () => {
    createMock.mockResolvedValueOnce({ id: "proj-x", name: "Demo", root_path: "/data/projects/proj-x", target_repo: "/repo", active_pipeline_id: "demo", created_at: 0, updated_at: 0 });
    kickoffMock.mockResolvedValueOnce(draftWithTeam());
    render(<NewProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
    fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: /custom location/i }));
    fireEvent.change(screen.getByRole("textbox", { name: "Root path" }), { target: { value: "/p" } });
    fireEvent.click(screen.getByRole("button", { name: /custom location/i }));
    expect(screen.getByRole("button", { name: /generate/i })).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox", { name: /target repo/i }), { target: { value: "/repo" } });
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));
    await screen.findByRole("button", { name: /add team/i });
    fireEvent.click(screen.getByRole("button", { name: /continue/i }));
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));
    await waitFor(() => expect(createMock).toHaveBeenCalledWith("Demo", null, expect.anything(), "/repo"));
  });

  it("typing only a target repo makes the draft dirty (Cancel confirms)", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const onClose = vi.fn();
    render(<NewProjectWizard open={true} onClose={onClose} onCreated={() => {}} />);
    fireEvent.change(screen.getByRole("textbox", { name: /target repo/i }), { target: { value: "/repo" } });
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(confirm).toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    confirm.mockRestore();
  });
```

Add to `ReviewStep.test.tsx`:

```ts
  it("shows the app-managed location and the target repo", () => {
    render(<ReviewStep basics={{ name: "Demo", root: "", description: "", targetRepo: "/repo" }} draft={draft()} />);
    expect(screen.getByText(/location: app-managed/i)).toBeInTheDocument();
    expect(screen.getByText(/target repo: \/repo/i)).toBeInTheDocument();
  });
```

- [ ] **Step 3: Run them to verify they fail**

Run: `npx vitest run src/wizard/NewProjectWizard.test.tsx src/wizard/ReviewStep.test.tsx`
Expected: the new tests FAIL. Examples: `Unable to find an accessible element with the role "button" and name /custom location/i`, `expected "Demo", "/p", …` vs `null`, and `Unable to find an element with the text: /location: app-managed/i`. The cancel test fails if `dirty` ignores the target.

- [ ] **Step 4: Implement**

`src/ipc/pipeline.ts`, change the `root` parameter type only:

```ts
export async function createProjectFromDraft(
  name: string,
  root: string | null,
  draft: DraftPipeline,
  targetRepo?: string | null,
): Promise<{ id: string; name: string; root_path: string; target_repo: string | null; active_pipeline_id: string | null; created_at: number; updated_at: number }> {
  return await invoke("create_project_from_draft", { name, root, draft, target_repo: targetRepo ?? null });
}
```

`src/wizard/NewProjectWizard.tsx`, after `const [targetRepo, setTargetRepo] = useState("");` add:

```ts
  const [customLocation, setCustomLocation] = useState(false);
  // App-managed projects need a target repo; a custom root makes it optional.
  const effectiveRoot = customLocation && root.trim() ? root.trim() : null;
  const locationReady = customLocation ? effectiveRoot !== null : targetRepo.trim() !== "";
```

Change `dirty` to:

```ts
  const dirty =
    step !== "basics" || name.trim() !== "" || root.trim() !== "" || targetRepo.trim() !== "" || description.trim() !== "";
```

In `create()`, change the call to:

```ts
      const project = await createProjectFromDraft(name, effectiveRoot, { ...draft, name, description }, targetRepo.trim() || null);
```

In the Basics JSX, replace the two `FolderPickerField` lines (Root path, Target repo) with:

```tsx
          <div style={lbl}>
            <FolderPickerField
              label={customLocation ? "Target repo (optional)" : "Target repo"}
              value={targetRepo}
              onChange={setTargetRepo}
              placeholder="~/projects/your-repo"
            />
          </div>
          <div style={lbl}>
            <Button aria-expanded={customLocation} onClick={() => setCustomLocation((v) => !v)}>
              {customLocation ? "▾" : "▸"} Custom location
            </Button>
            {!customLocation && (
              <div style={{ fontSize: "var(--ts-sm)", color: "var(--text-3)", marginTop: "var(--sp-1)" }}>
                The project lives in app data; the repo only gains agent-bus/… branches.
              </div>
            )}
            {customLocation && (
              <FolderPickerField label="Root path" value={root} onChange={setRoot} placeholder="~/projects/example" />
            )}
          </div>
```

Change the Generate button's `disabled` to `busy || !name.trim() || !locationReady || !description.trim()` and each template button's `disabled` to `busy || !name.trim() || !locationReady`.

Change the review render to:

```tsx
          <ReviewStep basics={{ name, root: effectiveRoot ?? "", description, targetRepo }} draft={draft} error={error} />
```

`src/wizard/ReviewStep.tsx`: change the props type to `basics: { name: string; root: string; description: string; targetRepo: string };` and add, as the first child of the returned `<div>`:

```tsx
      <div style={{ fontSize: 12, color: "var(--text-2)", marginBottom: "var(--sp-2)" }}>
        Location: {basics.root.trim() || "app-managed"} · Target repo: {basics.targetRepo.trim() || "none"}
      </div>
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `npx vitest run && npx tsc --noEmit`
Expected: all test files pass; `tsc` prints nothing.

- [ ] **Step 6: Commit**

```bash
git add src/ipc/pipeline.ts src/wizard/NewProjectWizard.tsx src/wizard/NewProjectWizard.test.tsx src/wizard/ReviewStep.tsx src/wizard/ReviewStep.test.tsx
git commit -m "feat(wizard): ask for the target repo; root moves behind Custom location"
```

---

### Task 6: Whole-branch verification

**Files:** none changed unless a check fails.

- [ ] **Step 1: Full suites**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace && npx vitest run && npx tsc --noEmit`
Expected: every Rust and frontend test passes; no type errors.

- [ ] **Step 2: Record what stays unverified**

The automated tests prove the wiring through fakes, plus one real-git `is_repo` test. They do not drive the real app. The end-to-end check belongs to the watched spike run, not to this plan: create a meal-planner project in the app, run one implementer item, then confirm `git -C <meal-planner> status --porcelain` is empty and an `agent-bus/<run>/<item>` branch exists. Note this in the branch's final summary.
