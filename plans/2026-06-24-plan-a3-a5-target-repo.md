# A3 + A5 — Folder picker + project target-repo binding

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a project-level `target_repo` (Workspace) that binds `${target_repo}` for every team's scope resolution and defaults the inject target, and a reusable native folder-picker (`FolderPickerField`) used for Root path (A3) and Target repo (A5) in the wizard Basics step and Settings.

**Architecture:** `target_repo` is a **Workspace** `Project` attribute (migration 010, append-only, tilde-expanded at create like `root_path`). The `${target_repo}` binding flows through the **path-resolution shared kernel** (`PathVars`) at the **worker-loop/composition root**: per task, `PathVars.target_repo = task.target_repo.or(project.target_repo)` — task overrides, project is the default. **Runtime stays ignorant of the Project** — the project's `target_repo` is read at the composition root (`load_active`) and passed in as a plain `Option<PathBuf>`/`Option<String>` (no new cross-context edge). The native dialog is a UI/Workspace concern behind a `pickFolder()` IPC wrapper over `@tauri-apps/plugin-dialog`.

**Tech Stack:** Rust (sqlx, Tauri 2), React + TypeScript, Vitest, `@tauri-apps/plugin-dialog` / `tauri-plugin-dialog`.

---

## Decisions

- **DD1 — Where the project→task default is applied.** AUTO: at the **worker loop** (`PoolContext.project_target_repo`) the PathVars default is `task.target_repo.or(project_target_repo)`; AND at **inject** (`inject_topic_inner` defaults the new task's `target_repo` to the project's when the caller supplies none). Both, because a task created before this change (or via a path that didn't set it) still resolves `${target_repo}` at the worker via the pool default; the inject default makes the stored task self-describing. Task-level inject still overrides.
- **DD2 — Runtime ignorance.** `RuntimeState` gains a `project_target_repo: Option<String>` field (read from the Project at `load_active`). This is NOT Runtime learning about the Project type — it's a plain resolved string handed in at the composition root, exactly like `project_root` already is. Runtime never imports `workspace::Project`.
- **DD3 — Pool default vs Runtime.** `PoolContext` gets `project_target_repo: Option<PathBuf>` (resolved, already tilde-expanded). The pool builds PathVars; Runtime's pool consumes already-resolved paths. No Project type crosses into the pool.
- **DD4 — Migration 010 shape.** `ALTER TABLE projects ADD COLUMN target_repo TEXT;` (nullable, append-only). Registered in BOTH migration lists; the idempotency test bumps `user_version` 9→10 and asserts the new column exists exactly once.
- **DD5 — `create_project_from_draft` carries `target_repo`.** A new optional 4th arg `target_repo: Option<String>` (expanded like root). The wizard passes it through. `workspace_create_project` likewise gains an optional `target_repo`.
- **DD6 — FolderPickerField returns absolute.** The picker writes the chosen absolute path into the field; typed input may still contain `~` (expanded backend-side). Same component for Root path and Target repo.
- **DD7 — Dialog plugin contained.** `@tauri-apps/plugin-dialog` (JS) + `tauri-plugin-dialog` (Rust) + `dialog:allow-open` capability. Live dialog is structural-only (can't run headless); component tests mock `pickFolder()`.

---

## Task 1: Migration 010 — `projects.target_repo` column

**Files:**
- Create: `src-tauri/app/migrations/010_project_target_repo.sql`
- Modify: `src-tauri/app/src/lib.rs` (both migration lists + idempotency test)

- [ ] **Step 1: Write the migration file**

`src-tauri/app/migrations/010_project_target_repo.sql`:
```sql
-- A5: project-level target repo. Binds ${target_repo} for all teams' scope
-- resolution and defaults the inject target. Nullable; tilde-expanded at create
-- (Workspace), same discipline as root_path. Append-only; 001-009 untouched.
ALTER TABLE projects ADD COLUMN target_repo TEXT;
```

- [ ] **Step 2: Register in `run_migrations` const list (lib.rs ~line 40-49)**

Add after the `(9, …009…)` entry:
```rust
        (10, include_str!("../migrations/010_project_target_repo.sql")),
```

- [ ] **Step 3: Register in the tauri-plugin-sql `migrations` vec (lib.rs ~line 805-860)**

Add after the version-9 `Migration { … }`:
```rust
        Migration {
            version: 10,
            description: "project target repo — projects.target_repo column",
            sql: include_str!("../migrations/010_project_target_repo.sql"),
            kind: MigrationKind::Up,
        },
```

- [ ] **Step 4: Bump the idempotency test (lib.rs ~line 1448-1452)**

Change the assertion from `9` to `10`, and add a column-presence assertion before it:
```rust
        let target_repo_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('projects') WHERE name='target_repo'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(target_repo_cols, 1, "migration 010 column present exactly once");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 10, "all ten migrations recorded");
```

- [ ] **Step 5: Run the test**

Run: `cd src-tauri && cargo test -p agent-bus-app fresh_file_is_created_migrated_and_idempotent`
Expected: PASS (version 10, column present once, second run idempotent — the `ADD COLUMN` is gated by user_version).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/migrations/010_project_target_repo.sql src-tauri/app/src/lib.rs
git commit -m "feat(workspace): migration 010 — projects.target_repo column"
```

---

## Task 2: `Project.target_repo` field + store round-trip

**Files:**
- Modify: `src-tauri/workspace/src/project.rs`
- Modify: `src-tauri/workspace/src/store.rs`

- [ ] **Step 1: Add a failing store test (store.rs tests mod)**

The existing `fresh_pool()` only runs migration 001. Update it to also run 010 so the column exists, then add the test:
```rust
    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../app/migrations/010_project_target_repo.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }
```
```rust
    #[tokio::test]
    async fn set_target_repo_updates_the_column() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        store.insert(&p).await.unwrap();
        store.set_target_repo(&p.id, Some("/repo"), 200).await.unwrap();
        let reloaded = store.get(&p.id).await.unwrap();
        assert_eq!(reloaded.target_repo, Some("/repo".to_string()));
        assert_eq!(reloaded.updated_at, 200);
    }
