# Candidate LF20-05 — `StoreRepo::reconcile_occupancy(run_id)` (new method)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Boot reconciliation, decision 4). This is the **resume** half — the occupancy leak that killing-on-exit *creates*.

## Location
- **New method** on `StoreRepo` in `../runtime/src/store.rs` (sibling to existing `reserve` `:55`, `release` `:71`, `occupancy` `:85`, `capacity`/`occupancy_capacity` `:100`). The store table is `stores (run_id, stage, capacity, occupancy)`.
- (This file is in the `runtime` crate, outside the app crate. Read-only candidate analysis here; the change lands in `runtime`.)

## Why it is a candidate
The spec's core insight: `release_orphaned_running` re-queues interrupted task rows, but **nothing rebuilds store occupancy**. A slot reserved before a kill (`occupancy += 1` via `reserve`) is never committed or released, so the store stays **falsely full → backpressure → the run stalls**. Killing on exit/Stop (LF20-01..04) actively creates this leak, so a reconcile primitive must ship with it. No such method exists today — `StoreRepo` only has the atomic `reserve`/`release` guards.

## Proposed change
Add `async fn reconcile_occupancy(&self, run_id: &str) -> Result<(), StoreError>`: for each store (stage) of the run, set `occupancy` = the count of work-items **resident** at that stage — truth read from `tasks`, using the engine's residency states (items parked awaiting a worker). A single `UPDATE stores SET occupancy = (SELECT count(*) FROM tasks WHERE run_id = ? AND <stage residency predicate>) WHERE run_id = ? AND stage = ?` per stage (exact predicate to match the engine's residency definition — confirm against `runtime::engine`).

## Tests
- Seed a store row with inflated `occupancy` + N resident task rows at that stage; reconcile; assert `occupancy == N`.
- A leaked reservation (occupancy = 2, zero resident) reconciles to 0.

## Dependencies / sequencing
- **Independent** of the kill-trigger items (LF20-01..04) — pure store primitive; can be built/tested in parallel.
- **Blocks** LF20-06 (boot wiring calls this).
- Must use the same residency state definition the bounded-buffer engine uses (built on **R**) — verify against `runtime::engine` so the rebuilt count matches what actually occupies a slot.
