# Runtime Redesign ④d — Cutover + Run lifecycle

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-06-24-runtime-bounded-buffer-pipeline-design.md` (Run lifecycle: Start/Stop; "the single-task linear flow is removed"). This is the CUTOVER: the composition root + activator stop using the old single-task `pool.rs`/`router.rs` and drive the NEW engine (④b/④c). Highest-risk plan — keep the workspace green at every commit; if a step can't be made green, STOP and report BLOCKED.

**Goal:** The live runtime runs the bounded-buffer engine. `Start a run` creates a `Run` + seeds the source; per-team worker pools (up to `workers.max`) drive `generate_once`/`transform_once`/fork/join with backpressure; the gate OHS commands call `engine::apply_gate_verdict`; run-completion + task-changed events fire; the old single-task pool is unwired (deleted if it stays green, else left dead + flagged).

**Architecture:** `PipelineActivator` builds an `EngineContext` (StoreRepo/RunStore/GeneratorLedger/FanOutStore/revision_reader/runner/pipeline/roots) and spawns, at a fresh generation: one generator loop for the source team + `workers.max` transformer loops per non-source team, each polling its engine step with a backpressure/idle sleep + the generation guard. `inject_topic` becomes `start_run` (topic optional). Frontend stays compatible (cards still come from `list_tasks`; the board's store/pool indicators are ④e).

---

## Available
- ④b/④c engine: `EngineContext { run_id, pipeline, stores, runs, ledger, tasks, brake, runner, project_root, target_repo, read_prompt, fanout, revision_reader }`, `generate_once`, `transform_once`, `fork_once`, `apply_gate_verdict`, `resolve_join_barrier`, `resolve_target`, `try_finish_run`, `StepOutcome` (Idle/Braked/Backpressure/Advanced/Generated/Retired/Revised/Escalated/GateBackpressure/JoinBackpressure/Failed), `RouteTarget`.
- The current activator (`app/src/pipeline_activator.rs`) — its generation-guard + per-team `tauri::async_runtime::spawn` loop pattern is the model; replace the `process_one_claim` body with engine steps.

## Tasks (commit per task; keep green)

- [ ] **Task 1 — `start_run` (runtime api).** In `runtime/src/api.rs`: add `start_run_inner(state, topic: Option<String>) -> Run` — create a `Run` (RunStore), `ensure` every team's store at its `store.capacity` for that run, and seed the source: create the initial generator trigger so `generate_once` will fire (e.g. mark the run active; the generator needs no input item — it produces). Keep `inject_topic` as a thin wrapper that calls `start_run` (topic optional, threaded as run context if present) so the existing terminal `/inject` keeps working. Tests (in-memory): start_run creates a run + stores. Commit.
- [ ] **Task 2 — Engine-driven worker loops in the activator.** Rewrite `spawn_team_loop` (or add `spawn_engine_loops`) so activation, for the active run, spawns: a generator loop for the source team + `workers.max` transformer loops per other team. Each loop: generation-guard check → call the team's engine step (`generate_once` for source; for a transformer, `transform_once`, which already routes to gate/fork/join/team) → on Backpressure/Idle/Retired sleep (e.g. 250–500ms) → after a settling step, `emit("task-changed", …)` + `try_finish_run` (emit `run-changed`/`usage-changed` on completion). Build the `EngineContext` from the resolved `ActivePipeline` + the boot-built collaborators (add StoreRepo/RunStore/GeneratorLedger/FanOutStore/revision_reader to `WorkerDeps`). Commit.
- [ ] **Task 3 — Re-point gate commands.** `approve_gate`/`revise_gate`/`reject_gate` (`runtime/src/api.rs` + the `RootDispatcher` arm in `app/src/lib.rs`) now call `engine::apply_gate_verdict` against an `EngineContext` for the task's run (build a per-call context, or hold the engine collaborators in `RuntimeState`). Emit `task-changed`. Keep the command signatures + the frontend IPC unchanged. Tests. Commit.
- [ ] **Task 4 — Activate on start + boot.** The activator’s `activate(project_id)` resolves the pipeline + (re)spawns the engine loops at a new generation (as today). Boot + create + the `activate_project` command all flow through it unchanged. `start_run` is what actually creates a Run and kicks the generator; calling it is wired to the terminal `/inject` and (later, ④e) the Start button. Commit.
- [ ] **Task 5 — Retire the old single-task path.** Remove the activator's use of `process_one_claim`. Then: if deleting `pool.rs`'s single-task production fns (`process_one_claim`/`settle_and_route`/`route`-driven single flow) + `router.rs` keeps `cargo test --workspace` green, DELETE them (and their now-obsolete tests), per the spec's "the single-task linear flow is removed." If deletion cascades beyond this plan's risk budget, leave them compiled-but-unwired with a `// DEAD: superseded by engine.rs (cleanup item)` note and list them for a cleanup plan. Either way the activator must NOT call them. Commit.
- [ ] **Task 6 — DOMAIN/context-map + reconcile.** Update DOMAIN.md/context-map.md: the WorkerPool now drives the bounded-buffer engine; `inject topic` → `start a run`; the single-task flow retired. Reconcile any now-stale comments. Commit.

## Verification gates (CRITICAL — must all pass each is non-negotiable)
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-runtime-4d`

## Constraints
Local commits on `main`, NEVER push. This is the cutover — keep every commit green; if blocked, STOP and report (do not leave a half-rewired broken tree). The live `claude` worker path remains structural-only (FakeRunner in tests); we are wiring the engine, not proving live claude. Frontend must keep building — `list_tasks` shape stays compatible (work-items are tasks); the board's new store/pool indicators are ④e.