```

- [ ] **Step 2: Run it — fails to compile**

Run: `cd src-tauri && cargo test -p workspace set_target_repo_updates_the_column`
Expected: FAIL — `Project` has no field `target_repo`, no method `set_target_repo`.

- [ ] **Step 3: Add the field to `Project` (project.rs)**

In the struct (after `root_path`):
```rust
    pub root_path: PathBuf,
    /// `${target_repo}` default for all teams (A5). Tilde-expanded at create,
    /// same as root_path. None = unset.
    #[serde(default)]
    pub target_repo: Option<String>,
```
In `Project::new`, set `target_repo: None,` after `root_path,`. Update the two existing project.rs tests to expect `target_repo: None` (the round-trip test needs no change; `project_new_…` can assert `assert_eq!(p.target_repo, None);`).

- [ ] **Step 4: Update `ProjectStore` (store.rs) insert/list/get + add `set_target_repo`**

`insert`: add the column and bind:
```rust
        sqlx::query(
            "INSERT INTO projects (id, name, root_path, target_repo, active_pipeline_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&project.id.0)
        .bind(&project.name)
        .bind(project.root_path.to_string_lossy().to_string())
        .bind(project.target_repo.as_ref())
        .bind(project.active_pipeline_id.as_ref().map(|p| &p.0))
        .bind(project.created_at)
        .bind(project.updated_at)
```
`list` and `get`: change the tuple type to include `Option<String>` for target_repo and add it to the SELECT + the `Project { … }` build. New tuple shape: `(String, String, String, Option<String>, Option<String>, i64, i64)` with SELECT `id, name, root_path, target_repo, active_pipeline_id, created_at, updated_at`, mapping `target_repo` into the struct. (Apply to both `list` and `get`.)

Add the method:
```rust
    pub async fn set_target_repo(
        &self,
        id: &ProjectId,
        target_repo: Option<&str>,
        now_unix: i64,
    ) -> Result<(), ProjectStoreError> {
        let result = sqlx::query(
            "UPDATE projects SET target_repo = ?, updated_at = ? WHERE id = ?",
        )
        .bind(target_repo)
        .bind(now_unix)
        .bind(&id.0)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(ProjectStoreError::NotFound(id.clone()));
        }
        Ok(())
    }
```

- [ ] **Step 5: Fix the existing api.rs test helper (workspace/src/api.rs tests)**

`state_with_project` runs only 001 — add the 010 migration line after it (mirror Step 1's pattern) so inserts with the new column succeed.

- [ ] **Step 6: Run the workspace tests**

Run: `cd src-tauri && cargo test -p workspace`
Expected: PASS (new test + all existing).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/workspace/src/project.rs src-tauri/workspace/src/store.rs src-tauri/workspace/src/api.rs
git commit -m "feat(workspace): Project.target_repo field + store round-trip"
```

