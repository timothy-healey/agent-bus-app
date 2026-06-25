# Candidate LF20-06 — Boot/activation reconcile wiring

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Boot reconciliation, decision 4).

## Location
- `src-tauri/app/src/lib.rs:1288` — boot recovery: `let _ = tasks.release_orphaned_running(now_unix()).await;` (in `setup`). The reconcile call goes immediately after.
- `src-tauri/app/src/pipeline_activator.rs:105` `PipelineActivator::activate(...)` — if activation is where recovery should run on project switch/create (the spec: "if activation is where recovery runs"). Decide whether reconcile belongs at boot only, or in `activate` too.

## Why it is a candidate
The new `StoreRepo::reconcile_occupancy` (LF20-05) does nothing until it is invoked on the resume path. The spec's ordered recovery is: (1) `release_orphaned_running` (exists) → (2) `reconcile_occupancy(run_id)` (new). Without this wiring, a resumed run that was killed mid-flight stays falsely backpressured and stalls — the exact gap the kill triggers expose.

## Proposed change
After `release_orphaned_running` at boot, resolve the run(s) to reconcile and call `stores.reconcile_occupancy(run_id)` for each. The active run is obtainable via the `RunStore` (`runs.latest_active_for_project(project_id)`, used in `pipeline_activator.rs:370` `active_run`). Sequencing matters: release orphaned `running` tasks → reconcile occupancy → then the worker loops + resumed run drive normally (active run, generator-dry flag, found-key ledger all persist). Confirm whether the same recovery should also run inside `activate()` for the project-switch path, or stay boot-only.

## Tests
Integration / structural at the composition root: after seeding an interrupted run (running task + inflated occupancy), the boot recovery path leaves occupancy reconciled and tasks re-queued. The arithmetic is covered by LF20-05's unit tests.

## Dependencies / sequencing
- **Depends on** LF20-05 (the method must exist).
- Final item in the resume half; pairs with the kill triggers (LF20-03/04) to "close LF20 + the resume-occupancy gap together" per the spec.
- Open question for the plan: boot-only vs also-on-activation, and which run(s) to reconcile when a project has history.
