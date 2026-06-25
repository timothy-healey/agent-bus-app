# Candidate LF20-15 — Close the spawn/register/kill race so a child can't survive Stop

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Killable spawner / § Kill triggers — Stop). The concurrency hardening LF20-02's "spawn → then register" ordering and LF20-04's single `kill_all` leave open.

## Location
- The spawner closure (LF20-02): `register(child.id())` happens **after** `.spawn()` (LF20-02 candidate, step 2 — "`.spawn()` → `Child`; `registry.register(child.id())`"). The window between exec and register is unguarded.
- The brake gate vs the spawn: the engine reads `brake.state()` to block **new claims**, but the brake check and the subsequent `runner.run()` → spawn are **not atomic** with `kill_all`. Brake-on sites (`lib.rs:1067`/`:1070`, `:1473`, `:1527`) do `brake.set_on(...)` then `registry.kill_all()` (LF20-04) concurrently with live worker loops in `pipeline_activator.rs:278`.
- `ProcessRegistry` (LF20-01, new `src-tauri/app/src/process_registry.rs`) — where the register/kill mutual exclusion must live.

## Why it is a candidate
LF20-04 makes Stop "actually halt running work" via one `kill_all()` sweep of the registry. But two TOCTOU windows let an in-flight `claude` **escape that sweep and keep running** — re-introducing the exact LF20 bug (orphaned process still writing the worktree) through a race the delivery never closes:

1. **Spawn-after-kill:** a worker that already passed the engine's brake check (brake then *off*) proceeds to `runner.run()`. Concurrently the user hits Stop: `brake.set_on` + `kill_all` sweep an empty/partial registry. The worker **then** spawns its `claude` — *after* the sweep — and the child is never killed. The brake blocks *new claims* but not the claim already in flight past the gate.
2. **Register-after-spawn:** even for a worker mid-spawn during the sweep, `kill_all` runs in the gap between `.spawn()` and `register(child.id())`, so the live child's pgid isn't in the registry yet and is missed.

In both cases Stop reports "stopped," the UI brakes, but a real `claude` keeps running and writing the worktree. None of LF20-01..14 addresses this: LF20-02 documents register-after-spawn without guarding the window; LF20-04 is a single best-effort sweep; LF20-13 only fixes *where* the grace runs (offload), not the race.

## Proposed change
Make registration and `kill_all` mutually exclusive, and make the registry **kill-latching** so nothing spawns into a swept state:
1. Add a "killing/closed" latch to `ProcessRegistry` (LF20-01) guarded by the same lock as `register`/`kill_all`. While Stop/exit holds it, `kill_all` (a) sets the latch, then (b) sweeps. `register` taken under the same lock either succeeds (added before the sweep, so it gets killed) or, if the latch is set, the spawner **immediately kills its own just-spawned child** (pgid in hand) rather than tracking it.
2. **Reserve-then-spawn** in the spawner: take a registry slot/guard *before* `.spawn()` (or register the `Child` the instant it exists under the lock and re-check the latch), eliminating the exec→register gap.
3. Keep the brake as the new-claim gate; the latch is specifically for the claim **already past the gate**. After Stop, the latch stays set until the brake clears (or kill scope ends) so a straggler claim can't spawn an unkillable child between sweep and Resume.

## Tests (no live `claude`)
- Concurrency: interleave `register` and `kill_all` (spawn a real `sleep`, race a register against a kill); assert no child survives — either it was killed, or the spawner self-killed it on a set latch. Registry empty afterward.
- Spawn-after-kill: with the latch set, a spawner attempt either no-ops the spawn or kills its own child immediately; assert the `sleep` is dead.

## Dependencies / sequencing
- **Depends on** LF20-01 (registry — adds the latch + lock discipline) and LF20-02 (spawner ordering — reserve/register-then-recheck).
- **Hardens** LF20-04 (Stop) and LF20-03 (exit) — both call `kill_all`, both exposed to the race.
- **Independent of** the resume half (LF20-05/06/07).

## Out of scope
The kill signal/grace mechanics (LF20-01); Windows (no-op there per LF20-01); per-reason policy (LF20-12) — the latch applies to whatever reasons LF20-12 decides should kill.