---

## Task 3: Surface `target_repo` on Workspace OHS commands + `tools()`

**Files:**
- Modify: `src-tauri/workspace/src/api.rs`

- [ ] **Step 1: Add a failing tools() test**

```rust
    #[test]
    fn tools_publishes_set_target_repo_under_workspace() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "workspace_set_target_repo" && s.supplier_context == "workspace"));
    }
```

- [ ] **Step 2: Run it — fails**

Run: `cd src-tauri && cargo test -p workspace tools_publishes_set_target_repo_under_workspace`
Expected: FAIL — tool not present.

- [ ] **Step 3: Extend `workspace_create_project` with optional `target_repo`**

```rust
#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_create_project(
    state: tauri::State<'_, WorkspaceState>,
    name: String,
    root_path: String,
    target_repo: Option<String>,
) -> Result<Project, String> {
    let home = home_dir();
    let root = expand_tilde(&root_path, &home);
    let mut project = Project::new(name, PathBuf::from(root), now_unix());
    project.target_repo = target_repo
        .filter(|s| !s.trim().is_empty())
        .map(|s| expand_tilde(&s, &home));
    state.store.insert(&project).await.map_err(|e| e.to_string())?;
    Ok(project)
}
```

- [ ] **Step 4: Add the `workspace_set_target_repo` command**

```rust
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
```

- [ ] **Step 5: Add both to `tools()`**

Update the `workspace_create_project` ToolSpec properties to add `"target_repo": { "type": ["string", "null"] }` (keep `required: ["name", "root_path"]`). Add a new ToolSpec:
```rust
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
```

- [ ] **Step 6: Add a create-with-target-repo unit test**

```rust
    #[tokio::test]
    async fn set_target_repo_command_expands_and_persists() {
        let root = std::env::temp_dir().join(format!("abp-tr-{}", uuid::Uuid::new_v4()));
        let (state, project_id) = state_with_project(&root).await;
        // simulate the command's inner logic via the store + expand_tilde
        let expanded = expand_tilde("~/repo", "/Users/tim");
        state.store.set_target_repo(&ProjectId(project_id.clone()), Some(&expanded), 0).await.unwrap();
        let got = state.store.get(&ProjectId(project_id)).await.unwrap();
        assert_eq!(got.target_repo, Some("/Users/tim/repo".to_string()));
    }
```

- [ ] **Step 7: Run workspace tests**

Run: `cd src-tauri && cargo test -p workspace`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/workspace/src/api.rs
git commit -m "feat(workspace): workspace_set_target_repo + target_repo on create + tools()"
```

---

## Task 4: Pool default — `PathVars.target_repo = task.or(project)`

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`

- [ ] **Step 1: Add a failing test in pool.rs tests**

Find an existing test that builds a `PoolContext` (search `PoolContext {`). Add a focused test that the per-task PathVars target_repo falls back to the pool default. Since PathVars binding is internal to `process_one_claim`, assert through the resolved scope file OR via a small extracted helper. Add this pure helper near the binding site:
```rust
/// Resolve the effective target_repo: task-level overrides the project-level
/// default (A5). The SINGLE precedence fn — both the worker PathVars build and
/// `inject_topic_inner`'s stored-task default call it (vet F2). Pure.
pub fn effective_target_repo(
    task_target: Option<&str>,
    project_default: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    task_target
        .map(std::path::PathBuf::from)
        .or_else(|| project_default.map(|p| p.to_path_buf()))
}
```
Test:
```rust
    #[test]
    fn task_target_repo_overrides_project_default() {
        use std::path::Path;
        assert_eq!(
            super::effective_target_repo(Some("/task-repo"), Some(Path::new("/proj-repo"))),
            Some(std::path::PathBuf::from("/task-repo"))
        );
        assert_eq!(
            super::effective_target_repo(None, Some(Path::new("/proj-repo"))),
            Some(std::path::PathBuf::from("/proj-repo"))
        );
        assert_eq!(super::effective_target_repo(None, None), None);
    }
```

- [ ] **Step 2: Run it — fails**

Run: `cd src-tauri && cargo test -p runtime task_target_repo_overrides_project_default`
Expected: FAIL — `effective_target_repo` / `project_target_repo` not defined.

- [ ] **Step 3: Add the field to `PoolContext`**

