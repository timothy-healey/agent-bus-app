# Runtime Redesign ④e — Frontend board (runs, store occupancy, Start/Stop)

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-06-24-runtime-bounded-buffer-pipeline-design.md` (Frontend / Board; Run lifecycle Start/Stop). Final runtime chunk. Surfaces the bounded-buffer model in the UI + adds the two small read commands the board needs.

**Goal:** The board is run-scoped: a run selector + "Start run" / brake controls; each stage lane shows store occupancy (e.g. "2/3") and pool activity (busy/max); cards are work-items grouped by run + stage; refetch on `task-changed`/`run-changed`.

**Architecture:** Two small read commands (`list_runs`, `run_store_occupancy`) over the ④a/④d aggregates; frontend IPC + hooks + board UI. The cutover (④d) exposed `start_run`, the `run-changed` event, the `Run` serde type, `RunStore::latest_active_for_project`, `StoreRepo::occupancy`, and `Team.store.capacity`/`Workers.max`.

---

## Tasks

- [ ] **Task 1 — Read commands (backend).** In `runtime/src/api.rs` + register in `app/src/lib.rs`:
  - `list_runs(project_id) -> Vec<Run>` (RunStore; newest first).
  - `run_store_occupancy(run_id) -> Vec<StoreOccupancy { stage, occupancy, capacity }>` — occupancy from `StoreRepo`, capacity from the active pipeline's `Team.store.capacity` (resolve at the composition root; the `runtime` aggregate doesn't know the pipeline capacities unless passed — read from `RuntimeState.active().pipeline`). Tests (in-memory). Commit.
- [ ] **Task 2 — Frontend IPC.** `src/ipc/runtime.ts`: add `startRun(topic?) -> Run`, `listRuns(projectId) -> Run[]`, `runStoreOccupancy(runId) -> StoreOccupancy[]`, the `Run`/`StoreOccupancy` types, and an `onRunChanged` listener over `EVENTS.runChanged`. Wire-contract test for `Run`/`StoreOccupancy`. Commit.
- [ ] **Task 3 — Hooks.** `useRuns(projectId)` (load + refetch on `run-changed`, expose the selected/active run) and extend `useTasks` to refetch on both `task-changed` and `run-changed` and to expose grouping by `run_id`. A `useStoreOccupancy(runId)` (refetch on `task-changed`). Tests with mocked IPC. Commit.
- [ ] **Task 4 — Run selector + Start/Stop.** A run selector (topbar or board header) listing the project's runs (active marked); a **Start run** button → `startRun()` (no topic needed — the prompts are the work) → selects the new run; the existing brake toggle serves as Stop. Component test. Commit.
- [ ] **Task 5 — Board store/pool indicators.** `BoardView`: scope to the selected run; group work-items by stage; each lane header shows store occupancy "n/cap" and pool "busy/max" (busy = running tasks at that stage; max = `Workers.max`). Cards labelled by `item_key` (fallback to topic/id). Keep the existing card/needs-you treatment. Component test. Commit.
- [ ] **Task 6 — App wiring.** `App.tsx`: thread the selected run into `BoardView`/`ListView`; subscribe `run-changed`; the terminal `/inject` (now a run start) and the Start button both land you on a run-scoped board with cards appearing as the generator produces. Commit.
- [ ] **Task 7 — Impeccable pass (frontend-facing).** Run the impeccable skill over the run selector + board indicators: tokens (no px/hex literals), `ui/Button`, focus-visible, a11y (selector role/labels, progressbar semantics for occupancy if used), empty states ("no runs yet — Start a run"), loading. Apply safe fixes. Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-runtime-4e`

## Spec coverage
- Board groups by stage + store occupancy + pool activity → Tasks 1,3,5. ✓
- Run is the unit the board scopes to; run selector + Start run / brake → Tasks 2,3,4. ✓
- `run-changed`/`task-changed` drive refresh → Tasks 2,3,6. ✓
- Start a run is topic-less (prompts are the work; the A6 insight) → Task 4. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. Match existing patterns (`useTasks`/`useRuntimeEvents`/`BoardView`/`ui/*`/tokens/`useModalA11y`). Capacities come from the active pipeline (composition root), not the runtime aggregate.
