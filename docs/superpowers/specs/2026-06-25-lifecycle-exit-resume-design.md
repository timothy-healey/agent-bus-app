# Spec — Runtime lifecycle: clean exit + clean resume

*Design doc. Brainstormed 2026-06-25. Promotes backlog **LF20** (exit/brake doesn't stop in-flight `claude` subprocesses) and the **resume gap** it exposes (store-occupancy leak on a crash/kill). Two halves of one thing: killing in-flight work on exit/Stop, and reconciling runtime state on reopen so a run picks up cleanly.*

## Why this exists

Today the app has no graceful shutdown: worker loops die with the process (fine — no new claims), but an in-flight `claude` invocation is spawned via **blocking `std::process::Command::output()`** with no kill handle / process group, so it **orphans** and keeps running after quit — potentially still writing the worktree (implementers mutate the target repo). The brake (Stop) only blocks new claims; it doesn't kill a running invocation. And on reopen, `release_orphaned_running` re-queues interrupted task rows, but **nothing rebuilds store occupancy** — a slot reserved before a kill (`occupancy += 1`) is never committed or released, so the store stays **falsely full → backpressure → the run stalls**. Killing on exit/Stop actually *creates* that leak, so the two must ship together.

## Decisions (from the brainstorm)

1. **Killable spawns** via the existing `with_spawner` ACL seam: `.spawn()` the child **in its own process group**, track it in a shared registry, and kill the *group* (so `claude`'s own child tools/subagents die too). No process types cross the `Runner`/`ChatRunner` traits.
2. **Two kill triggers:** **app exit** (Tauri `RunEvent::ExitRequested`) AND **Stop** (the brake — `brake_on` + the auto-meter sweep that trips it). Stop now actually halts running work.
3. **Politeness:** **SIGTERM → ~2–3s grace → SIGKILL** the group.
4. **Clean resume:** on boot/activation, after `release_orphaned_running` (exists), **rebuild each store's occupancy from the resident work-items** (clears leaked reservations) so the resumed run isn't falsely backpressured.
5. **Platform:** Unix (macOS + Linux) via process groups + signals; Windows is a documented gap (job objects / `taskkill /T`) — dev + target is macOS.

## Architecture

### `ProcessRegistry` (composition-root)
A shared `Arc<ProcessRegistry>` holding the live child **process-group ids** (the spawner registers a pgid on spawn, removes it on completion). `#[cfg(unix)]` methods:
- `register(pgid)` / `deregister(pgid)`.
- `kill_all()` — `kill(-pgid, SIGTERM)` for every registered group; wait a short grace (~2–3s, polling for exit); `kill(-pgid, SIGKILL)` for any survivor. Idempotent + best-effort (a gone pid is fine). Uses `nix` (or `libc`) `kill`.
- On non-unix: a no-op with a logged warning (the Windows gap).

### Killable spawner (the seam)
The composition root builds the worker `ClaudeCliRunner::with_spawner(...)` AND the chat `ClaudeChatRunner::with_spawner(...)` with a spawner closure capturing `registry.clone()` that:
1. Builds the `Command`, sets `.process_group(0)` (Unix `CommandExt`) so the child leads a fresh group (pgid == child pid), pipes stdout/stderr.
2. `.spawn()` → `Child`; `registry.register(child.id())`.
3. Reads stdout/stderr to completion and `.wait()`s (replicating `.output()` capture; the streaming runner keeps forwarding deltas as today).
4. `registry.deregister(pgid)`; returns the same `interpret_runner_output(stdout, stderr, success)` as the current closure.
`ClaudeCliRunner::new()`'s plain blocking `.output()` stays for tests/non-app use. The ACL is unchanged — only the root's spawner closure differs.

### Kill triggers
- **Exit:** `tauri::Builder…build()?.run(move |_app, event| { if matches!(event, RunEvent::ExitRequested { .. }) { registry.kill_all(); } })`.
- **Stop:** every brake-on site at the composition root (`brake_on` command + the auto-meter sweep that trips the brake) calls `registry.kill_all()` after setting the brake. (Route `brake_on` through a root wrapper, or give the brake an on-set hook — the plan picks the cleanest; the runtime `Brake` aggregate itself stays registry-unaware.)

### Boot reconciliation (resume)
At boot/activation, in order:
1. `TaskStore::release_orphaned_running` (exists) — `running` → `queued`.
2. **`StoreRepo::reconcile_occupancy(run_id)`** (new) — for each store of the run, set `occupancy` = the count of work-items resident at that stage (truth from `tasks`, using the engine's residency states — items parked awaiting a worker). This clears reservations leaked by a killed/crashed worker. Then the worker loops + the resumed run drive normally (the active run, generator-dry flag, and found-key ledger all persist).

## Components / files
- New: `ProcessRegistry` (app crate — composition root; or a small `runtime`/util module it can live in, but it's wired at the root). `StoreRepo::reconcile_occupancy`.
- Modified: `app/src/lib.rs` (build the killable spawners; the `RunEvent::ExitRequested` handler; brake-on → `kill_all`; call `reconcile_occupancy` in the boot/activation recovery path next to `release_orphaned_running`). `pipeline_activator.rs` if activation is where recovery runs.
- Unchanged ACL: `runners`/`llm_chat` traits; `ClaudeCliRunner`/`ClaudeChatRunner` keep `with_spawner`.

## Testing (no live `claude`)
- **ProcessRegistry (unix):** spawn a real `sleep 30` in its own group (with a child process so the *group* matters), `register`, `kill_all`; assert the process **and its child** are dead; a SIGTERM-ignoring child still dies after the grace via SIGKILL.
- **Killable spawner:** a real `echo hello` (or `sh -c`) through the spawner returns the captured output and registers-then-deregisters (registry empty afterward).
- **`reconcile_occupancy`:** seed a store row with inflated `occupancy` + N resident task rows at that stage; reconcile; assert `occupancy == N`. Plus: a leaked reservation (occupancy=2, zero resident) reconciles to 0.
- **Wiring** (exit handler / brake → kill_all): composition-root, structural (the registry methods carry the tested behavior).

## Out of scope / non-goals
- Windows kill (job objects / `taskkill /T`) — documented gap; Unix only now.
- Per-task selective kill (Stop kills ALL running, per the decision) — no per-card kill button in v1.
- Resuming a *partially-produced* artifact mid-invocation — a killed invocation's item is simply re-run from scratch (idempotent re-claim); no checkpoint/restore of in-flight model output.

## Relationship to other items
- Closes **LF20** + the resume-occupancy gap it exposed.
- Builds on **R** (the engine's reserve/commit/take store semantics) and the existing `release_orphaned_running` (crash recovery, F4).