After `pub project_root: PathBuf,`:
```rust
    /// Project-level `${target_repo}` default (A5). A task's own `target_repo`
    /// overrides it; this is the fallback so `${target_repo}` resolves even when
    /// a task didn't carry one. Already tilde-expanded (Workspace). The pool
    /// consumes a resolved path — it never sees the Project.
    pub project_target_repo: Option<PathBuf>,
```

- [ ] **Step 4: Use it in `process_one_claim` PathVars build**

Replace the existing target_repo binding block:
```rust
    let mut vars = PathVars::new(&ctx.project_root).with_task_id(&task.id.0);
    if let Some(repo) = effective_target_repo(
        task.target_repo.as_deref(),
        ctx.project_target_repo.as_deref(),
    ) {
        vars = vars.with_target_repo(repo);
    }
```
(Add the `effective_target_repo` helper from Step 1 if not yet added.)

- [ ] **Step 5: Fix every other `PoolContext { … }` construction**

Search `cargo build -p runtime` errors for missing `project_target_repo`. Every test/helper that builds a `PoolContext` adds `project_target_repo: None,`. (There are several in `pool.rs` tests and possibly `contract_tests.rs`.)

- [ ] **Step 6: Run runtime tests**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(runtime): pool binds PathVars.target_repo = task.or(project) default"
```

---

## Task 5: Inject default + RuntimeState carries project target_repo

**Files:**
- Modify: `src-tauri/runtime/src/api.rs`

- [ ] **Step 1: Add a failing test (api.rs tests)**

Find the existing inject test (search `inject_topic_inner` in `#[cfg(test)]`). Add `project_target_repo` to the test `RuntimeState` and assert the default applies. Add a test:
```rust
    #[tokio::test]
    async fn inject_defaults_target_repo_to_project_when_unset() {
        let state = test_state_with_project_target_repo(Some("/proj-repo")).await;
        let task = inject_topic_inner(&state, "topic".into(), None).await.unwrap();
        assert_eq!(task.target_repo, Some("/proj-repo".to_string()));
        // explicit task-level wins
        let task2 = inject_topic_inner(&state, "t2".into(), Some("/task-repo".into())).await.unwrap();
        assert_eq!(task2.target_repo, Some("/task-repo".to_string()));
    }
```
(Mirror the existing inject-test state builder; add a variant or extend it to set `project_target_repo`. If existing tests construct `RuntimeState { … }` inline, add `project_target_repo: …` there.)

- [ ] **Step 2: Run it — fails**

Run: `cd src-tauri && cargo test -p runtime inject_defaults_target_repo_to_project_when_unset`
Expected: FAIL — `RuntimeState` has no `project_target_repo`.

- [ ] **Step 3: Add the field to `RuntimeState`**

After `pub project_root: String,`:
```rust
    /// Project-level `${target_repo}` default (A5). When an inject supplies no
    /// target_repo, the new task defaults to this. A plain resolved string handed
    /// in at the composition root (Runtime never learns about the Project type).
    pub project_target_repo: Option<String>,
```

- [ ] **Step 4: Apply the default in `inject_topic_inner`**

```rust
pub async fn inject_topic_inner(
    state: &RuntimeState,
    topic: String,
    target_repo: Option<String>,
) -> Result<Task, String> {
    let stage = entry_stage(&state.pipeline)?;
    // Vet F2: reuse the single precedence fn (task overrides project default).
    // This binds the STORED task field; the worker binds live PathVars with the
    // same rule — deliberately separate bindings of one rule, one precedence fn.
    let effective = crate::pool::effective_target_repo(
        target_repo.as_deref().filter(|s| !s.trim().is_empty()),
        state.project_target_repo.as_deref().map(std::path::Path::new),
    )
    .map(|p| p.to_string_lossy().into_owned());
    let task = Task::injected(
        state.project_id.clone(),
        state.pipeline.id.clone(),
        stage,
        topic,
        effective,
        now_unix(),
    );
    state.tasks.insert(&task).await.map_err(|e| e.to_string())?;
    Ok(task)
}
```

- [ ] **Step 5: Fix every other `RuntimeState { … }` construction in runtime tests**

Search `cargo build -p runtime --tests` for missing `project_target_repo`; add `project_target_repo: None,` to each.

