# Candidate LF20-12 — Per-reason brake kill policy (auto-meter budget cap should not hard-kill in-flight work)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers — Stop). The policy decision LF20-04 collapsed by treating all three brake-on sites identically.

## Location
- `src-tauri/app/src/lib.rs:1473` — auto-meter sweep: `BrakeDecision::SetOn(reason) => { brake.set_on(reason); … }` where `reason == AUTO_METER_REASON` (`usage_telemetry::brake_policy::AUTO_METER_REASON`).
- vs the **user** Stop sites: RootDispatcher `"brake_on"` (`:1067`/`:1070`, default reason `"manual"`) and the frontend `runtime::api::brake_on` (`:1527`).
- LF20-04 wires `registry.kill_all()` at **all three** brake-on sites uniformly.

## Why it is a candidate
LF20-04's premise — "every site that turns the brake on must also `kill_all`" — is correct for a **user** Stop (the operator wants work to halt *now*). It is questionable for the **auto-meter** trip, and LF20-04 never distinguishes them:
- The auto-meter brake fires on a **budget/usage threshold** (`auto_brake_decision`). Hard-killing an in-flight `claude` invocation at that moment **discards partial work AND the tokens already spent on the call** (already paid, now wasted) — and the auto-meter then **auto-releases** (`BrakeDecision::Release`, `:1474`), so the killed task is re-run from scratch, spending *more* budget. That is the opposite of what a budget cap should do.
- A budget cap reads naturally as "**stop starting new work**" (block new claims — which the brake already does), not "abort the call currently in flight."

So the delivery has an unmade decision: should an `AUTO_METER_REASON` brake **hard-kill** (like Stop) or **soft-brake** (block new claims, let in-flight invocations drain)? LF20-04 silently picks hard-kill for both by routing every site through the same `kill_all`. This is a distinct policy work-item, not the trigger wiring LF20-04 owns.

## Proposed change
Make the kill trigger **reason-aware** rather than uniform:
- **User Stop** (`manual`, and any explicit operator stop) → `kill_all` (LF20-04 behavior — halt now).
- **Auto-meter** (`AUTO_METER_REASON`) → **soft brake only**: block new claims (existing brake behavior), let in-flight invocations finish; do **not** `kill_all`. Optionally make this configurable in the auto-meter config (a "hard cap" flag) if a hard ceiling is ever wanted.
- Implementation: if LF20-04 chose the `Brake` on-set hook shape, the hook inspects `state().reason` and only kills for non-auto reasons; if it chose per-site wiring, simply omit `kill_all` at the `:1473` auto-meter `SetOn` branch (and keep it at the two user sites).

## Tests
- Unit/structural: the on-set path invokes `kill_all` for `manual` but **not** for `AUTO_METER_REASON` (assert the reason gate).
- Behavioral: with a registered child, an auto-meter `SetOn` leaves it running (soft brake); a `manual` `set_on` kills it.

## Dependencies / sequencing
- **Depends on** LF20-04 (it refines LF20-04's "kill at every brake-on site" into a per-reason policy) and LF20-01 (`kill_all`).
- **Pairs with** LF20-10 (which reason persists across reboot) — settle "auto vs manual" once and apply to both kill and persistence.
- **Independent of** the resume half (LF20-05/06/07) and the exit path (LF20-03/09).

## Out of scope
The auto-meter threshold/decision logic itself (`usage_telemetry`); the kill primitive (LF20-01); UI surfacing of why the brake tripped.
