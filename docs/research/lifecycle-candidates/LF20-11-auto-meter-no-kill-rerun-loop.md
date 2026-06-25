# Candidate LF20-11 — Per-reason kill policy: the auto-meter brake must not kill-and-re-run in-flight work

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers — Stop, decision 2). This is the **auto vs manual** distinction the existing items defer to "LF20-12" (named in LF20-10 line 20 and the LF20-10 dependency list).

## Location
- `src-tauri/app/src/lib.rs:1473` — auto-meter sweep `BrakeDecision::SetOn(reason) => { brake.set_on(reason); … }`. LF20-04 adds `registry.kill_all()` here, identically to the manual-Stop sites.
- `src-tauri/app/src/lib.rs:1474` — auto-meter `BrakeDecision::Release => { brake.set_off(); … }`. LF20-07 adds the resume-recovery (re-queue killed `running` rows) here.
- `src-tauri/app/src/lib.rs:1469` — `AUTO_METER_REASON` is the reason string that distinguishes the auto-meter brake from the manual `"manual"` reason set at `lib.rs:1070`.
- The 15-second sweep loop: `lib.rs:1465`–`1478` (`tokio::time::sleep(… 15s)` then `auto_brake_decision`).

## Why it is a candidate
LF20-04 wires `kill_all()` at **all three** brake-on sites **uniformly** — including the auto-meter `SetOn` at `:1473` — and LF20-07 wires recovery (re-queue) at the auto-meter `Release` at `:1474`. The auto-meter brake is a **budget/rate guard** that trips and releases **autonomously every 15s** (`auto_brake_decision`), not a user decision. Composing LF20-04 + LF20-07 on this autonomous path produces a pathological loop:

1. Budget meter trips → `SetOn(AUTO_METER_REASON)` → `kill_all()` **discards all in-flight `claude` work** (partial reasoning already paid for is thrown away).
2. ~15s later usage drops below the release threshold → `Release` → recovery **re-queues the killed `running` rows** → workers **re-run the very work that was just killed**, paying for it **again**.
3. Re-running pushes usage back up → trips again → kill again → …

The brake whose entire purpose is to **reduce** spend would instead **amplify** it (pay-kill-pay-kill), and would repeatedly destroy progress. This is the opposite of "clean." A **manual Stop** killing in-flight work is the spec's intent (the user chose to halt); an **auto-meter** brake doing the same is a defect. None of LF20-01..10 separates the two policies — LF20-04 treats every brake-on site the same, and LF20-10 only asks whether the auto reason should *persist*, not whether it should *kill*.

## Proposed change
Make kill (and the symmetric re-run recovery) **conditional on the brake reason**, decided at the composition root:
- **Manual / user Stop** (`reason == "manual"`, the `:1070` and frontend `:1527` paths): `kill_all()` — halt now, per decision 2.
- **Auto-meter** (`reason == AUTO_METER_REASON`, the `:1473` path): do **not** `kill_all()`. Let the brake do what it does today — gate **new** claims so no new `claude` is spawned — and let in-flight invocations **finish naturally**. This drains spend monotonically without discarding paid work, and removes the re-run loop because nothing was killed to re-queue.
- Pair the policy with LF20-07: only the manual-Stop disposition needs the on-`Release`/on-`brake_off` re-queue recovery; the auto path has no killed rows to recover.

Concretely: pass the reason (or a `should_kill: bool`) into whatever shape LF20-04 chose (the on-set hook receives the reason, or each site decides inline). The runtime `Brake` already carries `reason` (`state().reason`, read at `:1469`), so the root has what it needs.

## Tests
- Structural: auto-meter `SetOn(AUTO_METER_REASON)` does **not** invoke `kill_all`; manual `set_on("manual")` does (assert the kill hook fires for one reason and not the other).
- Behavioral (with the registry + spawner in place): trip the auto-meter brake while a child is registered; assert the child is **not** killed and the registry still holds it; then `Release` and assert **no** re-queue/re-run of that task.

## Dependencies / sequencing
- **Depends on** LF20-04 (defines the kill-on-set wiring this makes reason-aware) and LF20-07 (defines the re-run recovery this scopes to manual-only).
- **Pairs with** LF20-10 (per-reason *persistence* — auto vs manual; same `AUTO_METER_REASON` discriminator).
- Should be settled **in the same plan** as LF20-04, because LF20-04's "kill at all three sites" is only correct for the two manual sites.

## Out of scope
The kill primitive (LF20-01); brake persistence (LF20-10); changing the auto-meter trip/release thresholds (`auto_brake_decision` lives in `usage_telemetry`, outside this delivery).
