# Spec — App-managed project home: keep the target repo clean

*Design doc. Brainstormed 2026-10-05. A new project's root (its `pipelines/ prompts/ .agent-bus/ worktrees/` scaffold) defaults to the app-data dir instead of a folder you pick, and every worktree git command runs against the **target repo** rather than the project root. The target repo only ever gains `agent-bus/<run>/<item>` branches.*

## Why this exists

Pointing a project at a real codebase today puts the whole scaffold inside it. `create_project_from_draft` requires a root, `project_subdirs()` scaffolds five directories under it, and `GitCliWorktreeProvider::ensure` runs `git -C <project_root> worktree add`, branching off the **root's** HEAD and ignoring the `target_repo` it is handed. The A5 split between project root and target repo exists in the data model, but worktree creation still assumes they are the same folder.

The cost in the target repo: untracked `pipelines/ prompts/ .agent-bus/ worktrees/` in `git status`, and nested worktree copies that Metro, `tsc` and jest crawl. The first real use, a watched DDD run on `meal-planner`, would hit both.

Artifacts already moved to `<app_data>/projects/<id>/artifacts` (LF26). This finishes the move for everything else.

## Decisions

1. **Default root is app-managed.** `create_project_from_draft` takes `root: Option<String>`. Empty or `None` resolves to `app_data_project_dir(app_data, id)` = `<app_data>/projects/<id>/`, the directory that already holds `artifacts/`. A supplied root keeps today's behaviour.
2. **Target repo is required when the root is app-managed**, and must be a git repository (`git -C <target> rev-parse --git-dir`). With a custom root it stays optional, as today.
3. **Worktree git runs against the effective target repo.** `ensure(run_id, item_key, target_repo)` uses its `target_repo` argument (already the per-task effective repo, A5 inject override included) for `git -C <repo> worktree add`, branching off that repo's `HEAD`. An empty `target_repo` falls back to the project root, which preserves the old behaviour for root-is-repo projects.
4. **The path guard does not loosen.** Every worktree *path* must still resolve under `<project_root>/worktrees/` (`worktree_under_root`), or no git command runs. Only the repo the command runs *in* changes.
5. **Branches are left in the target repo** on project delete. Worktrees are torn down; `agent-bus/…` branches stay for the operator to review or delete.
6. **No migration.** Existing projects keep their stored root. The app-managed default applies to new projects only.

## Architecture

### Workspace context (`workspace` crate)

**`worktree.rs`.** Split the "which repo" and "which subtree" roles that `project_root` currently plays:

```rust
pub fn list_worktrees_inner(git: &dyn WorktreeGit, repo: &str, project_root: &str)
    -> Result<Vec<WorktreeEntry>, String>;          // git -C repo; filter to <project_root>/worktrees/
pub fn remove_worktree_inner(git: &dyn WorktreeGit, repo: &str, project_root: &str, path: &str)
    -> Result<(), String>;                           // guard on project_root, git -C repo
pub fn add_worktree_inner(git: &dyn WorktreeGit, repo: &str, project_root: &str, path: &str,
    branch: &str, base_ref: &str) -> Result<(), String>;
```

`reset_worktree_inner` already runs `git -C <worktree>` and keeps its signature. `WorktreeGit` already takes `repo` on `list_porcelain`/`add`/`remove`, so the trait is unchanged.

**`cleanup_project_files_inner(git, root_path, target_repo)`.** Teardown lists and removes via `repo = target_repo.unwrap_or(root_path)`. The collision guard (skip everything when the target is the root or under it) is unchanged.

**New helper:** `pub fn resolve_project_root(app_data: &Path, project_id: &str, root: Option<&str>, home: &str) -> PathBuf`. Pure: a non-empty `root` is tilde-expanded, otherwise `app_data_project_dir(app_data, project_id)`.

**New check:** `WorktreeGit::is_repo(&self, path) -> bool` (`git -C <path> rev-parse --git-dir`), so the creation check is fake-able in tests.

