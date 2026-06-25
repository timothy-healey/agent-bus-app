# Candidate LF20-12 — Per-reason brake persistence policy (auto-meter vs manual/reactive)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. Refines the brake-persistence work (LF20-10) so "clean resume" restores the user's **intent** without resurrecting a stale, machine-derived brake. Explicitly named-and-deferred: LF20-10 leaves "does an `AUTO_METER_REASON` brake also persist, or only manual?" as an open plan decision and tags it *"Pairs with LF20-12 — the auto vs manual distinction."*

## Location
- `src-tauri/app/src/lib.rs:1285` — `let brake = Arc::new(Brake::new());`. Boot is where a persisted brake would be loaded (LF20-10) and where the persisted-reason policy must be applied.
- `src-tauri/app/src/lib.rs:1455`–`1480` — the auto-meter sweep: every 15 s it `compute_snapshot`s live usage and applies an `auto_brake_decision`, calling `brake.set_on(AUTO_METER_REASON)` (`:1473`) / `brake.set_off()` (`:1474`). This is the machine that *owns* the auto brake — it is recomputed and self-released continuously.
- `src-tauri/app/src/lib.rs:1469` — `auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON)`: the codebase already keys behavior on the brake **reason**, the exact discriminator this item formalizes for persistence.
- The manual/reactive on-sites: RootDispatcher `"brake_on"` (`:1067`), frontend `runtime::api::brake_on` (`:1527`) — these carry user/reactive reasons (e.g. `"rate-limit"`), not `AUTO_METER_REASON`.
- `usage_telemetry::brake_policy` (`AUTO_METER_REASON`, `auto_brake_decision`) and the `Brake` aggregate live in sibling crates (read-only analysis here; persistence policy is wired at the app root).

## Why it is a candidate
LF20-10 makes brake state durable so an explicit **Stop survives reboot**, and prescribes a blanket boot rule: *"if the persisted brake is on, come up braked … the run does not auto-resume until the user hits Resume."* That rule is **correct for a manual Stop and wrong for an auto-meter brake**, and nothing in LF20-01..11 distinguishes the two:

- An `AUTO_METER_REASON` brake is **not an intent** — it is a derived function of a usage window that the sweep recomputes every 15 s (`:1466`). By the time the app reboots, the window may have rolled over and usage recovered. Persisting that brake and then *withholding auto-resume until the user clicks Resume* (LF20-10's rule) would **strand a run that should already be free to proceed** — the opposite of the auto brake's self-releasing contract.
- Conversely, even if an auto brake *is* persisted, the first post-boot sweep (`:1471`–`:1474`) will independently `Release` it once usage is fine — so a persisted auto brake produces at best a redundant boot-flap and at worst a transient wrong state (braked for up to one sweep interval) gating the worker loops for no reason.
- The right policy is asymmetric **by reason**: persist + authoritatively restore *manual/reactive* brakes (intent — honor LF20-10's no-auto-resume), but for *auto-meter* brakes do **not** treat the persisted bit as authoritative — come up un-braked (or immediately let the sweep re-decide from fresh telemetry) rather than blocking on a user Resume.

This is a distinct decision LF20-10 explicitly punts and LF20-07 never reaches (LF20-07 reasons about re-queuing killed tasks on brake-off, never about *which brake reasons should survive a process boundary at all*). Getting it wrong silently changes product behavior (auto-throttle becomes sticky-across-reboot).

## Proposed change
Make brake persistence/restore **reason-aware**, layered on LF20-10's durable brake row:
1. **Persist policy:** record the reason alongside `(on, ts)` (LF20-10 already persists the tuple). Either (a) persist all reasons but tag auto vs manual, or (b) persist only non-auto reasons. Prefer (a) so the row is a faithful audit trail and the *restore* step makes the decision.
2. **Restore policy at boot (`:1285`):** if the persisted brake reason is a manual/reactive reason → restore braked and apply LF20-10's no-auto-resume gate. If it is `AUTO_METER_REASON` → do **not** come up braked on the strength of the persisted bit; let the auto-meter sweep (`:1471`) re-derive the brake from current usage on its first tick (optionally seed `auto_on` correctly so the sweep's hysteresis isn't confused).
3. **Define the discriminator once** (a small `is_manual_reason`/`persists_across_reboot` helper keyed off `AUTO_METER_REASON`), shared by the persist and restore sites so the two halves can't drift.
4. Confirm against `usage_telemetry::brake_policy` whether any *other* machine-derived reasons exist that should follow the auto (non-persistent-intent) rule.

## Tests
- Composition-root / structural: a persisted `manual` brake restores braked (worker loops gated, no auto-resume — composes with LF20-10's test); a persisted `AUTO_METER_REASON` brake does **not** gate the worker loops at boot.
- Behavioral: seed persisted `brake=on(AUTO_METER_REASON)` + low current usage; run boot + one auto-meter sweep tick (inject usage snapshot); assert the run is **not** stranded — brake ends off without a user Resume. Seed `brake=on("rate-limit")`; assert it stays on until explicit Resume.

## Dependencies / sequencing
- **Depends on** LF20-10 (the durable brake row + the boot load/restore hook this refines). Build directly on top of it.
- **Pairs with** LF20-07 (in-process resume) — the manual-restore path should route through the same recovery routine when the user later hits Resume.
- Cross-crate: reason constants live in `usage_telemetry`; the `Brake` aggregate stays persistence-unaware; the policy is decided/wired at the app root (mirrors LF20-04/10).

## Out of scope
The durable-row mechanics and the no-auto-resume gate themselves (LF20-10); the kill primitive (LF20-01); the auto-meter decision arithmetic (`auto_brake_decision`, owned by `usage_telemetry`); UI surfacing of "resumed-while-braked" vs "auto-cleared on boot" (detail, likely LF23).
