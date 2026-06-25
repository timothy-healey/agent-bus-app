# Candidate LF20-14 — Don't block the async executor with `kill_all`'s grace on the Stop path

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers — Stop). This is the **Stop-path** analogue of the timing concern LF20-09 owns only for the **exit** path.

## Location (the three Stop-path call sites, all in async contexts)
- `src-tauri/app/src/lib.rs:1067`–`1074` — the `RootDispatcher` `"brake_on"` arm. Dispatch runs inside an **`async fn`** (the surrounding arms `.await`, e.g. `:1057`–`:1060`). LF20-04 adds `registry.kill_all()` here.
- `src-tauri/app/src/lib.rs:1527` — `runtime::api::brake_on` in `invoke_handler` (or the thin root wrapper LF20-04 proposes) — a Tauri **async command**.
- `src-tauri/app/src/lib.rs:1461`–`1479` — the auto-meter sweep, a **`tauri::async_runtime::spawn`** task; LF20-04 adds `kill_all()` at the `SetOn` branch (`:1473`). (Note: LF20-11 may exempt this site from killing — but if it kills, it kills here.)

## Why it is a candidate
`kill_all()` (LF20-01) is a **synchronous, blocking** routine: SIGTERM → poll the grace window (~2–3 s) → SIGKILL. LF20-09 reasons carefully about not letting that block hang the **exit** event callback — but the exit callback runs on Tauri's **run-event loop**, not the async executor. The **Stop** path is the opposite environment: all three brake-on sites above execute **inside the Tokio/async runtime**. Calling a 2–3 s blocking `kill_all()` directly from an `async fn` / spawned task **blocks the executor thread** for the whole grace window:
- The dispatch path (`:1067`) and the `brake_on` command (`:1527`) make the **Stop button hang** for up to the grace bound, and can stall **other** in-flight async commands sharing that worker thread.
- The auto-meter task (`:1461`) blocks its executor thread on every trip.

LF20-09 explicitly scopes itself to the exit path and disclaims the Stop path ("Independent of the Stop/brake path (LF20-04)"). LF20-04 frames itself as "just the trigger wiring" and pushes grace mechanics down to LF20-01. So the **async-safety of invoking the blocking kill from Stop's async sites is unowned** — a real defect surface (UI freeze on Stop; executor starvation).

## Proposed change
Invoke the blocking `kill_all()` off the async executor at each Stop-path site:
- Wrap the call in `tokio::task::spawn_blocking(move || registry.kill_all())` (or `tauri::async_runtime::spawn_blocking`) and `.await` the handle where the caller wants Stop to be synchronous; or fire-and-forget where it should not block the UI response.
- Alternatively, give the registry an `async fn kill_all_async(&self)` that uses async sleeps for the grace poll (no blocking), and call that from the async sites; keep the sync `kill_all()` for the exit-event callback (LF20-09).
- Decide per site whether Stop should **await** kill completion (button reflects "stopped" only once work is dead) or return immediately and let the kill finish in the background. Recommend: the user-facing Stop awaits a **bounded** kill (reuse LF20-09's "return promptly once all groups confirmed dead, hard ceiling at the grace bound") so the UI is both responsive and truthful.

## Tests
- Structural: each Stop-path site invokes the non-blocking/`spawn_blocking` form, not a bare synchronous `kill_all()` on the executor thread.
- Behavioral (optional, with a real short-lived child): the `brake_on` async command returns within the bounded grace and the child is dead; a second concurrent async command is **not** starved during the kill.

## Dependencies / sequencing
- **Depends on** LF20-01 (the blocking `kill_all` primitive) and LF20-04 (the Stop-path wiring this makes async-safe).
- **Mirrors** LF20-09 (exit-path bounding) on the Stop path — together they make *both* triggers neither hang nor starve.
- **Composes with** LF20-11 (which may remove the auto-meter site as a kill site) and LF20-13 (scope) — independent of both.

## Out of scope
The grace/signal mechanics (LF20-01); the exit-path callback (LF20-09); Windows.
