# Candidate LF20-04 — Stop (brake) kill trigger at every brake-on site

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers — Stop).

## Location (the three brake-on sites at the composition root)
- `src-tauri/app/src/lib.rs:1067` — `RootDispatcher` dispatch arm `"brake_on"` (the terminal / agentic tool path; `set_on(...)`).
- `src-tauri/app/src/lib.rs:1527` — `runtime::api::brake_on` registered directly in the `invoke_handler` (the frontend Stop button). Lives in the `runtime` crate and cannot reach the root-only registry as-is.
- `src-tauri/app/src/lib.rs:1473` — the auto-meter sweep: `BrakeDecision::SetOn(reason) => brake.set_on(reason)`.

## Why it is a candidate
The spec's decision 2 states "**Stop now actually halts running work**." Today the brake only blocks *new* claims; an already-running invocation keeps going. Every site that turns the brake on must also `registry.kill_all()` so Stop is a real halt. The runtime `Brake` aggregate itself must stay registry-unaware (no new cross-context edge), so the wiring is a root concern at each call site.

## Proposed change
After setting the brake at each of the three sites, call `registry.kill_all()`. The spec offers two clean shapes — pick one in the plan:
- **Root wrapper command:** replace `runtime::api::brake_on` in the `invoke_handler` with a thin root `brake_on` command that sets the brake (delegating to the runtime API) then calls `kill_all`; the RootDispatcher arm and the auto-meter `SetOn` branch call `kill_all` inline.
- **Brake on-set hook:** give `Brake` an optional `on_set` callback wired at the root to `kill_all`, so all three sites converge through one hook. (`Brake` stays registry-unaware — it just invokes an injected `Fn`.)

The auto-meter sweep already holds `brake` + `handle`; capture the registry there too (the closure at `lib.rs:1455`–`1480`).

## Tests
Composition-root / structural (registry behavior is tested in LF20-01). If the on-set-hook shape is chosen, unit-test that `set_on` invokes the injected hook exactly once and `set_off` does not.

## Dependencies / sequencing
- **Depends on** LF20-01 (`kill_all`) and LF20-02 (something to kill).
- Independent of LF20-03 (exit) — same `kill_all`, different trigger.
- Design choice (wrapper vs hook) should be settled in the plan; the frontend `brake_on` command path is the part that forces the decision.