- [ ] **Step 6: Run runtime tests**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/runtime/src/api.rs
git commit -m "feat(runtime): inject defaults target_repo to project; RuntimeState carries it"
```

---

## Task 6: Composition root — wire project.target_repo through load_active → pool + RuntimeState

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Extend `load_active` to also return the project's target_repo**

Change the return type to `(String, String, Option<String>, Pipeline)` and return `project.target_repo` (the project loaded at line ~791):
```rust
async fn load_active(
    project_store: &ProjectStore,
) -> (String, String, Option<String>, Pipeline) {
    // … empty placeholder unchanged …
    let Ok(projects) = project_store.list().await else { return (String::new(), String::new(), None, empty); };
    let Some(project) = projects.into_iter().next() else { return (String::new(), String::new(), None, empty); };
    let root = project.root_path.to_string_lossy().into_owned();
    let target_repo = project.target_repo.clone();
    let store = pipeline::store::PipelineStore::new(&root);
    let pipe = store.list_ids().ok()
        .and_then(|ids| ids.into_iter().next())
        .and_then(|id| store.load(&id).ok())
        .unwrap_or(empty);
    (project.id.0, root, target_repo, pipe)
}
```

- [ ] **Step 2: Update the call site (lib.rs ~line 916)**

```rust
                let (project_id, project_root, project_target_repo, pipe) = load_active(&project_store).await;
```

- [ ] **Step 3: Add `project_target_repo` to both `RuntimeState` constructions (lib.rs ~926, ~930)**

Add `project_target_repo: project_target_repo.clone(),` to each `RuntimeState { … }`.

- [ ] **Step 4: Pass it into `spawn_worker_loops` (lib.rs ~1039)**

Add a `project_target_repo: Option<String>` parameter to `spawn_worker_loops`, pass `project_target_repo.clone()` at the call site (after `project_root`). Inside `spawn_worker_loops`, convert once:
```rust
    let project_target_repo: Option<std::path::PathBuf> =
        project_target_repo.map(std::path::PathBuf::from);
```
and set `project_target_repo: project_target_repo.clone(),` in the `PoolContext { … }` (after `project_root: …`).

- [ ] **Step 5: Build the app crate**

Run: `cd src-tauri && cargo build -p agent-bus-app`
Expected: compiles (fix any remaining `RuntimeState`/`PoolContext` literals flagged).

- [ ] **Step 6: Update `create_project_from_draft` to carry target_repo (lib.rs ~591-631)**

Add an optional `target_repo: Option<String>` arg to BOTH `create_project_from_draft_inner` and the `#[tauri::command]` wrapper. In the inner, after creating the project:
```rust
    let mut project = Project::new(name, std::path::PathBuf::from(expanded), now_unix());
    project.target_repo = target_repo
        .filter(|s| !s.trim().is_empty())
        .map(|s| workspace::api::expand_tilde(&s, &std::env::var("HOME").unwrap_or_default()));
    ws.store.insert(&project).await.map_err(|e| e.to_string())?;
```

- [ ] **Step 7: Update create_project_from_draft callers/tests**

Search lib.rs tests for `create_project_from_draft_inner(` and add `None` as the new arg. Check the terminal dispatcher (if it calls create_project_from_draft) — add the arg there too.

- [ ] **Step 8: Build + test app crate**

Run: `cd src-tauri && cargo test -p agent-bus-app`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): wire project.target_repo through load_active to pool + RuntimeState + create-from-draft"
```

---

## Task 7: Register `workspace_set_target_repo` command + dialog plugin (Rust)

**Files:**
- Modify: `src-tauri/Cargo.toml` (workspace deps), `src-tauri/app/Cargo.toml`
- Modify: `src-tauri/app/src/lib.rs` (invoke_handler + plugin + dispatcher if applicable)
- Modify: `src-tauri/app/capabilities/default.json`

- [ ] **Step 1: Add `tauri-plugin-dialog` to workspace + app Cargo.toml**

In `src-tauri/Cargo.toml` `[workspace.dependencies]` add:
```toml
tauri-plugin-dialog = "2"
```
In `src-tauri/app/Cargo.toml` `[dependencies]` add:
```toml
tauri-plugin-dialog.workspace = true
```

- [ ] **Step 2: Register the plugin (lib.rs builder chain ~862)**

Before `.plugin(tauri_plugin_sql::Builder…)`:
```rust
        .plugin(tauri_plugin_dialog::init())
