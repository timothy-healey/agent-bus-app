# Candidate LF20-13 — Stop-path kill must not block the Tokio runtime (offload `kill_all`'s grace off the async threads)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers — Stop). The Stop-path analogue of LF20-09 — which bounds only the **exit** thread and explicitly disclaims the brake path.

## Location
- `kill_all` (LF20-01, new `src-tauri/app/src/process_registry.rs`) is **synchronous**: SIGTERM → poll a ~2–3 s grace (sleeps) → SIGKILL.
- LF20-04 calls it at the **Stop sites, which run inside async contexts**:
  - `src-tauri/app/src/lib.rs:1067`/`:1070` — RootDispatcher `"brake_on"` arm, inside the **async** `dispatch(...)` (the agentic/terminal turn that issued Stop).
  - `src-tauri/app/src/lib.rs:1473` — the auto-meter sweep, inside `tauri::async_runtime::spawn(async move { … })` (`:1461`).
  - `src-tauri/app/src/lib.rs:1527` — the frontend `runtime::api::brake_on` (an **async** Tauri command).

## Why it is a candidate
LF20-09 owns the **exit** path's shutdown bounding and is explicit: "Independent of the Stop/brake path (LF20-04)." LF20-04 is "just the trigger wiring" and defers grace mechanics to LF20-01. So the behavior of calling a **blocking, multi-second** `kill_all` **from a Tokio worker thread** at the Stop sites is **unowned** — and it's a real defect surface:
- The grace loop in `kill_all` sleeps ~2–3 s synchronously. Invoked directly inside the async `dispatch` (`:1067`) or the async sweep (`:1473`), it **blocks a Tokio runtime worker thread** for the whole grace window. That stalls the very chat turn that pressed Stop, plus any other futures multiplexed on that thread — the app feels frozen on Stop, and on a small runtime can briefly starve all async work.
- This is the Stop-path twin of LF20-09's "hung quit," but in the async runtime rather than the exit thread — and LF20-09 explicitly scopes itself out of it.

## Proposed change
Keep `kill_all`'s blocking grace semantics (LF20-01) but never run them **on** a Tokio thread at the Stop sites:
1. At the async Stop sites, offload the kill: `tokio::task::spawn_blocking(move || registry.kill_all())` (or expose an `async fn kill_all_async` that internally `spawn_blocking`s the grace loop), so the async dispatch / sweep returns promptly and the brake-on UI feedback is immediate.
2. Decide (plan) whether Stop should **await** kill completion before reporting "stopped" (await the `spawn_blocking` handle) or fire-and-forget with the brake already visibly on. Recommend: brake flips on immediately (blocks new claims now), kill completes on the blocking pool — Stop never freezes the runtime.
3. Reconcile with LF20-09's exit path: exit runs on the sync `RunEvent` thread (blocking is acceptable there but must be **bounded** per LF20-09); the Stop path runs on Tokio and must be **offloaded** per this item. One `kill_all` primitive, two call-site disciplines.

## Tests
- Structural: the Stop sites invoke `kill_all` via `spawn_blocking` / the async wrapper, not inline on the async thread.
- Behavioral (if feasible without a live runtime): a Stop trigger with a registered slow-to-die child returns/reports promptly while the kill proceeds on the blocking pool.

## Dependencies / sequencing
- **Depends on** LF20-01 (`kill_all` grace primitive) and LF20-04 (the Stop-site wiring this refines).
- **Mirrors** LF20-09 (exit-path bounding) for the Stop path — the two together make *both* kill triggers non-hanging.
- **Independent of** the resume half (LF20-05/06/07).

## Out of scope
The kill primitive's signal/grace mechanics (LF20-01); the exit-thread bounding (LF20-09); Windows (`kill_all` is a logged no-op there per LF20-01).
