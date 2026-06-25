# Per-Work-Item Worktree Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give each work-item that reaches an implementer stage its own isolated git worktree (committed, never pushed) so implementers never mutate the target repo and disparate candidates never collide, with reset-on-resume hygiene for re-queued tasks.

**Architecture:** A new `Role::Implementer` marks repo-mutating stages. The engine resolves a work-item's `working_dir` through an injected, git-unaware `WorktreeProvider` seam (mirroring the `Runner` seam): `task.worktree_path` if inherited → use it; else `Implementer` role → `provider.ensure(...)` + persist; else the target repo (chunk-1 behavior). The app implements `GitCliWorktreeProvider` over an extended `workspace::worktree::WorktreeGit` (`add`/`reset`, path-scoped under `<root>/worktrees/`). Children copy the parent's `worktree_path` at every build site so an item's code-review shares the implement tree. Reset-on-resume is wired at the composition root over the re-queued implementer tasks.

**Tech Stack:** Rust (Cargo workspace under `src-tauri/`), Tauri 2, sqlx + SQLite migrations, serde, schemars; TypeScript (vitest/tsc) only for the `Task` IPC mirror.

---

## Authoritative references (read before starting)

- Spec: `docs/superpowers/specs/2026-06-25-worktree-isolation-design.md` — implement ALL of it.
- Roadmap: `docs/roadmap-remaining.md` §Worktree isolation (WT1+WT2) and §Lifecycle hardening (LH8).

## Confirmed real signatures (verified against current HEAD, tag `plan-lifecycle-hardening` lands FIRST)