```

- [ ] **Step 3: Register `workspace_set_target_repo` in `invoke_handler`**

Find the `tauri::generate_handler![ … ]` list (search `workspace_set_active_pipeline,` in the handler) and add `workspace::api::workspace_set_target_repo,` next to the other workspace commands.

- [ ] **Step 4: Add `dialog:allow-open` capability**

`src-tauri/app/capabilities/default.json` permissions:
```json
  "permissions": [
    "core:default",
    "dialog:allow-open"
  ]
```

- [ ] **Step 5: Build**

Run: `cd src-tauri && cargo build -p agent-bus-app`
Expected: compiles (dialog plugin resolves; if the exact crate version differs, adapt and note it).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/app/Cargo.toml src-tauri/app/src/lib.rs src-tauri/app/capabilities/default.json src-tauri/Cargo.lock
git commit -m "feat(app): register tauri-plugin-dialog + dialog:allow-open + set_target_repo command"
```

---

## Task 8: Frontend — `pickFolder()` IPC + `workspaceSetTargetRepo` + types

**Files:**
- Modify: `src/ipc/workspace.ts`
- Modify: `package.json`

- [ ] **Step 1: Add the dialog dep**

In `package.json` dependencies add `"@tauri-apps/plugin-dialog": "^2"`, then:
Run: `export PATH=/opt/homebrew/bin:$PATH && bun install`
Expected: dep added to bun.lock.

- [ ] **Step 2: Add `pickFolder()` + `workspaceSetTargetRepo` + extend Project type (workspace.ts)**

```ts
import { open } from "@tauri-apps/plugin-dialog";

export interface Project {
  id: string;
  name: string;
  root_path: string;
  target_repo: string | null;
  active_pipeline_id: string | null;
  created_at: number;
  updated_at: number;
}

/** Open the native folder picker; returns the chosen absolute path, or null if
 *  cancelled. Single directory selection. Sole crossing point for the
 *  `@tauri-apps/plugin-dialog` idiom — components depend on this wrapper, not the
 *  plugin (vet F3, same ACL-seal discipline as the keychain/git seams). */
export async function pickFolder(): Promise<string | null> {
  const result = await open({ directory: true, multiple: false });
  return typeof result === "string" ? result : null;
}

export async function workspaceSetTargetRepo(
  id: string,
  targetRepo: string | null,
): Promise<void> {
  await invoke("workspace_set_target_repo", { id, target_repo: targetRepo });
}
```

- [ ] **Step 3: Extend `createProjectFromDraft` IPC (pipeline.ts) with target_repo**

```ts
export async function createProjectFromDraft(
  name: string,
  root: string,
  draft: DraftPipeline,
  targetRepo?: string | null,
): Promise<{ id: string; name: string; root_path: string; target_repo: string | null; active_pipeline_id: string | null; created_at: number; updated_at: number }> {
  return await invoke("create_project_from_draft", { name, root, draft, target_repo: targetRepo ?? null });
}
```

- [ ] **Step 4: Typecheck**

Run: `export PATH=/opt/homebrew/bin:$PATH && bun run build`
Expected: tsc passes (no usages broken yet; the new Project field is additive).

- [ ] **Step 5: Commit**

```bash
git add src/ipc/workspace.ts src/ipc/pipeline.ts package.json bun.lock
git commit -m "feat(ipc): pickFolder + workspaceSetTargetRepo + target_repo on Project/create"
```

---

## Task 9: `FolderPickerField` component + test

**Files:**
- Create: `src/components/FolderPickerField.tsx`
- Create: `src/components/FolderPickerField.test.tsx`

- [ ] **Step 1: Write the failing component test**

`src/components/FolderPickerField.test.tsx`:
```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const pickFolderMock = vi.fn();
vi.mock("../ipc/workspace", () => ({
  pickFolder: () => pickFolderMock(),
}));

import { FolderPickerField } from "./FolderPickerField";

describe("FolderPickerField", () => {
  it("renders label + value and fires onChange on typing", () => {
    const onChange = vi.fn();
    render(<FolderPickerField label="Root path" value="~/x" onChange={onChange} />);
    expect(screen.getByText("Root path")).toBeInTheDocument();
    const input = screen.getByLabelText("Root path") as HTMLInputElement;
    expect(input.value).toBe("~/x");
    fireEvent.change(input, { target: { value: "~/y" } });
    expect(onChange).toHaveBeenCalledWith("~/y");
  });

  it("writes the picked absolute path into the field via Browse", async () => {
    pickFolderMock.mockResolvedValueOnce("/Users/tim/projects/example");
    const onChange = vi.fn();
    render(<FolderPickerField label="Target repo" value="" onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /browse/i }));
    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith("/Users/tim/projects/example"),
    );
  });

  it("does nothing when the picker is cancelled (null)", async () => {
    pickFolderMock.mockResolvedValueOnce(null);
    const onChange = vi.fn();
    render(<FolderPickerField label="Target repo" value="keep" onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /browse/i }));
    await waitFor(() => expect(pickFolderMock).toHaveBeenCalled());
    expect(onChange).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run it — fails**

Run: `export PATH=/opt/homebrew/bin:$PATH && bun vitest run src/components/FolderPickerField.test.tsx`
Expected: FAIL — module not found.

- [ ] **Step 3: Write the component**

`src/components/FolderPickerField.tsx`:
```tsx
import { useState } from "react";
import { Button } from "./ui/Button";
import { pickFolder } from "../ipc/workspace";