### Composition root (`app` crate)

**`GitCliWorktreeProvider::ensure`** uses the `target_repo` argument: `repo = if target_repo.trim().is_empty() { project_root } else { target_repo }`. Then `add_worktree_inner(git, repo, project_root, path, branch, "HEAD")`. Path derivation (`<project_root>/worktrees/<run>/<item>`) is unchanged.

**`create_project_from_draft_inner(ws, git: &dyn WorktreeGit, app_data, name, root: Option<String>, draft, target_repo)`.** `Project::new` mints the id, so the root is resolved after the id exists: build the project, set `root_path = resolve_project_root(...)`, then insert. Validation order, before anything is written:
1. pipeline hard-validate (unchanged);
2. app-managed root with no target repo → `Err("a target repo is required when the project lives in app data")`;
3. target repo given and `!git.is_repo(target)` → `Err("target repo is not a git repository: <path>")`.

The `create_project_from_draft` Tauri command passes `app_data_dir()` and the app's `GitCli` through.

### Frontend

**`NewProjectWizard` Basics.** Fields in order: Name, Description, **Target repo** (folder picker, required by default), then a collapsed **"Custom location"** disclosure that reveals the existing Root path picker. The Generate and Create guards change from `!root.trim()` to: target repo required unless a custom root is set.

**`createProjectFromDraft(name, root | null, draft, targetRepo | null)`.** `root` is `null` when the disclosure is closed or empty.

**`ReviewStep`** shows "Location: app-managed" or the custom path, plus the target repo.

## Data flow (new project, default path)

```
Wizard (name, target=~/…/meal-planner, root=null)
  → create_project_from_draft(root=None)
      validate pipeline → target present → is_repo(target) ✓
      Project::new → id → root_path = <app_data>/projects/<id>
      insert → write pipelines/ + prompts/ under root → activate
Run → implementer item
  → ensure(run, item, target_repo=~/…/meal-planner)
      path = <app_data>/projects/<id>/worktrees/<run>/<item>   (guard ✓)
      git -C ~/…/meal-planner worktree add -b agent-bus/<run>/<item> <path> HEAD
```

## Error handling

| Case | Behaviour |
|---|---|
| App-managed root, no target | Create rejected with an inline wizard error; nothing written |
| Target not a git repo | Create rejected: "target repo is not a git repository: <path>" |
| Target deleted or moved after create | `worktree add` fails, the item errors with git's message (existing error path; no new handling) |
| Worktree path outside `<root>/worktrees/` | Guard rejects; no git runs (unchanged) |
| Delete project, target removed meanwhile | Worktree list fails and is noted (tolerant, unchanged); app-data dir is still removed |

## Testing

**Workspace (fake `WorktreeGit` recording `(op, repo, path)`):**
- `add_worktree_inner` runs git in `repo`, guards on `project_root`; a path outside `project_root/worktrees` runs no git.
- `list_worktrees_inner` queries `repo`, keeps only entries under `project_root/worktrees`.
- Cleanup with an app-managed root tears down via the target repo; the collision guard still skips when the target is under the root.
- `resolve_project_root`: empty → app-data dir; `~/x` → expanded; absolute → as-is.

**App:**
- `ensure` with a target repo → git ran in the target, path under the project root.
- `ensure` with an empty target → falls back to the project root.
- `create_project_from_draft_inner`: `None` root → `root_path == <app_data>/projects/<id>`; `None` root and no target → error, no row; non-repo target → error, no row; custom root → unchanged.

**Frontend:**
- Basics: Create is disabled without a target repo by default; opening Custom location and entering a root makes the target optional.
- `createProjectFromDraft` is called with `root: null` when the disclosure is closed.

## Out of scope

- Moving existing projects' roots into app data.
- Deleting `agent-bus/…` branches on project delete.
- Listing worktrees across several per-task target repos for one project: cleanup uses the project's own target repo.
