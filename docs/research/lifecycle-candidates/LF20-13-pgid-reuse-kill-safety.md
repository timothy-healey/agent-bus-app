# Candidate LF20-13 — Guard `kill_all` against reaped/recycled process groups (PID-reuse safety)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ `ProcessRegistry` / Killable spawner). The correctness seam **between** the registry primitive (LF20-01) and the spawner that feeds it (LF20-02): neither owns the guarantee that `kill_all` never signals a process group that has already been reaped and recycled by the OS.

## Location
- `src-tauri/app/src/process_registry.rs` (new in LF20-01) — `kill_all()` iterates the registered pgids and does `kill(-pgid, SIGTERM)` → grace poll → `kill(-pgid, SIGKILL)`. LF20-01's contract is "idempotent + best-effort (a gone pid is fine)."
- The spawner closures (LF20-02) — `src-tauri/app/src/pipeline_activator.rs` (`runner_for` `:404`/`:411`, `runner_for_team` `:186`, `chat_runner_for` `:431`) routed through `…::with_spawner`. LF20-02's spawner sequence is: `.process_group(0)` → `.spawn()` → `registry.register(child.id())` → read stdout/stderr → **`.wait()`** → `registry.deregister(pgid)`.

## Why it is a candidate
"A gone pid is fine" (LF20-01) covers signalling a pid that is simply *dead* — `kill(2)` returns `ESRCH`, harmless. It does **not** cover the dangerous case: a pid/pgid that has been **reaped and then reused** by the kernel for an unrelated process. The LF20-02 spawner creates exactly this window by design:

1. The child is reaped at **`.wait()`** (step before deregister). The instant `wait()` returns, the kernel may recycle that pid/pgid for any new process on the system.
2. `registry.deregister(pgid)` runs **after** `wait()`. Between those two statements the registry still holds a pgid whose original owner is gone.
3. If `kill_all()` (fired from the exit trigger LF20-03 or the Stop trigger LF20-04) reads the set during that window, `kill(-pgid, SIGTERM/SIGKILL)` is delivered to **whatever now owns that recycled group** — an unrelated process, possibly outside the app. Because we deliberately signal the *negative* pgid to take down `claude`'s whole subtree (LF20-01), a mis-target sprays a SIGKILL across an unintended group.

The window is small but it is a real, security-/safety-relevant defect surface (killing an unrelated process), and it is **structurally unowned**: LF20-01 reasons about the kill mechanics assuming the set holds live groups; LF20-02 reasons about capturing output and registering, and its natural `wait()`-then-`deregister()` ordering is what opens the gap. Stop (LF20-04) makes it worse than exit — on Stop the app stays alive and the sweep/worker churn keeps spawning and reaping children, so reap↔kill races are recurring, not a one-shot at quit.

## Proposed change
Make "registered ⇒ not yet reaped" an invariant the registry can rely on. Pick one in the plan:
1. **Hold the `Child`, kill via the handle / verify before signal.** Register the `Child` (or an owned handle), not a bare pgid. `kill_all` then operates on entries whose `Child` has **not** been `wait()`ed, so the kernel cannot have recycled the pid. The spawner must coordinate ownership so `kill_all` and the reaping `wait()` don't both consume the same `Child` (e.g. an `Arc<Mutex<Option<Child>>>` per entry; whoever reaps takes it out and deregisters atomically).
2. **Deregister-before-reap under a lock.** Hold the registry lock across the reap→deregister transition (or take the entry out of the set *before* the blocking `wait()` and only signal entries still in the set), so `kill_all` can never observe a pgid whose process has been reaped.
3. **Start-time barrier.** Have `kill_all` snapshot the set under the same lock the spawner uses for register/deregister, and have the spawner remove the entry *before* the reaping `wait()` returns control — closing the post-reap window.

Option 1 is the most robust (no recycled-pid reachable by construction) and aligns with the spec's "killable spawner" framing; settle the ownership/`Arc<Mutex<Child>>` shape alongside LF20-01's registry type and LF20-02's spawner.

## Tests (no live `claude`)
- Structural/concurrency: a spawner that registers a real short-lived child, then races `kill_all()` against the child's natural exit+deregister in a loop; assert `kill_all` never signals a pgid after that pgid's `Child` was reaped (e.g. instrument the signal call to record targets and assert no target equals a deregistered/reaped pgid). 
- Invariant: after a child exits normally, the registry entry is gone before (or atomically with) the reap, so a subsequent `kill_all` is a no-op for it.

## Dependencies / sequencing
- **Couples** LF20-01 (registry type + `kill_all`) and LF20-02 (spawner register/reap/deregister) — best resolved *while* those two are designed, since the fix shapes the registry's stored type (`Child` vs `pgid`) and the spawner's reap ordering. Flag it as a constraint on both rather than a late add-on.
- Independent of the resume half (LF20-05/06/07/10/12) and the triggers' wiring (LF20-03/04), which only consume `kill_all`.

## Out of scope
The signal/grace escalation mechanics themselves (LF20-01); Windows (no `kill_all` there — LF20-01 no-ops); selective per-task kill (Stop kills all).
