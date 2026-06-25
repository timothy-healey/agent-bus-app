# Candidate LF20-08 — End-to-end lifecycle confidence test (kill → recover → resume)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. The assembled-path confidence test that none of LF20-01..07 provides.

## Location
- New integration test in the app crate, e.g. `src-tauri/app/tests/lifecycle_resume.rs` (or a `#[cfg(test)]` module in `src-tauri/app/src/lib.rs`, alongside the existing root tests at `:2019`+).
- Exercises the seam between the registry (LF20-01), spawner (LF20-02), kill triggers (LF20-03/04), the store reconcile (LF20-05), and the boot/un-brake recovery wiring (LF20-06/07).

## Why it is a candidate
Every one of LF20-01..07 explicitly defers the **composition-root wiring** to "structural" assertions and pushes behavior down to per-component unit tests (registry kill in LF20-01; store arithmetic in LF20-05; "structural" handlers in LF20-03/04/06/07). Nothing exercises the **assembled loop**: spawn a registered child → `kill_all` → the run's `running` rows are re-queued **and** store occupancy is reconciled → the run drains to completion without stalling. For a delivery whose entire thesis is "killing on exit/Stop *creates* an occupancy leak that resume must repair," a single end-to-end test is the cheapest guard against the components being individually correct but mis-wired (e.g. reconcile invoked with the wrong `run_id`, recovery not firing on the un-brake path, killed tasks terminalized before re-queue).

## Proposed change
A hermetic integration test (no live `claude`) that:
1. Stands up the runtime stores + tasks against a temp/in-memory DB, seeds a small run with several stages and a bounded buffer.
2. Drives a fake/echo spawner through the registry so a "worker" reserves a slot and registers a real short-lived child (`sleep`/`sh -c`).
3. Fires the Stop trigger (`kill_all`) mid-flight; asserts the child group is dead, a `running` task row remains (or is tagged killed), and occupancy is now inflated relative to residency.
4. Runs the recovery sequence (`release_orphaned_running` → `reconcile_occupancy`) via the boot path **and** the brake-off path (LF20-07); asserts occupancy == residency and the killed task is re-queued.
5. Lets the run resume and asserts it drains to terminal state with no stall (no permanently-reserved slot).

## Tests
This *is* the test. Keep it deterministic: fake spawner with bounded sleeps, fixed `now_unix()` injection (the codebase already threads `now_unix()` — e.g. `lib.rs:1288`), no reliance on wall-clock grace beyond a tiny bound.

## Dependencies / sequencing
- **Depends on** LF20-01..07 existing (or stubs thereof) — this is the **final** item, written last to validate the assembled delivery.
- Cross-crate: reads/asserts against `runtime` stores; lives in the app crate where the wiring is assembled.

## Out of scope
Live `claude` invocation; Windows kill path; UI/event assertions. Per-component correctness (covered by each item's own unit tests).
