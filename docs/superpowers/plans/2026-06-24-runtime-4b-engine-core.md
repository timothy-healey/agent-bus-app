# Runtime Redesign ④b — New Engine Core (worker pools + backpressure + generator + output contract)

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Detailed design: `docs/superpowers/specs/2026-06-24-runtime-bounded-buffer-pipeline-design.md`. This bundles spec sub-plans ④b (worker pools), ④c (generator), ④d (output contract/L1) — they're inseparable for a working, testable unit. Built as a NEW engine module tested with `FakeRunner`; **NOT wired into the live activator** (the old single-task `pool.rs` stays untouched and green). Cutover is ④d.

**Goal:** A new bounded-buffer execution engine — block-before-claim worker pools (transformers) + a loop-until-dry generator + the list-of-items output contract — fully unit-tested against an in-memory pool with `FakeRunner`, leaving the existing runtime running.

**Architecture:** Build on the ④a aggregates (`StoreRepo`, `RunStore`, `GeneratorLedger`). The `Runner` trait is UNCHANGED (old pool keeps using `RunnerOutput.verdict`/`artifact_path`); the new engine calls `Runner::invoke`/`invoke_stream` and parses `final_text` with a NEW `parse_items` (list of `{key, artifact_path, verdict?}`) plus an `output_contract` preamble it composes into the system prompt. So the change to the runners crate is additive; the engine lives in new `runtime` modules.

**Tech Stack:** Rust async (tokio), sqlx/SQLite, the existing `FakeRunner`.

---

## ④a API available (from the foundation): 
`StoreRepo { ensure, reserve→bool, release→bool, occupancy, is_full }`, `RunStore { create, get, set_generator_dry, try_complete→bool }`, `GeneratorLedger { record_keys→u64(new count), found_keys→HashSet }`. All ctors take an owned `SqlitePool` (wrap in `Arc`). `tasks.run_id`/`item_key` columns exist, not yet read/written.

## Tasks (TDD)

- [ ] **Task 1 — Output contract + list parser (runners ACL, additive).** In `src-tauri/runners/src/stream_json.rs` (co-located with `parse_verdict`/`parse_artifact`):
  - `pub struct OutputItem { pub key: String, pub artifact_path: Option<String>, pub verdict: Option<Verdict> }`.
  - `pub fn parse_items(text: &str) -> Vec<OutputItem>` — parse the agent's emitted item list. Convention (document it): each item is a fenced/marked block with `KEY:`/`ARTIFACT:`/`VERDICT:` lines (lenient; a single legacy `VERDICT:`/`ARTIFACT:` with no `KEY:` → one item with `key=""`). Never panics.
  - `pub fn output_contract(role: &str, artifact_dir: &str, already_found: &[String]) -> String` — the system-prompt preamble that TELLS the agent to emit the item list (producer → items with keys + artifact paths under `artifact_dir`; reviewer → add a `VERDICT:` per item; generator → only NEW keys not in `already_found`). This is the L1 fix (agents are finally told the contract). Pure; unit-tested for shape.
  - Keep `Runner` trait + `RunnerOutput` UNCHANGED. Tests: parse N items; legacy single-item; malformed → best-effort. Commit.
- [ ] **Task 2 — Work-item identity on TaskStore.** Extend `Task` with `run_id: Option<String>` + `item_key: Option<String>`; read/write the (already-present) columns in `insert`/`update`/`row_to_task`/`SELECT`. Add `Task::work_item(run_id, item_key, stage, artifact, …)` constructor. Keep existing constructors working (run_id/item_key None for legacy). Tests: round-trip a work-item; legacy task still loads. Commit.
- [ ] **Task 3 — Engine module skeleton + `EngineContext`.** New `src-tauri/runtime/src/engine.rs` (module, not wired anywhere yet). `EngineContext` bundles: `Arc<StoreRepo>`, `Arc<RunStore>`, `Arc<GeneratorLedger>`, `Arc<TaskStore>`, `Arc<Brake>`, `Arc<dyn Runner>`, the pipeline, project_root/target_repo, `read_prompt`. Pure helpers: `downstream_stage(team)`, `artifact_path(run_id, stage, key, attempt)`. Commit (compiles, no behavior).
- [ ] **Task 4 — Transformer step (block-before-claim).** `async fn transform_once(ctx, team) -> StepOutcome`:
  1. Brake check.
  2. Determine downstream store (the `on_approve`/hand-off target's input store). `reserve` a slot; if full → `Backpressure` (no claim). 
  3. `claim_next_for_stage(team.id)`; none → release reservation, `Idle`.
  4. Build invocation: system prompt = team prompt + `output_contract(role, artifact_dir, &[])`; scope includes the artifacts dir (FIX the L1 write-access gap — `--add-dir`/writes cover `${project}/artifacts`); run via `ctx.runner.invoke(_stream)`.
  5. On Ok: `parse_items`; for the (transformer = 1→1) first item, `commit` into the reserved downstream slot + insert a child work-item (run_id, new item_key or inherited, downstream stage, artifact); free this stage's input slot (the claimed item leaves → `release` this team's own input store). On parse-empty/err: `release` the downstream reservation; operational-failure path (audited error class; bounded retry → needs-human), mirroring the existing pool's synthetic-revise fallback.
  Tests with `FakeRunner`: happy path moves an item downstream + occupancy accounting; full downstream → Backpressure, no claim, no run; failure releases the reservation. Commit.
- [ ] **Task 5 — Generator step (loop-until-dry).** `async fn generate_once(ctx, source_team) -> StepOutcome`:
  1. Brake check; if `run.generator_dry` → `Retired`.
  2. Compute free downstream slots K (capacity − occupancy of the source's downstream store); K==0 → `Backpressure`.
  3. `found = ledger.found_keys`; run the generator with `output_contract("generator", artifact_dir, &found)` (and the source prompt) — it returns up to K items with NEW keys.
  4. `parse_items`; filter to keys not in `found` and cap at K; `record_keys` (new count); for each: `reserve`+`commit` into downstream + insert work-item. 
  5. New count == 0 → `set_generator_dry` → `Retired`.
  Tests with `FakeRunner` scripted [batch1, batch2, empty]: emits bounded by K (backpressure), dedups against the ledger, retires when dry. Commit.
- [ ] **Task 6 — Run completion checker.** `async fn try_finish_run(ctx, run_id) -> bool`: if `run.generator_dry` AND every stage store occupancy==0 AND no `running` tasks for the run → `RunStore::try_complete`. Test: completes exactly once when drained+idle; not before. Commit.
- [ ] **Task 7 — Pool driver (in-memory, test-only wiring).** A `run_pool_until_quiescent(ctx, teams)` test helper that loops `generate_once`/`transform_once` across teams until all return Idle/Backpressure/Retired and the run completes — the end-to-end crux test: a generator emitting M>capacity candidates, a single-worker transformer draining the bounded queue, items flowing to a terminal stage, backpressure observed, run completes when dry+empty. (This proves the model without touching the live activator.) Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace` (old pool tests still green; new engine tests green)
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run` (untouched)
- `npx tsc --noEmit`; `bun run build`
- Tag: `git tag plan-runtime-4b`

## Constraints
Local commits on `main`, NEVER push. **Do NOT modify `pool.rs`/`router.rs`/the activator** — the new engine is parallel and unwired; the old runtime stays green. Runner trait unchanged (additive parse path only). Mirror the existing atomic-guard + FakeRunner test idioms.
