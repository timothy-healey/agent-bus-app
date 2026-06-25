# Spec — Per-work-item worktree isolation for implementers

*Design doc. Brainstormed 2026-06-25. Implements roadmap WT1+WT2. Gives each work-item that reaches a repo-mutating (implementer) stage its own isolated git worktree, so implementers never mutate the target repo's working checkout, disparate candidates never collide, and each candidate ends as its own reviewable/hand-off-able branch. Builds on the lifecycle chunk's `working_dir` resolution (`plan-lifecycle`) and the S2 worktree seam (`workspace::worktree`).*

## Why this exists

A run's generator fans out (loop-until-dry) into **many independent candidates** — one recent run produced 15 (LWV-A1, A2, …). Each flows through spec→plan→implement→code-review **on its own**. Today:
- Per-task git **worktree creation does not exist** (S2 honest note): `workspace::worktree::WorktreeGit` has only `list_porcelain`/`remove`, no `add`. The `worktrees/` dir is scaffolded empty and nothing populates it.
- The lifecycle chunk anchors a worker's `working_dir` to the **target repo** (`effective_target_repo`). So when the pipeline reaches the `implement` stage, implementers would `claude` **directly in the target repo's working tree** — mutating it, and piling every candidate's unrelated changes onto one checkout.

The seed template's implementers prompt already *says* "work in a SELF-CONTAINED git worktree … everything stays local," but the infrastructure to create one and point the worker at it is missing. This spec adds it, keyed to the **work-item** so each candidate is isolated from the user's checkout *and* from every other candidate.

## Decisions