export interface FolderPickerFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
}

/** Label + text input + a "Browse…" button that opens the native folder picker
 *  (A3). The picker returns an absolute path written into the field; typing a
 *  ~-path by hand still works (expanded backend-side). Reused for Root path and
 *  Target repo. */
export function FolderPickerField({
  label,
  value,
  onChange,
  placeholder,
  disabled,
}: FolderPickerFieldProps) {
  const [busy, setBusy] = useState(false);
  async function browse() {
    setBusy(true);
    try {
      const picked = await pickFolder();
      if (picked) onChange(picked);
    } finally {
      setBusy(false);
    }
  }
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span style={{ fontSize: 12, color: "var(--text-2)" }}>{label}</span>
      <div style={{ display: "flex", gap: 8 }}>
        <input
          aria-label={label}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          disabled={disabled || busy}
          style={{ flex: 1 }}
        />
        <Button onClick={browse} disabled={disabled || busy} aria-label={`Browse for ${label}`}>
          {busy ? "Browsing…" : "Browse…"}
        </Button>
      </div>
    </label>
  );
}
```

- [ ] **Step 4: Run the test**

Run: `export PATH=/opt/homebrew/bin:$PATH && bun vitest run src/components/FolderPickerField.test.tsx`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/components/FolderPickerField.tsx src/components/FolderPickerField.test.tsx
git commit -m "feat(ui): FolderPickerField — label + input + native Browse (mocked in tests)"
```

---

## Task 10: Wire FolderPickerField into the wizard Basics step (A3 + A5)

**Files:**
- Modify: `src/wizard/NewProjectWizard.tsx`

- [ ] **Step 1: Add a `targetRepo` state + use FolderPickerField for Root path**

Add `const [targetRepo, setTargetRepo] = useState("");` near the `root` state. Replace the bare Root path `<label><input …/></label>` with:
```tsx
<FolderPickerField label="Root path" value={root} onChange={setRoot} placeholder="~/projects/example" />
<FolderPickerField label="Target repo (optional)" value={targetRepo} onChange={setTargetRepo} placeholder="~/projects/your-repo" />
```
Import `FolderPickerField` from `../components/FolderPickerField`.

- [ ] **Step 2: Pass targetRepo through create()**

```tsx
const project = await createProjectFromDraft(name, root, { ...draft, name, description }, targetRepo.trim() || null);
```

- [ ] **Step 3: Update the wizard test if it asserts the Root input shape**

Run the wizard test first to see if the input selector still matches (FolderPickerField keeps `aria-label="Root path"`, so `getByLabelText("Root path")` still works). Fix any test that asserted a specific DOM structure.

- [ ] **Step 4: Run wizard tests + build**

Run: `export PATH=/opt/homebrew/bin:$PATH && bun vitest run src/wizard && bun run build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/NewProjectWizard.tsx
git commit -m "feat(wizard): FolderPickerField for Root path (A3) + Target repo (A5) in Basics"
```

---

## Task 11: Settings → Projects target-repo field (S1 surface)

**Files:**
- Modify: `src/components/SettingsView.tsx`
- Modify: `src/App.tsx` (wire the `onSetTargetRepo` prop + reload)

- [ ] **Step 1: Add `onSetTargetRepo` prop to SettingsViewProps**

```ts
  onSetTargetRepo: (projectId: string, targetRepo: string | null) => Promise<void>;
```

- [ ] **Step 2: Render a FolderPickerField per project in the Projects section**

