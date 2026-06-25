# Candidate LF20-15 — Reap crash-orphaned `claude` processes on boot (the in-memory registry can't kill what an app *crash* left behind)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers / Boot reconciliation). The blind spot of the *clean*-exit kill: an **unclean** app death (crash, `SIGKILL`, OOM, power loss) bypasses every kill trigger, and the resume path re-runs work while the old children are still alive.

## Location
- `src-tauri/app/src/process_registry.rs` (new in LF20-01) — the registry is "a shared `Arc<ProcessRegistry>` holding the live child process-group ids behind a `Mutex`" — **purely in-memory**, reconstructed empty on every launch.
- `src-tauri/app/src/lib.rs` (LF20-02 spawner) — children are spawned with `.process_group(0)` so each `claude` (and its tool/subagent children) leads its **own** process group. That is precisely what stops the OS from reaping them when the app's own process dies.
- Boot recovery: `src-tauri/app/src/lib.rs:1288` `release_orphaned_running(now_unix())` → `reconcile_occupancy` (LF20-06), which **re-queues** the interrupted `running` tasks so the worker loops re-run them after `activate(&project_id)` (`:1448`).

## Why it is a candidate
LF20-03/09 kill in-flight `claude` on a **clean** exit (`RunEvent::ExitRequested`). But the kill handles live **only in RAM**. If the app dies **without** running its exit handler — a panic, an OS `SIGKILL`, an OOM, a crash, a hard power-off — then:
1. `kill_all()` never fires, so the in-flight `claude` children survive.
2. Because LF20-02 puts each in its **own process group**, they are **not** reaped by the app's process-group teardown — they keep running, still writing the worktree.
3. The next launch builds a **fresh, empty** `ProcessRegistry` (it cannot know the previous session's pgids — they were never persisted), then boot recovery (LF20-06) re-queues those same `running` tasks and the worker loops **re-run them**.

Result: **two `claude` invocations on the same task/worktree at once** — last session's surviving orphan and this session's re-run — racing on the same files. This is the exact "exit kills in-flight `claude`" headline of LF20, but for the failure mode the clean-exit path structurally cannot cover, and it actively collides with the resume path the delivery is building.

This is distinct from every existing item: LF20-09 bounds the **clean** shutdown so children die *before* exit; it assumes the handler runs. LF20-10 persists **brake** state, not process identity. Nothing persists or re-discovers live child pgids across the process boundary, and nothing reaps a previous session's survivors before recovery re-runs their tasks.

## Proposed change
Make the in-flight set survive an unclean death and reap it before recovery:
1. **Persist liveness**, not just register in RAM: when the spawner (LF20-02) registers a pgid, also write a durable record — `(run_id, task_id, pgid, started_ts)` in a small `live_processes` store row (or a pid file); deregister deletes it. The runtime `Brake`/store stay unaware — this is a composition-root record, mirroring LF20-10's persisted-brake pattern.
2. **Reap on boot, before recovery**: at startup (`lib.rs:1285`/`:1288`), *before* `release_orphaned_running` + `reconcile_occupancy`, read the persisted live-process records left by the prior session and `kill_all`-style reap any that are still alive (validate the pgid still maps to *our* process — guard against PID reuse — then SIGTERM→grace→SIGKILL via the LF20-01 primitive). Clear the records. Only then run the re-queue/reconcile so the re-run starts with no surviving competitor.
3. Decide (plan) the PID-reuse guard (store a start-time / cmdline fingerprint alongside the pgid so a recycled pid isn't signalled) and whether reaping is best-effort-and-log vs blocking boot.

## Tests
- Behavioral (no live `claude`): seed a persisted live-process record pointing at a real `sleep`-in-its-own-group child (simulating last session's orphan), run the boot reap, assert the child is dead and the record cleared *before* recovery re-queues the task.
- PID-reuse guard: a record whose pgid now maps to an unrelated/foreign process is **not** signalled.
- Negative: a normal clean exit (LF20-03/09 ran) leaves no stale records, so boot reap is a no-op.

## Dependencies / sequencing
- **Depends on** LF20-01 (the SIGTERM→grace→SIGKILL primitive it reuses) and LF20-02 (the spawner that would now also persist the record).
- **Sequences before** LF20-06 boot recovery (reap survivors → *then* re-queue/reconcile) — the ordering is the whole point.
- **Complements, does not duplicate** LF20-09 (clean-exit bounding): LF20-09 covers the handler-ran path; this covers the handler-didn't-run path.
- **Pairs with** LF20-14 (a reaped orphan also left a dirty worktree to repair).

## Out of scope
The clean-exit trigger/bounding (LF20-03/09); the kill primitive's signal mechanics (LF20-01); Windows (the persisted-record + reap is a Unix pgid story; non-unix stays the logged no-op per LF20-01); UI surfacing of "recovered after a crash."
