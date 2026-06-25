# Candidate LF20-14 — Worktree hygiene on killed-task re-queue (a killed `claude` leaves a dirty worktree the re-run inherits)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. The filesystem-side consequence of making exit/Stop a real kill: the resume half (LF20-05/06/07) re-queues a task whose **worktree** was left mid-write, but nothing repairs that worktree before the re-run.

## Location
- `src-tauri/app/src/lib.rs:1250`–`1253` — the worktree seam wired at the root: `handle.manage(workspace::worktree::WorktreeState { pool: pool.clone(), git: Arc::new(workspace::worktree::GitCli) })`. Worktree create/commit/reset semantics live in the `workspace` crate (`../workspace/src/`) — outside the app crate; read-only analysis here.
- The re-queue chain that re-runs the task: boot `release_orphaned_running` (`lib.rs:1288`) + `reconcile_occupancy` (LF20-06), and the in-process brake-off recovery (LF20-07). None of these touches the worktree.
- The runner that writes the worktree: `runners::claude_cli::ClaudeCliRunner` (`../runners/src/claude_cli.rs`), spawned via the killable spawner (LF20-02).

## Why it is a candidate
LF20-01..04 make exit/Stop SIGTERM→SIGKILL the in-flight `claude`. A `claude` invocation's whole purpose is to **mutate the worker worktree** (edits, staged changes, possibly a partial commit — `lib.rs:1191` documents "author name/email for worker worktree commits"). Kill it mid-write and the worktree is left in an **arbitrary partial state**: unstaged half-edits, a dirty index, maybe a started-but-incomplete commit.

The resume thesis (LF20-05/06/07) then **re-queues that task and re-runs it** — but every resume item reasons purely over the `tasks` rows + `stores` occupancy arithmetic. **None inspects or resets the worktree.** So the re-run starts `claude` on top of the previous attempt's debris:
- The model sees a half-applied prior diff as if it were the starting state → confused/compounding edits, or a build that no longer matches the task's premise.
- A partial commit from the killed attempt may already be in history; the re-run double-applies or conflicts.
- Worst case the worktree is left in a git state (e.g. mid-`merge`/`rebase`, lock file) that makes the re-run's own git operations fail outright.

This is a genuine, distinct gap: the delivery's recovery is **DB-state-centric** and silently assumes the worktree is fungible across a kill. It is not. LF20-07 flags the *task disposition* open question (failed vs re-queuable) but explicitly stops at the DB row; the worktree's physical state is unowned.

## Proposed change
Give the resume path a worktree-repair step paired with the task re-queue:
1. Confirm the worktree model against `../workspace/src/` (per-task vs per-run vs per-stage worktree; how commit-on-success works; whether a reset/`git checkout -- .` + `git clean` primitive already exists). This determines whether repair is "reset to the task's start ref" or "destroy + recreate."
2. When the recovery routine (boot LF20-06 / brake-off LF20-07) re-queues a killed/orphaned task, **reset its worktree to a clean, known-good baseline** (the ref the task was meant to start from) before the worker can re-claim it — so the re-run begins from the same state the original attempt did, not from its debris.
3. Decide (plan) the granularity: reset only the worktrees of re-queued tasks, vs a blanket reconcile of all the run's worktrees on boot. Keep `workspace` persistence/git-aware logic in the `workspace` crate; the root only invokes the repair in the same sequence it calls `reconcile_occupancy`.

## Tests
- Behavioral (no live `claude`): create a worktree, dirty it (write a file / stage a change) to simulate a killed mid-write attempt, run the recovery repair, assert the worktree is back to its baseline ref (clean status) before re-claim.
- Integration: extend LF20-08's assembled-loop test — after the kill, assert the re-queued task's worktree is reset, not dirty, when the run resumes.

## Dependencies / sequencing
- **Depends on** LF20-04 (kill-on-Stop) / LF20-03 (kill-on-exit) — the reason a worktree is ever left mid-write — and slots into the recovery sequence LF20-06/07 wires.
- **Pairs with** LF20-07's killed-task-disposition question: "is this task re-runnable?" and "is its worktree clean to re-run in?" are the two halves of one re-run-readiness decision.
- Cross-crate: worktree reset/commit semantics live in `workspace`; wired/sequenced at the app root alongside `reconcile_occupancy`.

## Out of scope
The kill primitive (LF20-01); the DB-side reconcile (LF20-05/06); preserving/surfacing the killed attempt's partial work in the UI (likely LF23); Windows.
