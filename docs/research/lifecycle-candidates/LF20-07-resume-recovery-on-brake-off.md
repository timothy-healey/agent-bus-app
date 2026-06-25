# Candidate LF20-07 — In-process resume recovery on brake-off (no reboot)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. Closes the **resume** half for the path that does **not** reboot: releasing the brake while the app stays alive.

## Location (the three un-brake / resume sites — all at the composition root)
- `src-tauri/app/src/lib.rs:1074` — `RootDispatcher` `"brake_off"` arm: `self.runtime.brake.set_off(); … emit(USAGE_CHANGED)`.
- `src-tauri/app/src/lib.rs:1474` — auto-meter sweep `BrakeDecision::Release => { brake.set_off(); … emit(USAGE_CHANGED) }` (the closure at `:1455`–`:1480`).
- `src-tauri/app/src/lib.rs:1528` — `runtime::api::brake_off` registered directly in the `invoke_handler` (the frontend Resume button) — lives in the `runtime` crate and cannot reach the root recovery as-is.
- The **only** recovery today: `src-tauri/app/src/lib.rs:1288` `tasks.release_orphaned_running(now_unix()).await` — runs at **boot only** (in `setup`). LF20-06 adds `reconcile_occupancy` right after it, also boot-only.

## Why it is a candidate
LF20-04 makes **brake-on kill in-flight `claude` work**. LF20-05/06 then rebuild the run's state — but **only on the boot path** (`lib.rs:1288`). The Stop→Resume cycle that keeps the app running never reboots: every un-brake site above simply flips the brake off and emits `USAGE_CHANGED`. So after an auto-meter brake trips (now a real kill) and later **auto-releases** (`:1474`), or after a user hits Stop then Resume (`:1074` / `:1528`), the killed tasks are recovered by **nothing** — the boot reconcile (LF20-06) does not fire. The run resumes against orphaned `running` rows and possibly leaked store occupancy → it stalls. LF20-06 explicitly flags only "boot-only vs also-on-activation" as open; the **brake-off / resume-without-reboot** path is a distinct, uncovered trigger.

A second, sequencing-critical open question this item must answer: **what state do killed tasks land in on the Stop path?** On *exit* the whole process dies, so the worker never runs its completion logic and the DB is left mid-flight (LF20-05's leak premise). On *Stop with the app alive*, the worker's spawner (LF20-02) returns from `.wait()` with a SIGKILLed child and yields a *failed* `interpret_runner_output`; the still-live worker then runs its normal failed-outcome handling. So a killed task may transition to a **terminal failed/error** state (which `release_orphaned_running` will not re-queue) rather than staying `running`. The recovery design must distinguish "killed by Stop, recoverable" from "genuinely failed" so resume re-runs it. This needs verification against `runtime::worker` / `runtime::engine` (outside the app crate).

## Proposed change
Introduce a single root **resume-recovery** routine that mirrors the boot sequence — `release_orphaned_running(now)` → `reconcile_occupancy(run_id)` (LF20-05) for the active run(s) — and invoke it whenever the brake transitions **off** while a run is live. Wire it at all three un-brake sites; the cleanest convergence is to pair it with whatever shape LF20-04 chose for kill-on-set:
- If LF20-04 adds a `Brake` on-set hook, add the symmetric **on-clear hook** wired at the root to the recovery routine, so `set_off` everywhere converges through it (`Brake` stays runtime/registry/store-unaware — it just invokes an injected `Fn`).
- Otherwise, replace the frontend `runtime::api::brake_off` (`:1528`) with a thin root `brake_off` command (set brake off via the runtime API, then recover), and call the recovery inline in the RootDispatcher arm (`:1074`) and the auto-meter `Release` branch (`:1474`).
Resolve the killed-task disposition question first (above): if the worker terminalizes killed tasks, either tag Stop-killed tasks so the worker leaves them re-queuable, or have the recovery routine re-queue the killed-failed disposition too.

## Tests
- Composition-root / structural: every `set_off` site routes through the one recovery routine (assert the on-clear hook fires exactly once on `set_off`, never on `set_on`).
- Behavioral (with LF20-05/06 in place): seed a run with a Stop-killed task (orphaned `running` + inflated occupancy), trigger brake-off, assert the task is re-queued and occupancy reconciled — without a reboot.

## Dependencies / sequencing
- **Depends on** LF20-04 (kill-on-Stop is what creates the gap), LF20-05 (`reconcile_occupancy`), and shares the boot routine LF20-06 wires.
- **Pairs with** LF20-06: together they make recovery fire on *both* resume paths (reboot **and** in-process un-brake), not just boot.
- Open question for the plan: killed-task disposition on the Stop path (verify `runtime::worker`); choose hook-symmetry vs per-site wiring to match LF20-04.

## Out of scope
Per-task selective resume; the reboot path (LF20-06); UI surfacing of killed/re-queued node state.
