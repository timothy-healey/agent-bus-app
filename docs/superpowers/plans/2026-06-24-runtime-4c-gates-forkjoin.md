# Runtime Redesign ④c — Gates-as-stores + Fork/Join integration

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-06-24-runtime-bounded-buffer-pipeline-design.md` (Gates-as-stores; Fork/Join integration; review-fan default = collect-all/revise-once). Builds on the ④b engine (`runtime/src/engine.rs`) and the EXISTING fanout machinery (`runtime/src/fanout_store.rs`, the `FanOutGroup` aggregate, P1–P3). Still ADDITIVE/unwired — old `pool.rs` stays green; live OHS gate commands are re-pointed at cutover (④d).

**Goal:** Teach the new engine to route through **gates** (bounded stores whose consumer is the human) and **forks/joins** (within-item parallelism: one item → lanes → barrier → collect-all/revise-once), reusing the existing `FanOutGroup` barrier, all tested with `FakeRunner`.

**Architecture:** The engine's transform/route step gains node-kind awareness for its downstream target: team (1→1, ④b), **gate** (commit into the gate store, await human verdict), **fork** (expand into lane work-items + a `FanOutGroup`), **join** (barrier; on completion commit the continuation or revise-bundle). The `FanOutGroup` aggregate + `fanout_store` guards are reused unchanged; lane items live in the new stores.

---

## Available APIs
- ④b engine: `transform_once`/`generate_once`/`try_finish_run`/`run_pool_until_quiescent`, `EngineContext`, `StepOutcome`, `artifact_path`/`artifact_dir`, `OutputItem`/`parse_items`/`output_contract`, `Task::work_item`.
- ④a: `StoreRepo`/`RunStore`/`GeneratorLedger`.
- Existing fork/join: study `runtime/src/fanout_store.rs` + `FanOutGroup` (its `record_and_try_complete`/quorum/early-cancel methods + the `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0` guard) and how `pool.rs` `expand_fork`/`settle_and_route`/`on_group_complete`/`spawn_continuation` use them (for reference — DO NOT modify pool.rs).

## Tasks (TDD, FakeRunner + in-memory pool)

- [ ] **Task 1 — Route-target resolution in the engine.** Add `enum RouteTarget { Team(Team), Gate(Gate), Fork(Fork), Join(Join), Escalation(Escalation), None }` + `fn resolve_target(pipeline, stage_id) -> RouteTarget`. `transform_once` uses it to decide what to do with an approved item's `on_approve` target. Unit-test resolution for each kind. Commit.
- [ ] **Task 2 — Gate-as-store routing.** When a transformer's approved item targets a **gate**: reserve a slot in the GATE's store (`StoreRepo.ensure(run, gate.id, gate_capacity)`; gate_capacity from the gate or a default), commit the work-item there in state `gated` (the human is the consumer; the gate store's capacity is the backpressure). Add `gated` handling so `try_finish_run` does NOT complete while gated items exist. Tests: routing to a gate parks the item in the gate store; a full gate store backpressures the upstream. Commit.
- [ ] **Task 3 — Gate verdict (engine fn).** `async fn apply_gate_verdict(ctx, task_id, verdict) -> StepOutcome`: **approve** → reserve+commit the item into the gate's `downstream` store (block-before-claim), free the gate slot; **revise** → route the item back to the producing team's store (with the feedback bundle, reusing the revision-bundle reader) ; **reject** → route to the escalation/needs-human terminal. (This is the engine-side logic; wiring the live `approve_gate`/`revise_gate`/`reject_gate` OHS commands to it is ④d — do NOT change those commands here.) Tests for all three. Commit.
- [ ] **Task 4 — Fork expansion.** When an approved item targets a **fork**: create a `FanOutGroup` (reuse the existing aggregate/store) + one lane work-item per `fork.lanes` entry, each reserved+committed into that lane team's store, sharing `group_id` + carrying `lane`/`join_target` (the new engine sets these on the work-item via `Task` fork fields, which already exist). The original item terminates as forked. Backpressure: if any lane store is full, the fork waits (don't partially expand — reserve all lanes or none). Tests: one approving item → N lane items with shared group; full lane store backpressures. Commit.
- [ ] **Task 5 — Join barrier.** When a lane's terminal item approves into its `join_target`: record the lane verdict in the `FanOutGroup` (reuse `record_and_try_complete`/quorum/early-cancel — honor `cancel_on_reject`/`quorum`, default full-barrier). On group completion: **all-approve** → reserve+commit the continuation item into `join.downstream` store; **any non-approve (collect-all/revise-once default)** → bundle all lane critiques and route the item back to the producer (revise), once. Reuse the completes-once guard. Tests: all-approve → downstream; one-reject (default) → revise-once with bundled feedback; the P2/P3 paths still behave (a focused test each). Commit.
- [ ] **Task 6 — Engine driver + completion update.** Extend `run_pool_until_quiescent` (test helper) to drive gate verdicts (via an injected verdict-decider for tests) + fork/join, and `try_finish_run` to require: generator dry AND all stores empty (incl. gate stores) AND no open `FanOutGroup`s AND no running tasks. Crux test: a pipeline with a fork (2 reviewer lanes) + a gate, end-to-end with FakeRunner — items fan out, rejoin (collect-all), pass a gate, and the run completes. Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace` (old pool + fanout tests still green; new engine gate/fork/join tests green)
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `npx vitest run`; `npx tsc --noEmit`; `bun run build`
- Tag: `git tag plan-runtime-4c`

## Constraints
Local commits on `main`, NEVER push. Do NOT modify `pool.rs`/`router.rs`/the activator/the live gate OHS commands — reuse the `FanOutGroup`/`fanout_store` aggregate as-is; the new engine is still parallel/unwired. Reuse the existing completes-once + reserve guards (no new race-prone count-then-act).
