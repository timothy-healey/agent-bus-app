# Candidate LF20-16 — Reset the worker worktree left dirty by a mid-write kill before re-run

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. The filesystem-side consistency the kill triggers create but the task/store reconcile (LF20-05/06/07) never addresses: the worktree a killed `claude` was mid-write in.

## Location
- `src-tauri/app/src/lib.rs:1251`–`1253` — `handle.manage(workspace::worktree::WorktreeState { … git: Arc::new(workspace::worktree::GitCli), … })`: the app manages per-worker git worktrees at the composition root.
- `src-tauri/app/src/lib.rs:1191` — `"git config — author name/email for worker worktree commits"`: workers **commit into worktrees** as they settle.
- `src-tauri/app/src/lib.rs:1493`–`1494` — `workspace::worktree::list_worktrees` / `remove_worktree` (the existing worktree command surface).
- Boot recovery: `src-tauri/app/src/lib.rs:1288` `release_orphaned_running` (+ `reconcile_occupancy`, LF20-06) — the resume path that re-queues a killed task **without touching its worktree**.

## Why it is a candidate
The stated *motivation* for killing on exit/Stop is that an in-flight `claude` is "still potentially writing the worktree" (LF20-03 candidate, line 10). LF20-01..04 stop the process; LF20-05/06/07 re-queue the task row and rebuild store occupancy. **Nothing reverts the half-written worktree.** A `claude` SIGKILLed mid-edit leaves the worker's git worktree with **uncommitted, partial changes** (a half-applied diff, a dangling lock, an unfinished commit). When recovery re-queues the task and a worker re-runs it, the new `claude` starts in a **dirty worktree** — it sees partial prior output, may double-apply, conflict, or commit a corrupt mixture. "Clean resume" of the task state is undermined by an unclean filesystem.

This is a genuinely distinct gap: every existing item reasons about `tasks` rows, `stores` occupancy, brake state, or the chat transcript (LF20-11) — none touches the git worktree, which is the actual artifact the kill was meant to protect. It is the filesystem analogue of LF20-05's occupancy reconcile and LF20-11's chat-transcript reconcile.

## Proposed change
Give the re-run path a clean worktree, symmetric to the store/transcript reconciles:
1. **On the resume/re-run path:** before a re-queued task is re-claimed (or as part of boot recovery alongside `release_orphaned_running`/`reconcile_occupancy`), **reset the task's worktree to a clean baseline** — `git reset --hard` + `git clean -fd` (and clear any stale `index.lock`) via the existing `GitCli`/`WorktreeState` — so the re-run starts from the last committed state, not the killed partial.
2. **Decide the baseline (plan):** reset to the worktree's last *settled* commit, or recreate the worktree from `remove_worktree` + re-add. Confirm against `workspace::worktree` whether worktrees are per-task/per-team and whether commits are per-settled-step (so the reset target is well-defined).
3. **Scope to killed work:** only worktrees whose task was killed/re-queued need resetting — avoid clobbering worktrees that committed cleanly. Tie the reset to the same set of rows LF20-07/LF20-06 re-queue.

## Tests (no live `claude`)
- Behavioral: seed a worktree with uncommitted partial changes (simulating a mid-write kill) plus a re-queued task row; run the resume/reset path; assert the worktree is clean (`git status` empty, back at the last committed state) before re-claim.
- Idempotence/safety: a worktree with a clean settled commit is left untouched by the reset pass.

## Dependencies / sequencing
- **Depends on** LF20-01..04 (the kill that creates the dirty worktree) and pairs with LF20-06/07 (the re-queue paths the reset must precede).
- **Independent of** the chat path (LF20-11) and per-reason policy (LF20-12).
- Cross-module: uses the app's `workspace::worktree` surface; wired at the composition root where `WorktreeState` is managed and recovery runs.

## Out of scope
The kill primitive (LF20-01); the `tasks`/`stores` reconcile (LF20-05/06/07); worktree creation/teardown policy beyond the reset-before-re-run; merge/conflict resolution of *committed* work (a re-run produces fresh output from a clean base).
