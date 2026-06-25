# Candidate LF20-09 — Bounded app-exit kill (don't let quit pre-empt grace→SIGKILL, and don't hang)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers — Exit). Promotes backlog **LF20** (exit/brake doesn't stop in-flight `claude` subprocesses).

## Location
- `src-tauri/app/src/lib.rs:1542` — the builder tail. LF20-03 changes this to `.build(...).run(move |_app, event| { if matches!(event, RunEvent::ExitRequested { .. }) { registry.kill_all(); } })`.
- `src-tauri/app/src/process_registry.rs` (new in LF20-01) — `kill_all()` does `SIGTERM` → poll grace ~2–3s → `SIGKILL`.

## Why it is a candidate
This is the **timing/ordering correctness** of the exit path, and it is currently **unowned**:
- LF20-03 explicitly disclaims it — "SIGTERM→grace→SIGKILL semantics live in `kill_all` (LF20-01), so this item is just the trigger wiring."
- LF20-01 owns the kill *primitive* (the grace loop), not the *shutdown sequencing* around it.

The seam between them is a real defect surface. `RunEvent::ExitRequested` carries an `api` you can `prevent_exit()` on; if you do **not**, Tauri proceeds to tear the process down. Calling a `kill_all()` that blocks ~2–3 s straight from the exit-event callback has two failure modes:
1. **Re-orphaning** — if Tauri/the OS terminates the app process before `kill_all`'s grace→`SIGKILL` escalation finishes, the SIGTERM-ignoring children survive their parent's death and orphan anyway — defeating the exact headline of LF20 ("exit kills in-flight `claude`"). Because children are spawned in their **own process group** (LF20-02 `.process_group(0)`), they are *not* reaped by the app's group teardown, so this is not hypothetical.
2. **Hung quit** — conversely, a naive synchronous 2–3 s block on the exit thread makes the app feel frozen on every quit even when children exit promptly on SIGTERM.

Neither LF20-03 (trigger only) nor LF20-08 (hermetic integration test, no live Tauri run loop) exercises the assembled exit-shutdown ordering.

## Proposed change
Make the exit handler *bound* exit to kill completion rather than fire-and-pray. In the `RunEvent::ExitRequested` arm:
1. On the first `ExitRequested`, call `api.prevent_exit()` and kick `registry.kill_all()` (ideally early-returning the moment all groups have exited, so the common case adds ~0 ms, not the full grace).
2. After `kill_all()` returns, request exit again (`app.exit(0)` / re-emit) so the second pass tears down cleanly.
3. Guard against re-entrancy (a flag) so the prevent/kill/re-exit dance runs exactly once.

Open design question for the plan: whether to make `kill_all()` return promptly once every group is confirmed dead (poll-to-exit, capped by the grace bound) so quit is fast in the normal case; and whether the bound should be a hard ceiling (e.g. escalate to `SIGKILL` immediately at the cap) to guarantee quit never hangs longer than the grace. Settle the prevent_exit-vs-block shape alongside LF20-03's wiring.

## Tests
Composition-root / structural — the grace-loop arithmetic is LF20-01's. Assert: the handler calls `prevent_exit` then `kill_all` then re-exits, exactly once (re-entrancy guard holds on repeated `ExitRequested`). A behavioral check (real short-lived child in its own group + a SIGTERM-ignoring child) that the process is confirmed dead *before* the second exit request is the strongest signal but needs a live run loop — note if deferred.

## Dependencies / sequencing
- **Depends on** LF20-01 (`kill_all`, the grace primitive) and LF20-03 (the `ExitRequested` wiring this refines).
- **Pairs with** LF20-02 (children in their own process group — the reason the OS won't reap them on app death).
- Independent of the Stop/brake path (LF20-04) and the resume half (LF20-05..07).

## Out of scope
The kill primitive's signal/grace mechanics (LF20-01); Windows shutdown (non-unix `kill_all` is a logged no-op per LF20-01); surfacing the interrupted state in the UI (separate backlog item, LF23).
