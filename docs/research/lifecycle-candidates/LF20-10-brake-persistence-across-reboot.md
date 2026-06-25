# Candidate LF20-10 — Persist brake state so an explicit Stop survives app exit/reboot

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. The resume-intent gap that boot reconciliation (LF20-05/06) *creates*: resume must repair state without silently un-Stopping the user.

## Location
- `src-tauri/app/src/lib.rs:1285` — `let brake = Arc::new(Brake::new());`. The brake is constructed **purely in-memory**, with no load from the DB/settings. There is no persisted brake row and no `brake_state` restore on the boot path.
- Boot recovery: `src-tauri/app/src/lib.rs:1288` `release_orphaned_running` (+ `reconcile_occupancy` per LF20-06) runs unconditionally in `setup`, *before* `activator.activate(&project_id)` (`:1448`) starts the per-team worker loops.
- The brake-on sites that LF20-04 turns into real kills: RootDispatcher `"brake_on"` (`:1067`/`:1070`), the frontend `runtime::api::brake_on` (`:1527`), the auto-meter `SetOn` (`:1473`).

## Why it is a candidate
LF20-04 makes **Stop a real kill** of in-flight `claude`; LF20-03/05/06 make **boot re-queue the killed `running` rows and reconcile occupancy** so the run resumes. Combine them across a reboot and the user's intent is lost: a user hits **Stop** (brake on, work killed), then **quits**. On next launch `Brake::new()` comes up **OFF**, boot recovery re-queues the killed tasks, and the worker loops — no longer braked — **immediately re-run the very work the user explicitly stopped**. "Clean resume" must not silently un-Stop.

This is distinct from every LF20-01..09 item: none of them persists or restores brake state. LF20-06/07 reason about whether tasks are re-queued, never about whether the *run should be allowed to resume at all* on the reboot path. Right now the auto-resume-after-Stop-then-quit behavior is **accidental**, not a decision the delivery made.

## Proposed change
Make brake state durable and authoritative across the process boundary:
1. Persist `(on/off, reason, ts)` to a settings/store row on every `set_on`/`set_off` (wire at the composition root — the runtime `Brake` aggregate stays persistence-unaware, mirroring the registry/store-unaware pattern LF20-04 establishes; the root writes the row in the same place it triggers `kill_all`).
2. At boot (`:1285`), **load** the persisted brake before recovery. Run the recovery sequence (release-orphaned → reconcile-occupancy) regardless, but if the persisted brake is **on**, come up braked: keep the worker loops gated and surface the braked state to the UI, so the run does **not** auto-resume until the user hits Resume (which then routes through LF20-07's in-process recovery).
3. Decide explicitly (plan): does an `AUTO_METER_REASON` brake also persist, or only `manual`? (Pairs with LF20-12 — the auto vs manual distinction.)

## Tests
- Composition-root / structural: `set_on`/`set_off` write the persisted row; boot loads it into the constructed `Brake`.
- Behavioral: seed a persisted `brake=on` + an orphaned `running` task; run boot recovery; assert the task is reconciled but the worker loops stay gated (no claim) until brake-off.

## Dependencies / sequencing
- **Depends on** LF20-04 (Stop is now a real kill — the reason this gap bites) and LF20-06 (boot reconcile is what would auto-resume).
- **Pairs with** LF20-07 (Resume-without-reboot) and LF20-12 (per-reason brake policy).
- Cross-crate: the `Brake` aggregate lives in `runtime`; persistence is wired at the app root (keep `Brake` unaware).

## Out of scope
The kill primitive (LF20-01); per-task selective resume; the in-process Resume path (LF20-07); UI design of the "resumed-while-braked" affordance (surfacing only, detail likely LF23).
