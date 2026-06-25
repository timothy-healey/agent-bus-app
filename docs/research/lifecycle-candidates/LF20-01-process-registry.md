# Candidate LF20-01 — `ProcessRegistry` (kill-handle registry)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. Promotes backlog **LF20** (exit/brake doesn't stop in-flight `claude` subprocesses).

## Location
- **New file** in the app (composition-root) crate, e.g. `src-tauri/app/src/process_registry.rs`, declared with `mod process_registry;` in `src-tauri/app/src/lib.rs:1` (next to `mod events; mod pipeline_activator;`).

## Why it is a candidate
The spec's Architecture section names `ProcessRegistry` as the first new component. Today there is **no kill handle** for in-flight `claude` invocations: workers spawn via blocking `std::process::Command::output()` (no `Child`, no pgid), so the process orphans on quit and Stop can't halt it. A shared registry of live process-group ids is the foundation every other lifecycle work-item (spawners, exit trigger, Stop trigger) depends on.

## Proposed change
Add `ProcessRegistry`: a shared `Arc<ProcessRegistry>` holding the live child **process-group ids** behind a `Mutex`/lock. `#[cfg(unix)]` methods:
- `register(pgid)` / `deregister(pgid)`.
- `kill_all()` — `kill(-pgid, SIGTERM)` for every registered group; wait a short grace (~2–3s, polling for exit); `kill(-pgid, SIGKILL)` for any survivor. Idempotent + best-effort (a gone pid is fine). Use `nix` (or `libc`) `kill`; negative pid targets the whole group so `claude`'s own child tools/subagents die too.
- **Non-unix:** a no-op with a logged warning (the documented Windows gap — job objects / `taskkill /T` out of scope).

No process types cross the `Runner`/`ChatRunner` traits — the registry is a pure composition-root utility.

## Tests (no live `claude`)
- Spawn a real `sleep 30` in its own group **with a child process** (so the *group* matters), `register`, `kill_all`; assert the process **and its child** are dead.
- A SIGTERM-ignoring child still dies after the grace via SIGKILL.

## Dependencies / sequencing
- **Blocks** LF20-02 (spawners register/deregister here), LF20-03 (exit calls `kill_all`), LF20-04 (Stop calls `kill_all`).
- New crate dep likely required: `nix` (or `libc`) in `src-tauri/app/Cargo.toml`.

## Out of scope
Per-task selective kill (Stop kills ALL); Windows kill.