1. **Granularity: per-work-item.** A worktree belongs to a candidate (its `item_key` within a run), created when the item reaches its first **implementer-role** stage, and **persists across that item's downstream stages** (code-review, any revise→implement loop). Not per-run (a run = many disparate candidates) and not per-single-task (the item's review must see the implement's tree).
2. **`Role::Implementer`** added to the `Role` enum (today only `Producer`/`Reviewer`). It marks the stage that *creates/enters* a worktree. The seed template's `implementers` team is tagged `Implementer`.
3. **Layout:** `<project_root>/worktrees/<run_id>/<item_key>/` on a branch `agent-bus/<run_id>/<item_key>`, created off the **target repo's current HEAD**. **Never pushed** (no remote wiring; the agent commits locally, the operator reviews/merges the branch out-of-band).
4. **Read-only earlier stages** (research/spec/plan + their reviews) keep using the target repo at HEAD — they haven't diverged anything, so no worktree.
5. **Threading:** the work-item carries a `worktree_path`; once an implementer stage creates it, **child tasks inherit it** so the item's code-review (a Reviewer) operates in the same tree. Resolution at invoke: `task.worktree_path` if set → use it; else if `team.role == Implementer` → create + record it; else → `effective_target_repo` (chunk-1 behavior).
6. **Runtime stays git-unaware.** The engine resolves the worktree through an **injected `WorktreeProvider` seam** (runtime trait; the app implements it over `workspace`'s `GitCli`; tests inject a fake) — mirroring the `Runner` injection. No `git` idiom crosses into `runtime`.
7. **Reset on resume (WT2):** when a killed/orphaned implementer task is re-queued (the lifecycle/lifecycle-hardening recovery paths), reset its worktree to the branch baseline (`git reset --hard` + `git clean -fd`) before re-run, so the re-run starts clean rather than on a half-written tree.

## Architecture

### `Role::Implementer` (pipeline model)
Add the variant to `pipeline/src/model.rs` `Role` (`#[serde(rename_all="lowercase")]` → `"implementer"`; additive, default still `Producer`, **no `SCHEMA_VERSION` bump**). The graph builder's role-aware edges treat it like a producer (no verdict). `seed_template.rs`'s `implementers` team sets `role = Role::Implementer`.

### `WorktreeProvider` seam (runtime trait, app impl)
```rust
// runtime — git-unaware
pub trait WorktreeProvider: Send + Sync {
    /// Idempotent: ensure a worktree for (run_id, item_key) off target_repo HEAD
    /// exists; return its absolute path. Branch agent-bus/<run>/<item_key>.
    fn ensure(&self, run_id: &str, item_key: &str, target_repo: &str) -> Result<String, String>;
    /// Reset a worktree to its branch baseline (discard a killed mid-write tree).
    fn reset(&self, worktree_path: &str) -> Result<(), String>;
}
```
`EngineContext` gains `worktree_provider: Option<Arc<dyn WorktreeProvider>>` (None in pure tests / topic-less runs ⇒ falls back to target-repo `working_dir`, preserving chunk-1 behavior).

The app provides `GitCliWorktreeProvider` over an extended `workspace::worktree::WorktreeGit`:
- Add `add(&self, repo, path, branch, base_ref)` to the `WorktreeGit` trait + `GitCli` (`git -C <repo> worktree add -b <branch> <path> <base_ref>`) + `reset(&self, worktree_path)` (`git -C <wt> reset --hard <upstream> && git -C <wt> clean -fd`) — fake in tests. Path-scope every created/reset path with the existing `worktree_under_root` guard (stays under `<project_root>/worktrees/`).

### `working_dir` resolution (engine `invoke`)
Replace chunk-1's `let working_dir = effective_repo.map(...)` with:
```
let working_dir = if let Some(p) = task.worktree_path.clone() {
    Some(p)                                   // inherited from an upstream implementer stage
} else if team.role == Role::Implementer {
    match (&ctx.worktree_provider, effective_repo) {
        (Some(wp), Some(repo)) => {
            let path = wp.ensure(&ctx.run_id, &task.item_key, &repo.to_string_lossy())?;
            // record on the task so downstream children inherit it
            ctx.tasks.set_worktree_path(&task.id, &path).await?;
            Some(path)
        }
        _ => effective_repo.map(|p| p.to_string_lossy().into_owned()), // no provider/repo ⇒ chunk-1 fallback
    }
} else {
    effective_repo.map(|p| p.to_string_lossy().into_owned())          // read-only stages
};
```
`item_key` is already on the work-item. The created worktree is the worker's cwd, so `claude` can write code there with no extra `--add-dir` (the artifact base from chunk-1 stays separate, in app-data).

### Threading to children
`Task` gains `worktree_path: Option<String>` (migration `014`, nullable, append-only; register in BOTH `app/src/lib.rs` migration lists + bump the `user_version` idempotency test). Everywhere `transform_once`/fork/join construct a **child** work-item from a parent (the `Task::work_item(...)` / child-build sites in `engine.rs`), copy `parent.worktree_path` onto the child. So once `implement` sets it, `code-review` and any revise→implement loop of the same item inherit the same tree. `TaskStore` gains `set_worktree_path(task_id, path)` (single-column UPDATE) + the column in its row read/write.

### Reset on resume (WT2)
The recovery paths that re-queue killed/orphaned implementer tasks (lifecycle boot `release_orphaned_running` + lifecycle-hardening's brake-off/crash recovery) call `worktree_provider.reset(path)` for each re-queued task that has a `worktree_path`, before the worker can re-claim it. Wired at the composition root alongside `reconcile_occupancy` (the root holds the provider; keep `runtime` git-unaware — it exposes which tasks were re-queued + their `worktree_path`, the root resets them). Scope the reset to re-queued implementer tasks only (don't clobber a cleanly-committed worktree).

## Components / files
- `pipeline/src/model.rs` — `Role::Implementer`; `pipeline/src/seed_template.rs` — tag the implementers team.
- `runtime/src/engine.rs` — `WorktreeProvider` trait; `EngineContext.worktree_provider`; `working_dir` resolution; copy `worktree_path` to children.
- `runtime/src/task_store.rs` — `worktree_path` column read/write + `set_worktree_path`.
- `runtime/src/api.rs` / `pipeline_activator.rs` — build/pass the provider into `EngineContext`.
- `workspace/src/worktree.rs` — `WorktreeGit::add` + `reset` on the trait + `GitCli` (+ `FakeGit` in tests).
- `app/src/lib.rs` — `GitCliWorktreeProvider` (app, over `workspace` `GitCli`); inject into the engine context build; wire reset-on-resume into the recovery path; migration `014` registration.
- New migration `app/migrations/014_task_worktree.sql`.

## Testing (no live `claude`, no real remote)
- **`Role::Implementer`** serializes `"implementer"` + round-trips; default stays `producer`.
- **`WorktreeGit::add`/`reset`** path-scoping: a path outside `<project_root>/worktrees/` is rejected and runs no git (extend the S2 `worktree_under_root` tests); the git idiom is exercised via `FakeGit` (records the add/reset calls), never a real repo.
- **`working_dir` resolution (engine, fake provider):** an `Implementer` task with no `worktree_path` → provider `ensure` called, `working_dir` == the returned path, and the path is persisted; an inherited `worktree_path` → used without a second `ensure`; a `Producer`/`Reviewer` task with no inherited path → `working_dir` == target repo (chunk-1 behavior, regression-guarded).
- **Threading:** a child built from a parent with `worktree_path` set inherits it (so a downstream reviewer shares the tree).
- **Reset on resume:** a re-queued implementer task with a `worktree_path` triggers `provider.reset(path)` exactly once before re-claim; a task with no `worktree_path` (read-only stage) triggers none.
- **No provider (pure runtime tests):** `worktree_provider = None` ⇒ implementer falls back to target-repo `working_dir` (no panic), so existing engine tests stay green.

## Out of scope / non-goals
- **Per-lane isolation within one item's parallel implement** — concurrent fork-lanes of the SAME item share that item's worktree and may conflict on the same files (inherent to parallel edits). Documented; per-lane sub-worktrees are a future option.
- **Auto-merge / push** — branches stay local; the operator handles merge/push out-of-band (the no-push constraint).
- **A real-`git` integration test** — the `WorktreeGit` git path is structural-only via `FakeGit` (same discipline as S2 / the spawner / keychain seams); the live `git worktree add` is exercised manually.
- **Surfacing the worktree/branch/diff in the UI** — beyond recording `worktree_path`; a diff viewer is a separate item (pairs with B2 lineage compare).
- **Cleanup policy** — S2's `list_worktrees`/`remove_worktree` already cover removal; this spec only creates + resets. (S2's per-`worktrees/` discovery now finds live-task worktrees too — a future enhancement can subtract in-flight tasks.)

## Relationship to other items
- Closes roadmap **WT1** (per-task worktree creation) + **WT2** (reset/hygiene on resume; candidates LF20-14/16).
- Builds on **`plan-lifecycle`** (`working_dir`, recovery paths) and pairs with **lifecycle-hardening LH8** (killed-task stays re-runnable — the reset operates on exactly those re-queued tasks).
- Uses the **S2** `workspace::worktree` seam (extended with `add`/`reset`).
