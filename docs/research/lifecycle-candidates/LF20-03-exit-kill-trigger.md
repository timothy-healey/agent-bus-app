# Candidate LF20-03 — App-exit kill trigger (`RunEvent::ExitRequested`)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers — Exit).

## Location
- `src-tauri/app/src/lib.rs:1542` — currently the builder tail is `.run(tauri::generate_context!())`.

## Why it is a candidate
The spec names **app exit** as the first of two kill triggers. Today quitting the app leaves in-flight `claude` invocations orphaned (still potentially writing the worktree). The Tauri `RunEvent::ExitRequested` hook is the documented place to call `registry.kill_all()` on shutdown. The current `.run(generate_context!())` form gives no event callback, so this is a required structural change.

## Proposed change
Switch the builder tail from:
```rust
.run(tauri::generate_context!())
.expect("error while running tauri application");
```
to the build-then-run form so the run loop has an event callback:
```rust
.build(tauri::generate_context!())
.expect("error while building tauri application")
.run(move |_app, event| {
    if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
        registry.kill_all();
    }
});
```
The shared `Arc<ProcessRegistry>` must be captured into the run-event closure (built in `setup`/at the root and cloned into the closure). SIGTERM→grace→SIGKILL semantics live in `kill_all` (LF20-01), so this item is just the trigger wiring.

## Tests
Composition-root / structural — the registry methods carry the tested behavior (LF20-01). Assert the handler matches `ExitRequested` and calls `kill_all` (structural).

## Dependencies / sequencing
- **Depends on** LF20-01 (`kill_all`) and benefits from LF20-02 (so there is something registered to kill).
- Touches only the builder tail in `lib.rs` — isolated from the brake-trigger item.