Inside the per-project block, after the root_path line, add a per-row local-state editor. Simplest: a small inline component `ProjectTargetRepo` that seeds from `p.target_repo`, renders a `FolderPickerField` + a "save" `Button`:
```tsx
function ProjectTargetRepo({ project, onSave }: { project: Project; onSave: (path: string | null) => Promise<void> }) {
  const [val, setVal] = useState(project.target_repo ?? "");
  const [busy, setBusy] = useState(false);
  return (
    <div style={{ marginTop: 6, display: "flex", gap: 8, alignItems: "flex-end" }}>
      <div style={{ flex: 1 }}>
        <FolderPickerField label="Target repo" value={val} onChange={setVal} placeholder="~/projects/your-repo" disabled={busy} />
      </div>
      <Button disabled={busy} onClick={async () => { setBusy(true); try { await onSave(val.trim() || null); } finally { setBusy(false); } }}>save</Button>
    </div>
  );
}
```
Use it in the map: `<ProjectTargetRepo project={p} onSave={(path) => onSetTargetRepo(p.id, path)} />`. Import `FolderPickerField`.

- [ ] **Step 3: Wire in App.tsx**

Find where `<SettingsView … />` is rendered. Add:
```tsx
onSetTargetRepo={async (id, targetRepo) => { await workspaceSetTargetRepo(id, targetRepo); /* reload projects */ }}
```
Import `workspaceSetTargetRepo` from the workspace IPC; reuse the existing projects-reload path used by `onRemoveProject` (mirror it).

- [ ] **Step 4: Update SettingsView tests**

Run the SettingsView test; add an `onSetTargetRepo: vi.fn()` to its props fixture so it renders. Add an assertion that the Target repo field renders for a project.

- [ ] **Step 5: Run frontend tests + build**

Run: `export PATH=/opt/homebrew/bin:$PATH && bun vitest run && bun run build`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/components/SettingsView.tsx src/App.tsx
git commit -m "feat(settings): per-project Target repo field via FolderPickerField + workspace_set_target_repo"
```

---

## Task 12: Full verification

- [ ] **Step 1: cargo test**

Run: `cd src-tauri && cargo test --workspace`
Expected: all green (count ≈ prior + new).

- [ ] **Step 2: cargo check + clippy**

Run: `cd src-tauri && cargo check --workspace && cargo clippy --workspace -- -D warnings`
Expected: clean.

- [ ] **Step 3: vitest + build**

Run: `export PATH=/opt/homebrew/bin:$PATH && bun vitest run && bun run build`
Expected: all green.

---

## Task 13: DOMAIN.md language + backlog (vet F1)

**Files:**
- Modify: `DOMAIN.md`
- Modify: `docs/v1.1-backlog.md`

- [ ] **Step 1: Register A5 language in DOMAIN.md (vet F1)**

In `### Cross-context (the kernel)`:
- Extend **Project**: "…a root directory, an **optional target repo** (binds `${target_repo}`; A5), an active pipeline, and a registered set of pipelines."
- Extend **Path variables** `${target_repo}`: "…the repo a task targets; **defaults to the Project's `target_repo`** and is overridden per-task at inject (A5)."

- [ ] **Step 2: Mark A3 + A5 done in the backlog**

In `docs/v1.1-backlog.md`, change A3 and A5 `- [ ]`/`backlog` → `- [x]`/`done (tag plan-a3-a5-target-repo)` with a one-line summary each.

- [ ] **Step 3: Commit**

```bash
git add DOMAIN.md docs/v1.1-backlog.md
git commit -m "docs: register A5 target-repo language in DOMAIN.md; mark A3+A5 done"
```

## Self-review notes

- Spec coverage: A3 folder picker (Task 9 component, Task 10 wizard root); A5 project target_repo (Tasks 1-7 backend, Task 10 wizard field, Task 11 Settings). Migration 010 both lists + idempotency bump (Task 1). `tools()` includes both (Task 3). `${target_repo}` binding via PathVars at the worker loop with task-overrides-project (Task 4) + inject default (Task 5). Runtime stays ignorant (plain strings, Task 5/6). Dialog plugin contained (Tasks 7, 8).
- Type consistency: `effective_target_repo` used identically in pool.rs; `project_target_repo: Option<String>` on RuntimeState, `Option<PathBuf>` on PoolContext (converted once at the root); `target_repo: string | null` on TS Project; `pickFolder(): Promise<string | null>`.
- Live dialog is structural-only (can't run headless) — component tests mock `pickFolder()`.
