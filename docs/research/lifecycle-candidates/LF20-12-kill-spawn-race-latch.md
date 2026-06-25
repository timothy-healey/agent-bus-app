# Candidate LF20-12 — Close the `kill_all` vs spawn-after-snapshot race on the live Stop path

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Kill triggers — Stop, decision 2: "Stop now actually halts running work"). This item makes that guarantee **race-free** on the path where the worker loops keep running (Stop with the app alive).

## Location
- `src-tauri/app/src/process_registry.rs` (new in LF20-01) — `kill_all()` iterates a **snapshot** of currently-registered pgids; the spawner (LF20-02) calls `register(child.id())` after each `.spawn()`.
- `src-tauri/app/src/lib.rs:1067`–`1074` — the `RootDispatcher` `"brake_on"`/`"brake_off"` arms; brake-on at the root triggers `kill_all` (LF20-04).
- `src-tauri/app/src/lib.rs:1527` — the frontend `runtime::api::brake_on` Stop path.
- The worker claim→spawn loop lives in `runtime::worker` (`../runtime`, outside the app crate) — it keeps claiming and spawning until it **observes** the brake gate.

## Why it is a candidate
`kill_all()` as specified (LF20-01) is a **one-shot snapshot**: it reads the set of registered process groups and kills each. But on the **Stop-with-app-alive** path the worker loops are still running. There is a window between (a) the root setting the brake + calling `kill_all`, and (b) a worker loop actually **observing** the brake before its next claim. A worker that has already passed its brake check can `.spawn()` a fresh `claude` and `register()` it **after** `kill_all` took its snapshot — so that child is never in the kill set and **escapes**. The exact headline of LF20 ("Stop halts in-flight work") then silently fails for any invocation that races the snapshot. This is invisible on the **exit** path (the whole process dies, so no new spawns happen — LF20-09's concern is different: bounding the grace on shutdown), and no existing item addresses the **concurrent spawn** race because LF20-01 frames `kill_all` as a pure best-effort snapshot and LF20-04 frames itself as "just the trigger wiring."

## Proposed change
Give the registry a **killing latch** so a kill is a *state*, not a one-shot:
1. `kill_all()` first sets an atomic `killing`/generation flag, **then** snapshots-and-kills.
2. The spawner (LF20-02), inside the critical section where it would `register()` a freshly-spawned child, checks the latch: if `killing` is set, it **immediately kills the child it just spawned** (same SIGTERM→grace→SIGKILL) instead of registering it, and returns the failed `interpret_runner_output`. (Equivalently: register-then-if-latched-kill, so there is no TOCTOU.)
3. The latch clears when the brake goes off (wire alongside LF20-07's on-`brake_off` recovery), re-enabling normal spawning on Resume.

This guarantees: once Stop is pressed, **no** `claude` survives or starts until Resume — closing the window without requiring the worker loops to observe the brake synchronously. Keep the runtime `Brake`/`worker` unaware; the latch is a registry concern wired at the root, mirroring the registry-unaware pattern LF20-01/04 establish.

## Tests (no live `claude`)
- Race test: set the registry `killing` latch, then drive the spawner with a real short-lived child (`sleep`/`sh -c`); assert the child is killed and **not** left registered (it does not escape).
- Idempotency: `kill_all()` while the latch is already set is a no-op beyond re-killing survivors.
- Latch clears on brake-off: after clear, the spawner registers normally again.

## Dependencies / sequencing
- **Depends on** LF20-01 (registry + `kill_all`) and LF20-02 (the spawner that must consult the latch).
- **Refines** LF20-04 (Stop trigger) — the trigger alone is insufficient against concurrent spawns.
- **Distinct from** LF20-09 (exit-path shutdown *bounding*; no concurrent spawns there) and LF20-07 (recovery *after* the brake clears).
- Verify the worker claim/brake-gate timing against `runtime::worker` to confirm the window's width (it does not change the fix — the latch closes the window regardless).

## Out of scope
The kill primitive's signal mechanics (LF20-01); per-task selective kill; Windows (non-unix `kill_all` no-op per LF20-01).