- `pipeline/src/model.rs:242` — `Role` derive set: `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema)]` + `#[serde(rename_all = "lowercase")]`, variants `Producer` (default) / `Reviewer`. There is NO `SCHEMA_VERSION` bump (currently `3`).
- `runtime/src/engine.rs:249` — `fn role_str(team: &Team) -> &'static str` is a NON-exhaustive `match team.role { Reviewer => "reviewer", Producer => "producer" }` — adding `Implementer` forces a compile error here (good).
- `runtime/src/engine.rs:130` — `pub struct EngineContext` fields end with `audit`. It derives `Clone`.
- `runtime/src/engine.rs:1242` — `async fn invoke(ctx: &EngineContext, team: &Team, task: &Task, system_prompt: String) -> Result<Vec<OutputItem>, EngineError>`. `task` is borrowed `&Task` (immutable). The `working_dir` is computed at `engine.rs:1256-1266` from `effective_repo` (an `Option<PathBuf>` via `effective_target_repo(task.target_repo.as_deref(), ctx.target_repo.as_deref())`).
- `runtime/src/engine.rs` — `Task::work_item(project_id, pipeline, run_id, item_key, stage, parent_artifact, target_repo, now_unix)` is called at the child-build sites: lines **416** (transform success), **498** (gate approve), **535** (gate revise), **793** (join downstream), **824** (join revise), **844** (join escalate), **936** (generator pass task), **988** (generator child). `Task::forked(&task, lane, group_id, join, now_unix)` at **663**.
- `runtime/src/task.rs:60` — `pub struct Task` ends with `run_id: Option<String>` / `item_key: Option<String>` (both `#[serde(default)]`). Constructors: `injected`, `work_item`, `forked`.
- `runtime/src/task_store.rs` — `struct Row` (sqlx `FromRow`, 18 cols), `insert` (18-col INSERT), `row_to_task`, `const SELECT` (18 cols), `update` (11-col UPDATE), `release_orphaned_running(now_unix) -> Result<u64, _>` returns a COUNT only (must change to return the re-queued rows for WT2). Test harness `fresh_pool()` applies migrations 001, 003, 006, 012 — migration 014 must be added there.
- `workspace/src/worktree.rs:128` — `trait WorktreeGit { list_porcelain; remove }`, `struct GitCli`, `fn worktree_under_root(project_root, path) -> Result<PathBuf, String>`, and a test-only `FakeGit` (records `removed`). `WorktreeState { store, git: Arc<dyn WorktreeGit> }`.
- `pipeline/src/seed_template.rs:153` — the `implementers` team is built via `producer("implementers", ...)` (sets `role = Role::Producer`); its routes are patched in the `for t in d.teams.iter_mut()` loop at line 189.
- `app/src/lib.rs:39` — `run_migrations` `const MIGRATIONS: &[(i64, &str)]` (12 entries today; lifecycle-hardening adds 13). `app/src/lib.rs:1182` — the tauri-plugin-sql `migrations` vec (mirror list). `app/src/lib.rs:1873` — idempotency test asserts `user_version == 12` (will be 14 after this + lifecycle-hardening's 13; see RECONCILE note).
- `app/src/lib.rs:1300` — `WorktreeState { store, git: Arc::new(GitCli) }` is managed; `app/src/lib.rs:1337` — `tasks.release_orphaned_running(now_unix())` at boot; `app/src/lib.rs:1353` — `stores.reconcile_occupancy` at boot (the WT2 wiring site).
- `app/src/pipeline_activator.rs:361` — `fn ctx_builder(...)` clones `self.deps.*` and builds the FULL `EngineContext` at line **386** (gets the real provider). `runtime/src/api.rs:170` — `engine_ctx_for_run` builds the gate-verdict `EngineContext` at line **176** (provider = `None`).
- TS: `src/ipc/runtime.ts:14` — `export interface Task` lists columns with optional `run_id?`/`item_key?`. Add `worktree_path?: string | null` (additive-optional). `src/wizard/draft.ts:80` — `setTeamRole(..., role: "producer" | "reviewer")`; `Implementer` is seed/backend-only and is NEVER author-selectable, so this union is intentionally left unchanged (documented in Task 1).

## RECONCILE NOTES (lifecycle-hardening lands first)

- **Migration number / user_version:** This plan uses migration **014** and bumps the idempotency assertion to **14** assuming lifecycle-hardening already added **013** and bumped the assertion to **13**. When executing: re-confirm the highest existing migration file under `app/migrations/` and the current `user_version` assertion in `app/src/lib.rs`. If lifecycle-hardening did NOT land yet, use the next free number and the matching version, but keep the file basename `0NN_task_worktree.sql`.
- **WT2 recovery path (Task 9):** Lifecycle-hardening LH8 makes a killed task re-queue NON-terminally and reworks brake-off/crash recovery. This plan wires `worktree_provider.reset(path)` over the tasks that the recovery path re-queues. The IMPLEMENTER MUST RECONCILE: locate the post-lifecycle-hardening recovery path (boot `release_orphaned_running` + brake-off/crash re-queue) and call `reset` for each re-queued task that carries a `worktree_path`, AT THE COMPOSITION ROOT (the root holds the provider; `runtime` stays git-unaware — it only reports which tasks were re-queued + their `worktree_path`). The signature change in Task 8 (`release_orphaned_running` returns the re-queued `Task` rows) is the seam; if lifecycle-hardening already changed that signature, adapt to its return shape instead of re-changing it.

## File Structure

- `src-tauri/pipeline/src/model.rs` — add `Role::Implementer`.
- `src-tauri/pipeline/src/seed_template.rs` — tag the `implementers` team `Role::Implementer`.
- `src-tauri/workspace/src/worktree.rs` — `WorktreeGit::add` + `reset` on the trait, `GitCli`, and `FakeGit`; path-scoped.
- `src-tauri/runtime/src/engine.rs` — `WorktreeProvider` trait; `EngineContext.worktree_provider`; `role_str` Implementer arm; `working_dir` resolution in `invoke`; copy `worktree_path` to children at every build site.
- `src-tauri/runtime/src/task.rs` — `Task.worktree_path` field + constructor wiring.
- `src-tauri/runtime/src/task_store.rs` — `worktree_path` column read/write, `set_worktree_path`, and `release_orphaned_running` returning re-queued rows.
- `src-tauri/app/migrations/014_task_worktree.sql` — new column.
- `src-tauri/app/src/lib.rs` — register migration 014 in BOTH lists; bump idempotency test; `GitCliWorktreeProvider`; inject the provider into the activator deps; wire reset-on-resume.
- `src-tauri/app/src/pipeline_activator.rs` — thread the provider through `ctx_builder`/`deps`.
- `src-tauri/runtime/src/api.rs` — set `worktree_provider: None` in the gate-verdict context.
- `src/ipc/runtime.ts` — add `worktree_path?` to the `Task` interface.

---

## Task 1: Add `Role::Implementer` to the pipeline model

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs:244-248` (the `Role` enum) and its tests block.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` mod in `src-tauri/pipeline/src/model.rs` (after `role_serializes_lowercase_and_round_trips`):

```rust
    #[test]
    fn implementer_role_serializes_lowercase_and_round_trips() {
        assert_eq!(serde_json::to_string(&Role::Implementer).unwrap(), "\"implementer\"");
        let r: Role = serde_json::from_str("\"implementer\"").unwrap();
        assert_eq!(r, Role::Implementer);
    }

    #[test]
    fn role_default_is_still_producer_after_adding_implementer() {
        assert_eq!(Role::default(), Role::Producer);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p pipeline implementer_role_serializes_lowercase_and_round_trips`
Expected: FAIL — `no variant named Implementer found for enum Role`.

- [ ] **Step 3: Add the variant**

In `src-tauri/pipeline/src/model.rs`, change the enum body (keep the existing derive + `#[serde(rename_all = "lowercase")]`):

```rust
pub enum Role {
    #[default]
    Producer,
    Reviewer,
    /// A repo-mutating stage (vet F8 / worktree isolation). Treated like a
    /// producer for routing (no verdict); marks the stage that creates/enters a
    /// per-work-item git worktree. Additive; default stays `producer`; no
    /// SCHEMA_VERSION bump. NOTE: seed/backend-only — never offered in the
    /// authoring wizard (the TS `setTeamRole` union stays producer|reviewer).
    Implementer,
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p pipeline`
Expected: PASS (the existing `role_str` in `runtime` does not compile yet — that is Task 5; `pipeline` builds standalone here).

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add pipeline/src/model.rs
git commit -m "feat(pipeline): add Role::Implementer (additive, no schema bump)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Tag the seed `implementers` team `Role::Implementer`

**Files:**
- Modify: `src-tauri/pipeline/src/seed_template.rs:189-195` (the route-patch loop) and the tests block.

- [ ] **Step 1: Write the failing test**

Add to the `tests` mod in `src-tauri/pipeline/src/seed_template.rs`:

```rust
    #[test]
    fn ddd_seed_implementers_team_has_implementer_role() {
        let d = ddd_seed();
        let imp = d.teams.iter().find(|t| t.id == "implementers").expect("implementers team exists");
        assert_eq!(imp.role, Role::Implementer, "implementers must be tagged Implementer");
        // Regression: producers/reviewers around it keep their roles.
        let research = d.teams.iter().find(|t| t.id == "research").unwrap();
        assert_eq!(research.role, Role::Producer);
        let cr = d.teams.iter().find(|t| t.id == "code-reviewers").unwrap();
        assert_eq!(cr.role, Role::Reviewer);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline ddd_seed_implementers_team_has_implementer_role`
Expected: FAIL — `assertion failed: ... Producer == Implementer`.

- [ ] **Step 3: Set the role in the route-patch loop**

In `src-tauri/pipeline/src/seed_template.rs`, change the `"implementers"` arm of the `for t in d.teams.iter_mut()` loop (currently around line 189) to also set the role:

```rust
            "implementers" => {
                t.role = Role::Implementer;
                t.outputs.on_revise = Some("plan-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p pipeline`
Expected: PASS (including the existing `ddd_seed_*` validation tests — `Implementer` routes like a producer so hard validation still holds).

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add pipeline/src/seed_template.rs
git commit -m "feat(pipeline): tag the seed implementers team Role::Implementer

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Add `WorktreeGit::add` + `reset` (trait + GitCli + FakeGit), path-scoped

**Files:**
- Modify: `src-tauri/workspace/src/worktree.rs` (trait at :128, `GitCli` impl at :139, tests `FakeGit` at :434).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` mod in `src-tauri/workspace/src/worktree.rs`. First extend `FakeGit` to record adds/resets (replace its struct + impl with the version below — it keeps the existing `removed`/`fail_remove` fields so existing tests still pass):

```rust
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
```

Then add these tests (they exercise the path-scoped wrappers `add_worktree_inner` / `reset_worktree_inner` added in Step 3):

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p workspace add_worktree_inner_runs_git_for_an_in_subtree_path`
Expected: FAIL — `cannot find function add_worktree_inner` / `no method named add` on the trait.

- [ ] **Step 3: Add the trait methods, GitCli impls, and path-scoped wrappers**

In `src-tauri/workspace/src/worktree.rs`, extend the `WorktreeGit` trait (after `remove`):

```rust
    /// `git -C <repo> worktree add -b <branch> <path> <base_ref>`.
    fn add(&self, repo: &str, path: &str, branch: &str, base_ref: &str) -> Result<(), String>;
    /// Reset a worktree to its branch baseline:
    /// `git -C <worktree_path> reset --hard @{upstream}` is NOT used (no remote);
    /// instead reset to HEAD and clean untracked:
    /// `git -C <worktree_path> reset --hard && git -C <worktree_path> clean -fd`.
    fn reset(&self, worktree_path: &str) -> Result<(), String>;
```

Add to `impl WorktreeGit for GitCli` (after `remove`):

```rust
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
```

Add the path-scoped wrappers next to `remove_worktree_inner` (after it, ~line 189):

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p workspace`
Expected: PASS (all existing worktree tests + the 4 new ones).

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add workspace/src/worktree.rs
git commit -m "feat(workspace): WorktreeGit::add + reset, path-scoped under worktrees/

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Add `Task.worktree_path` (field + constructors)

**Files:**
- Modify: `src-tauri/runtime/src/task.rs` (struct at :60, `injected` :103, `work_item` :139, `forked` :175, tests).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` mod in `src-tauri/runtime/src/task.rs`:

```rust
    #[test]
    fn injected_and_work_item_have_no_worktree_path() {
        let t = t();
        assert_eq!(t.worktree_path, None);
        let wi = Task::work_item("p".into(), "pl".into(), "r".into(), "k".into(), "stage".into(), None, None, 100);
        assert_eq!(wi.worktree_path, None);
    }

    #[test]
    fn forked_sibling_inherits_worktree_path() {
        let mut parent = t();
        parent.worktree_path = Some("/p/worktrees/R-1/alpha".into());
        let sib = Task::forked(&parent, "lane-a", "G-1", "join-1", 500);
        assert_eq!(sib.worktree_path.as_deref(), Some("/p/worktrees/R-1/alpha"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime injected_and_work_item_have_no_worktree_path`
Expected: FAIL — `no field worktree_path on type Task`.

- [ ] **Step 3: Add the field + wire constructors**

In `src-tauri/runtime/src/task.rs`, add to the `Task` struct (after `item_key`):

```rust
    /// The per-work-item git worktree this item runs in (worktree isolation /
    /// WT1). Set when the item reaches its first Implementer-role stage; copied
    /// onto children so downstream stages (code-review, revise→implement) share
    /// the same tree. `None` for read-only stages and legacy tasks.
    #[serde(default)]
    pub worktree_path: Option<String>,
```

Set `worktree_path: None` in the `injected` constructor (after `item_key: None,`) and in `work_item` (after `item_key: Some(item_key),`). In `forked`, INHERIT it (after `item_key: parent.item_key.clone(),`):

```rust
            worktree_path: parent.worktree_path.clone(),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime task::tests`
Expected: PASS. (The crate will not fully compile until `task_store.rs` reads/writes the field — Task 6 — but `task::tests` and the field exist now. If the crate fails to build here due to `row_to_task` missing the field, proceed to Task 6 before running; alternatively reorder Task 6 before Task 4. Implementer note: it is cleaner to do Task 4 + Task 6 + the migration Task 7 as one compile unit; run cargo test after Task 7.)

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add runtime/src/task.rs
git commit -m "feat(runtime): Task.worktree_path field; forked inherits it

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Define the `WorktreeProvider` seam + `EngineContext.worktree_provider`; fix `role_str`

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` (top-of-file trait, `EngineContext` at :130, `role_str` at :249, the test `ctx` builders).

- [ ] **Step 1: Write the failing test**

Add to the `tests` mod in `src-tauri/runtime/src/engine.rs` (near the other unit tests, e.g. after `effective_target_repo` tests). This is a fake-provider sanity test; it does not yet exercise `invoke` resolution (that is Task 8):

```rust
    struct RecordingProvider {
        ensure_calls: std::sync::Mutex<Vec<(String, String, String)>>,
        reset_calls: std::sync::Mutex<Vec<String>>,
        returns: String,
    }
    impl crate::engine::WorktreeProvider for RecordingProvider {
        fn ensure(&self, run_id: &str, item_key: &str, target_repo: &str) -> Result<String, String> {
            self.ensure_calls.lock().unwrap().push((run_id.into(), item_key.into(), target_repo.into()));
            Ok(self.returns.clone())
        }
        fn reset(&self, worktree_path: &str) -> Result<(), String> {
            self.reset_calls.lock().unwrap().push(worktree_path.into());
            Ok(())
        }
    }

    #[test]
    fn worktree_provider_records_ensure_and_reset() {
        let p = RecordingProvider {
            ensure_calls: Default::default(),
            reset_calls: Default::default(),
            returns: "/proj/worktrees/R-1/alpha".into(),
        };
        let got = WorktreeProvider::ensure(&p, "R-1", "alpha", "/repo").unwrap();
        assert_eq!(got, "/proj/worktrees/R-1/alpha");
        WorktreeProvider::reset(&p, "/proj/worktrees/R-1/alpha").unwrap();
        assert_eq!(p.ensure_calls.lock().unwrap()[0], ("R-1".into(), "alpha".into(), "/repo".into()));
        assert_eq!(p.reset_calls.lock().unwrap()[0], "/proj/worktrees/R-1/alpha");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p runtime worktree_provider_records_ensure_and_reset`
Expected: FAIL — `cannot find trait WorktreeProvider`.

- [ ] **Step 3: Define the trait, add the field, and fix `role_str`**

In `src-tauri/runtime/src/engine.rs`, add the trait near the top (after the `EngineError` enum, before `EngineContext`):

```rust
/// The git-unaware worktree seam (worktree isolation). The engine resolves an
/// implementer's working dir through this; the app implements it over
/// `workspace::worktree::GitCli`; tests inject a fake. Mirrors the `Runner`
/// injection — no `git` idiom crosses into `runtime`.
pub trait WorktreeProvider: Send + Sync {
    /// Idempotent: ensure a worktree for (run_id, item_key) off `target_repo`'s
    /// current HEAD exists; return its absolute path. Branch
    /// `agent-bus/<run_id>/<item_key>`.
    fn ensure(&self, run_id: &str, item_key: &str, target_repo: &str) -> Result<String, String>;
    /// Reset a worktree to its branch baseline (discard a killed mid-write tree).
    fn reset(&self, worktree_path: &str) -> Result<(), String>;
}
```

Add the field to `EngineContext` (after `audit`):

```rust
    /// The injected worktree seam (worktree isolation). `None` in pure runtime
    /// tests and topic-less runs ⇒ implementer stages fall back to the target-repo
    /// `working_dir`, preserving chunk-1 behavior.
    pub worktree_provider: Option<std::sync::Arc<dyn WorktreeProvider>>,
```

Fix `role_str` (line ~249) to handle the new variant (Implementer is a producer for the output contract):

```rust
fn role_str(team: &Team) -> &'static str {
    match team.role {
        pipeline::model::Role::Reviewer => "reviewer",
        pipeline::model::Role::Producer | pipeline::model::Role::Implementer => "producer",
    }
}
```

- [ ] **Step 4: Make the test `EngineContext` builders set the new field**

Find every `EngineContext { ... }` literal inside `engine.rs` tests (search `EngineContext {`). Add `worktree_provider: None,` to each. Do the same for any `..` -free literal. (The two production builders are fixed in Tasks 9/10.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS (engine tests compile with the new field defaulted to `None`; existing behavior unchanged).

- [ ] **Step 6: Commit**

```bash
cd src-tauri && git add runtime/src/engine.rs
git commit -m "feat(runtime): WorktreeProvider seam + EngineContext.worktree_provider; role_str handles Implementer

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Persist `Task.worktree_path` in TaskStore (read/write + `set_worktree_path`)

**Files:**
- Modify: `src-tauri/runtime/src/task_store.rs` (`Row` :23, `insert` :54, `row_to_task` :84, `SELECT` :108, `update` :136, add `set_worktree_path`, tests + `fresh_pool`).

> NOTE: this task depends on migration 014 (Task 7) for the live column. Apply Task 7's migration to `fresh_pool()` here so the round-trip tests pass. (Reorder: do Task 7 before running Task 6's tests, or include both in one commit. The steps below add 014 to `fresh_pool`.)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` mod in `src-tauri/runtime/src/task_store.rs`. First, add migration 014 to `fresh_pool()` (after the `012_runtime_stores.sql` line):

```rust
        sqlx::query(include_str!("../../app/migrations/014_task_worktree.sql")).execute(&pool).await.unwrap();
```

Then the tests:

```rust
    #[tokio::test]
    async fn insert_get_round_trips_worktree_path() {
        let store = TaskStore::new(fresh_pool().await);
        let mut task = Task::work_item("p".into(), "pl".into(), "R-1".into(), "alpha".into(), "implementers".into(), None, None, 100);
        task.worktree_path = Some("/p/worktrees/R-1/alpha".into());
        store.insert(&task).await.unwrap();
        let back = store.get(&task.id).await.unwrap();
        assert_eq!(back.worktree_path.as_deref(), Some("/p/worktrees/R-1/alpha"));
    }

    #[tokio::test]
    async fn set_worktree_path_persists_single_column() {
        let store = TaskStore::new(fresh_pool().await);
        let task = Task::work_item("p".into(), "pl".into(), "R-1".into(), "alpha".into(), "implementers".into(), None, None, 100);
        store.insert(&task).await.unwrap();
        store.set_worktree_path(&task.id.0, "/p/worktrees/R-1/alpha").await.unwrap();
        let back = store.get(&task.id).await.unwrap();
        assert_eq!(back.worktree_path.as_deref(), Some("/p/worktrees/R-1/alpha"));
    }

    #[tokio::test]
    async fn legacy_task_loads_with_none_worktree_path() {
        let store = TaskStore::new(fresh_pool().await);
        let task = Task::injected("p".into(), "pl".into(), "research".into(), "topic".into(), None, 100);
        store.insert(&task).await.unwrap();
        let back = store.get(&task.id).await.unwrap();
        assert_eq!(back.worktree_path, None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime set_worktree_path_persists_single_column`
Expected: FAIL — `no field worktree_path on struct Row` / `no method set_worktree_path` (and `014_task_worktree.sql` missing — create it in Task 7 first if not present).

- [ ] **Step 3: Wire the column through the store**

In `src-tauri/runtime/src/task_store.rs`:

Add to `struct Row` (after `item_key: Option<String>,`):

```rust
    worktree_path: Option<String>,
```

`insert` — add `worktree_path` to the column list, add one `?` to VALUES (now 19 placeholders), and add the bind (after `.bind(&task.item_key)`):

```rust
            "INSERT INTO tasks (id, project_id, pipeline, topic, target_repo, target_scope,
             current_stage, state, attempts, parent_artifact, review_artifact, created_at, updated_at,
             group_id, lane, join_target, run_id, item_key, worktree_path)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
```
```rust
        .bind(&task.worktree_path)
```

`row_to_task` — add (after `item_key: r.item_key,`):

```rust
            worktree_path: r.worktree_path,
```

`SELECT` — append `worktree_path` to the column list:

```rust
    const SELECT: &'static str =
        "SELECT id, project_id, pipeline, topic, target_repo, target_scope, current_stage,
         state, attempts, parent_artifact, review_artifact, created_at, updated_at,
         group_id, lane, join_target, run_id, item_key, worktree_path FROM tasks";
```

`update` — add `worktree_path=?` to the SET list and bind it (place the bind alongside `item_key`, BEFORE the trailing `.bind(&task.id.0)` for the WHERE):

```rust
            "UPDATE tasks SET current_stage=?, state=?, attempts=?, parent_artifact=?,
             review_artifact=?, updated_at=?, group_id=?, lane=?, join_target=?,
             run_id=?, item_key=?, worktree_path=? WHERE id=?",
```
```rust
        .bind(&task.item_key)
        .bind(&task.worktree_path)
        .bind(&task.id.0)
```

Add the single-column setter (after `update`):

```rust
    /// Persist just the work-item's resolved worktree path (worktree isolation).
    /// Single-column UPDATE so it does not race the broader `update`.
    pub async fn set_worktree_path(&self, task_id: &str, path: &str) -> Result<(), TaskStoreError> {
        let res = sqlx::query("UPDATE tasks SET worktree_path=? WHERE id=?")
            .bind(path)
            .bind(task_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(TaskStoreError::NotFound(TaskId(task_id.to_string())));
        }
        Ok(())
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime task_store`
Expected: PASS (requires `014_task_worktree.sql` from Task 7 to exist).

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add runtime/src/task_store.rs
git commit -m "feat(runtime): persist Task.worktree_path; add set_worktree_path

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Migration 014 — `tasks.worktree_path` + register in both lists + bump idempotency test

**Files:**
- Create: `src-tauri/app/migrations/014_task_worktree.sql`
- Modify: `src-tauri/app/src/lib.rs` (`MIGRATIONS` :39, tauri-plugin-sql `migrations` vec :1182, idempotency test :1846-1877).

> RECONCILE: confirm 013 exists (lifecycle-hardening). If the next free number is not 014, use it consistently across the file and migration name, and set `user_version` to match.

- [ ] **Step 1: Write the failing test**

Add to the migration idempotency test in `src-tauri/app/src/lib.rs` (after the `item_key_cols` assertion, before the `version` assertion), and CHANGE the `version` assertion to `14`:

```rust
        let worktree_path_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('tasks') WHERE name='worktree_path'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(worktree_path_cols, 1, "migration 014 tasks.worktree_path present exactly once");
```
```rust
        assert_eq!(version, 14, "all fourteen migrations recorded");
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p app migration` (or the test's exact name; search for `worktree_path_cols`)
Expected: FAIL — migration 014 not found / `version == 12 (or 13)` not 14.

- [ ] **Step 3: Create the migration and register it**

Create `src-tauri/app/migrations/014_task_worktree.sql`:

```sql
-- Worktree isolation (WT1): the per-work-item git worktree a task runs in.
-- Nullable, append-only; read-only stages and legacy tasks carry NULL.
ALTER TABLE tasks ADD COLUMN worktree_path TEXT;
```

In `run_migrations` `const MIGRATIONS` (after the `(12, ...)` entry — and after lifecycle-hardening's `(13, ...)` if present):

```rust
        (14, include_str!("../migrations/014_task_worktree.sql")),
```

In the tauri-plugin-sql `migrations` vec (after the version-12/13 `Migration { ... }` block):

```rust
        Migration {
            version: 14,
            description: "task worktree_path — per-work-item git worktree isolation",
            sql: include_str!("../migrations/014_task_worktree.sql"),
            kind: MigrationKind::Up,
        },
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add app/migrations/014_task_worktree.sql app/src/lib.rs
git commit -m "feat(app): migration 014 tasks.worktree_path; register in both lists

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: `working_dir` resolution in `invoke` + copy `worktree_path` to children

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` (`invoke` :1242-1310; child-build sites at 416, 498, 535, 793, 824, 844; `transform_once` parent threading; `release_orphaned_running` return is Task 8b but lives in task_store — see note).

This is the heart of the feature. `invoke` borrows `&Task`, so it cannot mutate the caller's `task`. The resolution therefore both (a) returns/exposes the resolved working dir for in-memory child threading and (b) persists it. The cleanest seam that keeps `invoke`'s signature: resolve inside `invoke` for the `working_dir`, AND have `transform_once` (and the other step fns) set `task.worktree_path` from the SAME resolution helper BEFORE building children. We extract a pure-ish async helper `resolve_working_dir` that both persists and returns the path.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` mod in `src-tauri/runtime/src/engine.rs`. These reuse the `RecordingProvider` from Task 5 and the existing test `ctx`/`team`/`Task::work_item` helpers (see the existing `invoke_sets_working_dir_to_effective_target_repo` test at :1638 for the harness pattern — a `FakeRunner` records the `InvocationRequest.working_dir`).

```rust
    #[tokio::test]
    async fn invoke_creates_and_persists_worktree_for_implementer_without_inherited_path() {
        // ctx wired with a RecordingProvider returning a known worktree path and a
        // FakeRunner that records the working_dir (mirror invoke_sets_working_dir_to_effective_target_repo).
        let provider = std::sync::Arc::new(RecordingProvider {
            ensure_calls: Default::default(),
            reset_calls: Default::default(),
            returns: "/proj/worktrees/R-1/alpha".into(),
        });
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::<InvocationRequest>::new()));
        let mut ctx = /* build the standard test ctx */ todo_build_ctx_with(received.clone());
        ctx.target_repo = Some(std::path::PathBuf::from("/repo"));
        ctx.worktree_provider = Some(provider.clone());
        let impl_team = team_with_role("implementers", None, Role::Implementer, 8);
        let item = Task::work_item("proj".into(), "pl".into(), ctx.run_id.clone(), "alpha".into(), "implementers".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();
        let _ = invoke(&ctx, &impl_team, &item, "sys".into()).await;
        // working_dir == the provider's returned path
        assert_eq!(received.lock().unwrap()[0].working_dir.as_deref(), Some("/proj/worktrees/R-1/alpha"));
        // ensure called once with (run, key, repo)
        assert_eq!(provider.ensure_calls.lock().unwrap().len(), 1);
        assert_eq!(provider.ensure_calls.lock().unwrap()[0], (ctx.run_id.clone(), "alpha".into(), "/repo".into()));
        // persisted on the task
        let back = ctx.tasks.get(&item.id).await.unwrap();
        assert_eq!(back.worktree_path.as_deref(), Some("/proj/worktrees/R-1/alpha"));
    }

    #[tokio::test]
    async fn invoke_uses_inherited_worktree_path_without_calling_ensure() {
        let provider = std::sync::Arc::new(RecordingProvider { ensure_calls: Default::default(), reset_calls: Default::default(), returns: "/should/not/be/used".into() });
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::<InvocationRequest>::new()));
        let mut ctx = todo_build_ctx_with(received.clone());
        ctx.target_repo = Some(std::path::PathBuf::from("/repo"));
        ctx.worktree_provider = Some(provider.clone());
        let impl_team = team_with_role("implementers", None, Role::Implementer, 8);
        let mut item = Task::work_item("proj".into(), "pl".into(), ctx.run_id.clone(), "alpha".into(), "implementers".into(), None, None, 100);
        item.worktree_path = Some("/proj/worktrees/R-1/alpha".into());
        ctx.tasks.insert(&item).await.unwrap();
        let _ = invoke(&ctx, &impl_team, &item, "sys".into()).await;
        assert_eq!(received.lock().unwrap()[0].working_dir.as_deref(), Some("/proj/worktrees/R-1/alpha"));
        assert!(provider.ensure_calls.lock().unwrap().is_empty(), "inherited path must not call ensure");
    }

    #[tokio::test]
    async fn invoke_producer_falls_back_to_target_repo() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::<InvocationRequest>::new()));
        let mut ctx = todo_build_ctx_with(received.clone());
        ctx.target_repo = Some(std::path::PathBuf::from("/repo"));
        ctx.worktree_provider = Some(std::sync::Arc::new(RecordingProvider { ensure_calls: Default::default(), reset_calls: Default::default(), returns: "/x".into() }));
        let prod = team_with_role("research", Some("spec"), Role::Producer, 8);
        let item = Task::work_item("proj".into(), "pl".into(), ctx.run_id.clone(), "alpha".into(), "research".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();
        let _ = invoke(&ctx, &prod, &item, "sys".into()).await;
        assert_eq!(received.lock().unwrap()[0].working_dir.as_deref(), Some("/repo"));
    }

    #[tokio::test]
    async fn invoke_implementer_with_no_provider_falls_back_to_target_repo() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::<InvocationRequest>::new()));
        let mut ctx = todo_build_ctx_with(received.clone());
        ctx.target_repo = Some(std::path::PathBuf::from("/repo"));
        ctx.worktree_provider = None; // pure-runtime fallback
        let impl_team = team_with_role("implementers", None, Role::Implementer, 8);
        let item = Task::work_item("proj".into(), "pl".into(), ctx.run_id.clone(), "alpha".into(), "implementers".into(), None, None, 100);
        ctx.tasks.insert(&item).await.unwrap();
        let _ = invoke(&ctx, &impl_team, &item, "sys".into()).await;
        assert_eq!(received.lock().unwrap()[0].working_dir.as_deref(), Some("/repo"));
    }
```

IMPLEMENTER NOTE: replace `todo_build_ctx_with(...)` and `team_with_role(...)` with the EXACT existing test helpers in `engine.rs` (the file already builds a ctx + a `FakeRunner` recording `InvocationRequest`s in `invoke_sets_working_dir_to_effective_target_repo` at :1638, and a `team(id, on_approve, role, capacity)` helper at :1640). Reuse those verbatim; do NOT introduce new helpers if the existing ones suffice. The `team(...)` helper already takes a `Role`, so pass `Role::Implementer`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime invoke_creates_and_persists_worktree_for_implementer_without_inherited_path`
Expected: FAIL — assertion: working_dir is `/repo`, not the worktree path (resolution not implemented yet).

- [ ] **Step 3: Implement the resolution in `invoke`**

In `src-tauri/runtime/src/engine.rs`, REPLACE the `working_dir` computation (currently `engine.rs:1266`):

```rust
    let working_dir = effective_repo.map(|p| p.to_string_lossy().into_owned());
```

with the worktree-aware resolution (uses `effective_repo: Option<PathBuf>`, `task.worktree_path`, `team.role`, `ctx.worktree_provider`, `ctx.run_id`, `task.item_key`, `ctx.tasks`):

```rust
    // Worktree isolation: an inherited path wins; else an Implementer stage
    // creates+records a worktree via the injected provider; else (read-only
    // stages, or no provider/repo) the effective target repo (chunk-1 behavior).
    let working_dir = if let Some(p) = task.worktree_path.clone() {
        Some(p)
    } else if team.role == pipeline::model::Role::Implementer {
        match (&ctx.worktree_provider, effective_repo.as_ref()) {
            (Some(wp), Some(repo)) => {
                let path = wp
                    .ensure(&ctx.run_id, task.item_key.as_deref().unwrap_or_default(), &repo.to_string_lossy())
                    .map_err(EngineError::Invoke)?;
                // Record on the task so downstream children inherit it. Best-effort
                // ordering: persist before the run so a crash mid-run still resumes
                // on the same tree.
                ctx.tasks.set_worktree_path(&task.id.0, &path).await?;
                Some(path)
            }
            _ => effective_repo.as_ref().map(|p| p.to_string_lossy().into_owned()),
        }
    } else {
        effective_repo.as_ref().map(|p| p.to_string_lossy().into_owned())
    };
```

NOTE on `effective_repo` ownership: it is currently consumed by `.map(...)`. The block above borrows it via `.as_ref()`; confirm the earlier `if let Some(repo) = effective_repo.clone()` (binding `${target_repo}`) still compiles — it clones, so `effective_repo` remains available here.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime invoke_creates_and_persists_worktree_for_implementer_without_inherited_path invoke_uses_inherited_worktree_path_without_calling_ensure invoke_producer_falls_back_to_target_repo invoke_implementer_with_no_provider_falls_back_to_target_repo`
Expected: PASS.

- [ ] **Step 5: Thread `worktree_path` to children (in-memory + every child-build site)**

The persisted `worktree_path` must reach children built from the parent. `invoke` persisted it but the in-memory `task` in `transform_once` (and the other step fns) is stale. Two places to handle:

(a) In `transform_once` (around :366), AFTER `let result = invoke(...).await;` and BEFORE building the child at :416, refresh the parent's worktree path from the store so the child copy sees it (a re-read is the simplest correct seam — the worktree was just persisted inside `invoke`):

```rust
    // The implementer's worktree (if any) was just persisted inside `invoke`;
    // refresh it onto the in-memory parent so the child copy below inherits it.
    if task.worktree_path.is_none() {
        if let Ok(refreshed) = ctx.tasks.get(&task.id).await {
            task.worktree_path = refreshed.worktree_path;
        }
    }
```

(b) At EVERY `Task::work_item(...)` child-build site that constructs a downstream/continuation child from a parent task, copy the parent's `worktree_path` onto the child right after construction. Apply at lines 416 (transform child — copies from `task`), 498 (gate approve — from `task`), 535 (gate revise — from `task`), 793 (join downstream — from `lane_task`), 824 (join revise — from `lane_task`), 844 (join escalate — from `lane_task`). For each, add immediately after the `let mut child = Task::work_item(...);` line:

```rust
        child.worktree_path = task.worktree_path.clone();        // sites 416/498/535
```
or, at the join sites (the parent variable is `lane_task`):
```rust
        child.worktree_path = lane_task.worktree_path.clone();   // sites 793/824/844
```

`Task::forked` (site 663) ALREADY inherits `worktree_path` (Task 4), so the fork lanes are covered — do nothing there. The generator sites (936 transient pass task, 988 generator child) are SOURCE-stage items (`research`, a producer/source) with no worktree — leave them as-is (`worktree_path` defaults to `None`).

Add a threading test:

```rust
    #[tokio::test]
    async fn transform_child_inherits_parent_worktree_path() {
        // An Implementer transform whose downstream is a Reviewer (code-review):
        // the committed child must carry the implementer's worktree_path so the
        // reviewer shares the tree.
        // Build ctx with a RecordingProvider returning /proj/worktrees/R-1/alpha,
        // pipeline = [implementers (Implementer) -> code-reviewers (Reviewer, terminal)].
        // After transform_once on the implementer item, the inserted code-reviewers
        // child carries worktree_path == /proj/worktrees/R-1/alpha.
        // (Mirror the existing transform_once happy-path test harness.)
    }
```

IMPLEMENTER NOTE: flesh out the threading test body using the existing transform_once test harness in `engine.rs` (search for a passing `transform_once` test that asserts a committed child by listing `ctx.tasks.list_by_state(TaskState::Queued)` and reading its fields). Assert `child.worktree_path == Some("/proj/worktrees/R-1/alpha")`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS (all engine tests, including the regression tests at :1638/:1662/:2617/:2648 that assert chunk-1 target-repo `working_dir` for producers).

- [ ] **Step 7: Commit**

```bash
cd src-tauri && git add runtime/src/engine.rs
git commit -m "feat(runtime): worktree-aware working_dir resolution; thread worktree_path to children

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8b: `release_orphaned_running` returns the re-queued rows (WT2 seam, runtime stays git-unaware)

**Files:**
- Modify: `src-tauri/runtime/src/task_store.rs` (`release_orphaned_running` :230, tests :400).

> RECONCILE: if lifecycle-hardening already changed `release_orphaned_running`'s signature/behavior (LH8 / crash-orphan reaping), adapt to ITS return shape — the requirement is only that the composition root can learn each re-queued task's id + `worktree_path`. If it already returns rows, skip this task and reuse that.

- [ ] **Step 1: Write the failing test**

Replace/augment the existing `release_orphaned_running_requeues` test (:400) to assert the returned rows expose `worktree_path`:

```rust
    #[tokio::test]
    async fn release_orphaned_running_returns_requeued_rows_with_worktree_path() {
        let store = TaskStore::new(fresh_pool().await);
        let mut t = Task::work_item("p".into(), "pl".into(), "R-1".into(), "alpha".into(), "implementers".into(), None, None, 100);
        t.state = TaskState::Running;
        t.worktree_path = Some("/p/worktrees/R-1/alpha".into());
        store.insert(&t).await.unwrap();
        let requeued = store.release_orphaned_running(300).await.unwrap();
        assert_eq!(requeued.len(), 1);
        assert_eq!(requeued[0].id, t.id);
        assert_eq!(requeued[0].worktree_path.as_deref(), Some("/p/worktrees/R-1/alpha"));
        // and the row is now queued
        assert_eq!(store.get(&t.id).await.unwrap().state, TaskState::Queued);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p runtime release_orphaned_running_returns_requeued_rows_with_worktree_path`
Expected: FAIL — `release_orphaned_running` returns `u64`, has no `.len()`.

- [ ] **Step 3: Change the return type to the re-queued rows**

In `src-tauri/runtime/src/task_store.rs`, replace `release_orphaned_running`:

```rust
    /// Crash recovery (spec F4): release any task stuck in `running` back to
    /// `queued` on startup. Returns the re-queued task rows so the composition
    /// root can reset their worktrees (WT2) — runtime stays git-unaware.
    pub async fn release_orphaned_running(&self, now_unix: i64) -> Result<Vec<Task>, TaskStoreError> {
        // Snapshot the running rows BEFORE flipping them (we need their
        // worktree_path; the UPDATE does not return rows in sqlite).
        let running = self.list_by_state(TaskState::Running).await?;
        sqlx::query("UPDATE tasks SET state='queued', updated_at=? WHERE state='running'")
            .bind(now_unix)
            .execute(&self.pool)
            .await?;
        Ok(running)
    }
```

- [ ] **Step 4: Fix the call site in `app/src/lib.rs`**

The boot call at `app/src/lib.rs:1337` is `let _ = tasks.release_orphaned_running(now_unix()).await;`. It still compiles (the `Vec<Task>` is dropped). Leave it for now; Task 9 rewrites it to use the returned rows.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cd src-tauri && git add runtime/src/task_store.rs
git commit -m "feat(runtime): release_orphaned_running returns re-queued rows (WT2 seam)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: App `GitCliWorktreeProvider` + inject into the activator + reset-on-resume wiring

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (new provider impl near the other root impls ~:85; managed wiring ~:1300; boot recovery ~:1337; activator deps construction).
- Modify: `src-tauri/app/src/pipeline_activator.rs` (`deps` struct + `ctx_builder` :361-407).
- Modify: `src-tauri/runtime/src/api.rs` (gate-verdict ctx :176 — add `worktree_provider: None`).

- [ ] **Step 1: Write the failing test (provider behavior over a FakeGit)**

Add a test in `src-tauri/app/src/lib.rs` (in an existing `#[cfg(test)] mod` or a new one) that exercises `GitCliWorktreeProvider::ensure` derives the right path + branch and calls `WorktreeGit::add` once via a fake:

```rust
    #[test]
    fn git_cli_worktree_provider_ensure_derives_path_and_branch() {
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
        }
        let git = std::sync::Arc::new(FakeGit { added: Mutex::new(vec![]) });
        // The provider needs the PROJECT ROOT to scope worktrees/ — it is built
        // with the project root; ensure() takes (run, key, target_repo).
        let provider = GitCliWorktreeProvider::new(git.clone(), "/proj".into());
        let path = runtime::engine::WorktreeProvider::ensure(&provider, "R-1", "alpha", "/repo").unwrap();
        assert_eq!(path, "/proj/worktrees/R-1/alpha");
        let calls = git.added.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "/proj/worktrees/R-1/alpha");
        assert_eq!(calls[0].2, "agent-bus/R-1/alpha");
        // base is the target repo HEAD ref
        assert_eq!(calls[0].0, "/repo");
        assert_eq!(calls[0].3, "HEAD");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p app git_cli_worktree_provider_ensure_derives_path_and_branch`
Expected: FAIL — `cannot find type GitCliWorktreeProvider`.

- [ ] **Step 3: Implement `GitCliWorktreeProvider`**

Add to `src-tauri/app/src/lib.rs` (near `SqliteRevisionReader`, ~:85). It holds the project root (to derive the `worktrees/` path) and the `WorktreeGit` seam. `ensure` is idempotent: if the path already exists it skips `add`.

```rust
/// The app's git-backed worktree provider (worktree isolation). Implements the
/// git-unaware `runtime::engine::WorktreeProvider` over `workspace`'s
/// `WorktreeGit` seam. Lives at the composition root so `runtime` never learns
/// `git`. The project root scopes every worktree under `<root>/worktrees/`.
pub struct GitCliWorktreeProvider {
    git: std::sync::Arc<dyn workspace::worktree::WorktreeGit>,
    project_root: String,
}

impl GitCliWorktreeProvider {
    pub fn new(git: std::sync::Arc<dyn workspace::worktree::WorktreeGit>, project_root: String) -> Self {
        Self { git, project_root }
    }

    fn worktree_path_for(&self, run_id: &str, item_key: &str) -> String {
        std::path::Path::new(&self.project_root)
            .join("worktrees")
            .join(run_id)
            .join(item_key)
            .to_string_lossy()
            .into_owned()
    }
}

impl runtime::engine::WorktreeProvider for GitCliWorktreeProvider {
    fn ensure(&self, run_id: &str, item_key: &str, target_repo: &str) -> Result<String, String> {
        let path = self.worktree_path_for(run_id, item_key);
        // Idempotent: a worktree dir already present (resume / re-claim) is reused.
        if std::path::Path::new(&path).is_dir() {
            return Ok(path);
        }
        let branch = format!("agent-bus/{run_id}/{item_key}");
        // Create the branch off the target repo's current HEAD; path-scoped guard
        // runs no git on an out-of-subtree path.
        workspace::worktree::add_worktree_inner(
            self.git.as_ref(),
            &self.project_root,
            &path,
            &branch,
            "HEAD",
        )?;
        // NOTE: add_worktree_inner runs `git -C <project_root> worktree add ...`;
        // the base_ref "HEAD" resolves in the project root's repo. If the target
        // repo differs from the project root, the implementer must confirm the
        // `git -C` repo argument: per spec the worktree is created off the TARGET
        // repo's HEAD. RECONCILE: if target_repo != project_root, pass target_repo
        // as the `-C` repo to add_worktree_inner (extend its signature) — keep the
        // path-scope guard on `path` against project_root unchanged.
        Ok(path)
    }

    fn reset(&self, worktree_path: &str) -> Result<(), String> {
        workspace::worktree::reset_worktree_inner(self.git.as_ref(), &self.project_root, worktree_path)
    }
}
```

RECONCILE (target repo vs project root): the spec says the worktree is created off "the target repo's current HEAD" and lives under `<project_root>/worktrees/`. `git worktree add` must run with `-C <the repo that owns the branch>`. Confirm at execution whether `add_worktree_inner` should take the repo (`target_repo`) separately from the path-scope root (`project_root`). If so, extend `add_worktree_inner(git, scope_root, repo, path, branch, base)` (path-scope `path` against `scope_root`, run `git -C repo`). Update Task 3's signature + tests accordingly. This plan keeps them equal for the common case (project root IS the target repo) and flags the divergence.

- [ ] **Step 4: Inject the provider into the activator deps + both EngineContext builders**

In `src-tauri/app/src/pipeline_activator.rs`: add a field to the `deps` struct (the struct that holds `stores`/`runs`/etc. — search for `pub stores:` / `stores:` in the deps definition) :

```rust
    pub worktree_provider: Option<std::sync::Arc<dyn runtime::engine::WorktreeProvider>>,
```

In `ctx_builder` (:361), clone it before the closure and set it in the `EngineContext { ... }` literal (:386, after `audit: audit.clone(),`):

```rust
        let worktree_provider = self.deps.worktree_provider.clone();
```
```rust
            worktree_provider: worktree_provider.clone(),
```

In `src-tauri/runtime/src/api.rs:176` (gate-verdict ctx), add (after `audit: None,`):

```rust
            worktree_provider: None,
```

In `src-tauri/app/src/lib.rs` where the activator deps are constructed (search for the struct literal that fills `stores`/`runs`/`ledger`/`fanout`/`audit`), build and pass the provider. The provider needs the project root + the managed `GitCli`. Reuse the same `GitCli` already managed at :1302:

```rust
                let worktree_git: std::sync::Arc<dyn workspace::worktree::WorktreeGit> =
                    std::sync::Arc::new(workspace::worktree::GitCli);
                let worktree_provider: Option<std::sync::Arc<dyn runtime::engine::WorktreeProvider>> =
                    Some(std::sync::Arc::new(GitCliWorktreeProvider::new(
                        worktree_git.clone(),
                        project_root.clone(),
                    )));
```

and add `worktree_provider: worktree_provider.clone(),` to the deps literal. (`project_root` is in scope from `load_active` at :1331.)

- [ ] **Step 5: Wire reset-on-resume at boot (WT2)**

Replace the boot recovery call at `app/src/lib.rs:1337`:

```rust
                let _ = tasks.release_orphaned_running(now_unix()).await;
```

with the reset-on-resume loop (resets ONLY re-queued tasks that carry a `worktree_path` — read-only stages have none, so they are skipped; never clobbers a cleanly-committed worktree of a task that was not re-queued):

```rust
                // F4 crash recovery + WT2 reset-on-resume: re-queue orphaned
                // running tasks, then reset each re-queued implementer worktree to
                // its baseline before a worker can re-claim it (the kill may have
                // left a half-written tree). Runtime stays git-unaware: it reports
                // the re-queued rows; the root (which holds the provider) resets.
                match tasks.release_orphaned_running(now_unix()).await {
                    Ok(requeued) => {
                        if let Some(wp) = worktree_provider.as_ref() {
                            for t in &requeued {
                                if let Some(path) = t.worktree_path.as_deref() {
                                    if let Err(e) = wp.reset(path) {
                                        eprintln!("app: WT2 worktree reset failed for {}: {e}", t.id.0);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("app: release_orphaned_running failed: {e}"),
                }
```

RECONCILE: if lifecycle-hardening moved/renamed the boot recovery (crash-orphan reaping LH4) or added a brake-off re-queue path, apply the SAME reset loop over THAT path's re-queued rows. The provider is in scope at the root; reset only the re-queued tasks with a `worktree_path`.

- [ ] **Step 6: Run the tests + build to verify**

Run: `cd src-tauri && cargo test -p app && cargo build`
Expected: PASS / clean build.

- [ ] **Step 7: Commit**

```bash
cd src-tauri && git add app/src/lib.rs app/src/pipeline_activator.rs runtime/src/api.rs
git commit -m "feat(app): GitCliWorktreeProvider; inject into engine ctx; WT2 reset-on-resume

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: TS `Task.worktree_path` mirror + TS gates

**Files:**
- Modify: `src/ipc/runtime.ts:14` (`Task` interface).

- [ ] **Step 1: Add the optional field**

In `src/ipc/runtime.ts`, add to `interface Task` (after `item_key?`):

```ts
  /// The per-work-item git worktree the task runs in (worktree isolation). `null`
  /// for read-only stages and legacy tasks. Surfaced for a future diff viewer.
  worktree_path?: string | null;
```

- [ ] **Step 2: Run the TS gates**

Run: `npx tsc --noEmit && npx vitest run`
Expected: PASS (additive-optional; existing Task literals in tests still type-check).

- [ ] **Step 3: Commit**

```bash
git add src/ipc/runtime.ts
git commit -m "feat(ui): mirror Task.worktree_path in the IPC type

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: Full verification gate

**Files:** none (verification only).

- [ ] **Step 1: Run the Rust gates**

```bash
cd src-tauri && cargo test
```
Expected: PASS (all crates).

- [ ] **Step 2: Clippy (deny warnings)**

```bash
cd src-tauri && cargo clippy --all-targets -- -D warnings
```
Expected: clean. (Watch for `clippy::large_enum_variant` on `Role` — it is `Copy`/tiny, no boxing needed.)

- [ ] **Step 3: Build**

```bash
cd src-tauri && cargo build
```
Expected: clean.

- [ ] **Step 4: TS gates (a TS type changed in Task 10)**

```bash
npx tsc --noEmit && npx vitest run
```
Expected: PASS.

- [ ] **Step 5: Final commit (if any verification fixups were needed)**

```bash
git add -A && git commit -m "chore: worktree isolation verification fixups

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review (spec coverage)

- Spec §Role::Implementer → Task 1 (enum) + Task 2 (seed tag) + Task 5 (`role_str`).
- Spec §WorktreeProvider seam → Task 5 (trait + ctx field) + Task 9 (app impl + injection + gate-verdict None).
- Spec §`WorktreeGit::add`/`reset` path-scoped → Task 3 (trait/GitCli/FakeGit + `add_worktree_inner`/`reset_worktree_inner` guards + reject-no-git tests).
- Spec §working_dir resolution → Task 8 (inherited → ensure → fallback; no-provider fallback; producer regression).
- Spec §Threading to children → Task 4 (`forked` inherit) + Task 8 Step 5 (transform/gate/join child copies; generator left None).
- Spec §`Task.worktree_path` column + migration 014 + both lists + idempotency bump → Tasks 4, 6, 7.
- Spec §`set_worktree_path` → Task 6.
- Spec §Reset on resume (WT2) → Task 8b (return rows seam) + Task 9 Step 5 (root reset loop, re-queued implementer tasks only) with the lifecycle-hardening RECONCILE note.
- Spec §Testing matrix → Tasks 1/3/6/8/8b each implement the named cases (serialize+round-trip; path-scope reject; resolution variants; threading; reset-once; no-provider fallback).
- Spec §No SCHEMA_VERSION bump → Task 1 (explicit).

## Execution Handoff

Plan complete. Two execution options:
1. **Subagent-Driven (recommended)** — fresh subagent per task, two-stage review between tasks.
2. **Inline Execution** — batch with checkpoints via executing-plans.
