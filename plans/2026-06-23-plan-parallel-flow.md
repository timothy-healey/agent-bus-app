# Agent Bus App — Plan: Parallel flow (fork / join) — sub-project 2 of the brainstorming wizard

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add fan-out / fan-in to the pipeline schema and the Runtime so a pipeline can split one task into parallel **lanes** at a **fork** node and rejoin them at a **join** barrier — all-must-approve-else-needs-human — with the *completes-exactly-once* invariant owned by a new `FanOutGroup` aggregate guarded by the same single-row conditional-UPDATE pattern `claim` already uses.

**Architecture:** Two contexts change. **Pipeline Authoring** (`pipeline` crate) gains explicit `Fork` and `Join` node kinds (routes stay single-target), `Pipeline.forks`/`joins`, `NodeKind::{Fork, Join}`, `SCHEMA_VERSION = 2`, and v2-only validation (lanes/waits_for resolve to teams, ≥2 lanes, lane linearity, v1-rejects-fork, reachability). **Runtime** (`runtime` crate) gains nullable `group_id`/`lane`/`join_target` on Task, a new `FanOutGroup` aggregate (`fanout_group.rs`) + `fanout_store.rs` (the barrier guarded by `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0`), pool fork-expansion (one sibling task per lane; original terminates forked), and a pure `route()` that maps a `Join` target to a barrier `Routed` outcome the pool/store resolve. Migration `006_fanout.sql` adds the Task lane columns + `fanout_groups` + `fanout_lanes`. The frontend `Pipeline` TS interface + `PipelineView` render fork/join.

**Tech Stack:** Rust (serde, sqlx, tokio, thiserror, uuid), SQLite, the `agent_bus_core` kernel, React + TypeScript + Vitest.

**Source spec:** `docs/superpowers/specs/2026-06-23-parallel-flow-design.md` (brainstormed + DDD-vetted + amended 2026-06-23). Vet: `docs/vet-parallel-flow-design-2026-06-23.md`. Honour every decision, especially the vetted F1/F2/F3 below.

**DDD anchors (DOMAIN.md + the council vet 2026-06-23):**
- **F1 [medium] — invariants belong to aggregates.** The *exactly-one-continuation* barrier invariant spans the **lane sibling set**, so it needs an owning aggregate. **Resolution:** a tiny `FanOutGroup` aggregate (root `group_id`) holds the expected lanes, each lane's settled verdict, and a `completed` flag; the continuation is created by a method guarded by `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0` (rows-affected = the single-writer guard — the *same* pattern `claim_next_for_stage` uses). This fixes the count-then-create race in one move.
- **F2 [low] — off-language naming.** A fork→join parallel path is a **lane**, never a "branch" (the app already has git/worktree *branches*). Field/type names are `Fork.lanes`, `task.lane`, `Join.waits_for`.
- **F3 [low] — approving into a join is underspecified.** `route()` stays pure and total: a `Join` target resolves to a **barrier `Routed` outcome** (a sentinel `next_state`) the pool/store handle. `route()` is never asked to return many.
- **Shared-kernel bump.** `Pipeline ↔ Runtime` is a shared kernel keyed on `schema_version`. Fork/join is a breaking change → `schema_version` **1 → 2**; `validate.rs` accepts **both** 1 and 2; only v2 pipelines may contain fork/join.
- **Ownership split.** Fork/join **node definitions** live in Pipeline Authoring; the **fan-out group + barrier + verdict aggregation** live in Runtime. Fork-expansion is a **pool** operation; the join-barrier is the **store**; `route()` stays pure.
- **Lane contents.** A lane is a **linear team chain** (each team with its normal revise loop) — **no gate, escalation, or fork inside a lane**. This keeps the fan-out group **flat** (no nested groups) in v1.

---

## Orientation — the real code this plan builds on

Read these before starting; every task references them.

- **Pipeline Authoring (`pipeline` crate).**
  - `src-tauri/pipeline/src/model.rs` — the `Pipeline` aggregate. `Pipeline { id, name, description, schema_version, teams, gates, escalations }`; `Team { id, name, prompt, runner, scope, outputs: Routes, workers }`; `Routes { on_approve, on_revise, on_reject }` (each `Option<String>`, single-target); `Gate { id, label, downstream }`; `Escalation { id, triggers }`; `enum NodeKind { Team, Gate, Escalation }` (`#[serde(rename_all = "lowercase")]`); `pub const SCHEMA_VERSION: u32 = 1`; `Pipeline::node_ids() -> Vec<(String, NodeKind)>`. `#[serde(default = "default_schema_version")]` makes a missing `schema_version` default to `SCHEMA_VERSION`.
  - `src-tauri/pipeline/src/validate.rs` — `validate(&Pipeline) -> Result<(), PipelineValidationError>`. Enforces: schema_version supported, no teams ⇒ `NoTeams`, unique node ids (`DuplicateNodeId`), every route/gate-downstream resolves (`UnresolvedRoute`/`UnresolvedGateDownstream`), no self-reference (`SelfReference`), reachability via an `inbound` set (`UnreachableTeam` for every team except the first declared). The error enum is `#[derive(Debug, Error, PartialEq, Eq)]`.
  - `src-tauri/pipeline/src/parse.rs` — `parse_pipeline(yaml) -> Result<Pipeline, PipelineParseError>` (just serde_yaml; validation is separate).
  - `src-tauri/pipeline/src/lib.rs` — `pub mod model/parse/validate/template/store/api;` with `pub use` re-exports; `#[cfg(test)] mod contract_tests;`.
- **Runtime (`runtime` crate).**
  - `src-tauri/runtime/src/task.rs` — the `Task` aggregate. `Task { id: TaskId, project_id, pipeline, topic, target_repo, target_scope, current_stage, state: TaskState, attempts, parent_artifact, review_artifact, created_at, updated_at }`. `enum TaskState { Queued, Running, Gated, Revising, NeedsHuman, Done, Braked }` (`#[serde(rename_all = "snake_case")]`, with `as_str()` + `parse()`). `Task::injected(project_id, pipeline, entry_stage, topic, target_repo, now) -> Task` (id `T-<uuid>`, queued, attempts 1). `can_transition_to` / `transition_to` / `bump_attempts` / `next_state_for_verdict`.
  - `src-tauri/runtime/src/task_store.rs` — `TaskStore { pool: SqlitePool }`. `insert`, `get`, `list_by_state`, `update` (full-row), and the atomic `claim_next_for_stage(stage, now)`: select oldest queued id, then `UPDATE tasks SET state='running'… WHERE id=? AND state='queued'` — **rows_affected == 1 = the single-writer guard**. `release_orphaned_running(now)` requeues all `running`. The `Row` tuple type + `SELECT` const enumerate columns; `row_to_task` maps them. **Adding columns means extending `Row`, `SELECT`, `insert`, `update`, `row_to_task`.** Tests use a `fresh_pool()` with `foreign_keys(false)` that runs `001_initial.sql` + `003_runtime.sql`.
  - `src-tauri/runtime/src/router.rs` — the pure `route(pipeline, stage, verdict, attempts) -> Result<Routed, RouteError>`. `Routed { next_stage, next_state, bump_attempts }`. `kind_of(p, id) -> Option<NodeKind>` and `target_to_outcome(p, target) -> Result<(String, TaskState), RouteError>` (Team⇒Queued, Gate⇒Gated, Escalation⇒NeedsHuman, literal `"done"`⇒Done, else `UnknownStage`). Gate-stage and team-stage handling. **This is where a `Join` target must be recognised.**
  - `src-tauri/runtime/src/pool.rs` — `process_one_claim(ctx, team) -> Result<ClaimOutcome, PoolError>`: brake check → claim → prepare scope → invoke runner → cleanup → publish usage → `settle_and_route`. `ClaimOutcome { Idle, Braked, Settled { task_id, next_stage, next_state }, RateLimited { task_id } }`. `settle_and_route(ctx, task, verdict)` calls `route()`, bumps attempts if asked, sets `task.state`/`current_stage`, persists via `tasks.update`. `PoolContext { pipeline, runner, tasks, brake, project_root, read_prompt, usage_sink, revision_reader }`. Tests use `FakeRunner` + an in-memory `fresh_pool()` (FKs off, 001 + 003) + `ctx_with(...)` helper + `approve_output()`.
  - `src-tauri/runtime/src/lib.rs` — `pub mod task/task_store/worker/router/brake/pool/revision/api;` with `pub use`; `#[cfg(test)] mod contract_tests;`.
  - `src-tauri/runtime/src/contract_tests.rs` — serde key-set regression tests for `Task`/`TaskState`/`BrakeState` locking the TS interfaces. **Additive only** — adding nullable Task fields adds keys here.
- **Migrations + composition root.**
  - `src-tauri/app/migrations/` — `001_initial.sql` … `005_usage.sql`. `003_runtime.sql` creates `tasks` (with `FOREIGN KEY(project_id) REFERENCES projects(id)`), `workers`, `comments`, and indexes. This plan adds `006_fanout.sql`; **001–005 are never edited.**
  - `src-tauri/app/src/lib.rs` — `run_migrations(pool)` holds `const MIGRATIONS: &[(i64, &str)]` (versions 1–5, gated by `PRAGMA user_version`) **and** a parallel `tauri-plugin-sql` `migrations` vec (1–5). **Both lists get a version-6 entry.** `spawn_worker_loops(...)` builds a `PoolContext` per team and loops `process_one_claim`. A test (`run_migrations` idempotency) asserts `user_version == 5` after a run — **that assertion bumps to 6**.
- **Workspace wiring.** `src-tauri/Cargo.toml` `[workspace] members` (unchanged — no new crate). `ddd-council.json` `schema_version: 1` is the *detector* config version, unrelated to the pipeline schema_version — **leave `ddd-council.json` untouched** (no new context, no new path; `fanout_group.rs`/`fanout_store.rs` live under the already-mapped `src-tauri/runtime/**`). See Decision D9.
- **Frontend.** `src/ipc/pipeline.ts` — TS mirrors of every model type (`Pipeline`, `Team`, `Gate`, `Escalation`, `Routes`, …). `src/ipc/pipeline.test.ts` — IPC contract tests (the `loadPipeline` test builds a literal graph object). `src/components/PipelineView.tsx` — groups nodes by kind into cards. `src/components/PipelineView.test.tsx` — viewer render tests.

---

## Spec anchors (source of truth — do not invent)

- **Schema (Pipeline Authoring):** `NodeKind` gains `Fork`, `Join`. New nodes `Fork { id, lanes: Vec<String> }` (lane = entry team id of each parallel lane) and `Join { id, waits_for: Vec<String>, downstream: String }` (waits_for = terminal team id of each lane). `Pipeline` gains `#[serde(default)] forks: Vec<Fork>` + `joins: Vec<Join>`; `node_ids()` includes them. `SCHEMA_VERSION = 2`.
- **Validation (v2 only):** every `fork.lanes[i]` and `join.waits_for[i]` resolves to a **team**; a fork has **≥2 lanes**; each lane is a **linear team chain** (no gate/escalation/fork inside a lane); `downstream` resolves to any node kind; reachability + no-orphan extended to fork/join; a **v1 pipeline containing fork/join is rejected** ("forks require schema_version: 2"). `validate.rs` accepts schema_version 1 **and** 2.
- **Runtime Task:** gains nullable `group_id: Option<String>`, `lane: Option<String>`, `join_target: Option<String>` (absent for linear tasks).
- **`FanOutGroup` aggregate (`fanout_group.rs`):** root `group_id`; holds expected lanes, each lane's settled verdict, a `completed` flag. Owns the *completes-exactly-once* barrier invariant.
- **`fanout_store.rs` — the barrier:** when a lane's terminal team approves into its `join_target`, record that lane's verdict, then attempt `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0`. rows-affected 0 ⇒ not all lanes in yet, or another settlement won ⇒ no continuation here. rows-affected 1 (this caller won, all lanes settled) ⇒ create **exactly one** continuation: at `join.downstream` if **all lanes approved**, else at **needs-human**; inherits the original task's identity/lineage. A lane settling while others still run records its verdict and parks.
- **Pool fork-handling:** when a task settles `approve` and its `on_approve` target is a **fork** node, create a `FanOutGroup` and spawn **one sibling task per lane** (shared `group_id`, each with its `lane` + `join_target`); the original task terminates as forked. Each sibling runs its lane linearly.
- **`route()`:** unchanged for teams/gates; a `Join` target resolves to a barrier `Routed` outcome the pool/store resolve.
- **Migration `006_fanout.sql`:** Task lane columns + `fanout_groups (id PK, pipeline, join_target, downstream, completed INTEGER DEFAULT 0)` + `fanout_lanes (group_id, lane, verdict)`. Append-only.
- **Crash recovery:** orphaned `running` lane tasks are released + re-claimed; the barrier re-evaluates from the persisted `FanOutGroup` (lane verdicts + `completed` flag), and the `completed` guard keeps recovery idempotent.
- **Frontend:** `PipelineView` renders fork/join; the `Pipeline` TS interface gains `forks`/`joins`; contract tests extended.
- **DOMAIN.md / config (applied in the final task):** add **Fork** / **Join** / **Lane** to Pipeline Authoring's ubiquitous language; add **Fan-out group** to Runtime's; bump the `Pipeline ↔ Runtime` shared-kernel entry to `schema_version: 2` in `docs/context-map.md`.

---

## File structure (created / modified)

```
src-tauri/
├── pipeline/src/
│   ├── model.rs                 M  Fork, Join structs; NodeKind::{Fork,Join}; Pipeline.forks/joins; node_ids(); SCHEMA_VERSION=2
│   ├── validate.rs              M  new error variants + v2 fork/join rules; accept v1 AND v2; v1-rejects-fork
│   ├── parse.rs                 M  (test only) a v2 fork/join YAML parses
│   └── contract_tests.rs        M  (if present) lock Fork/Join/Pipeline key sets — else skip
├── runtime/src/
│   ├── task.rs                  M  Task gains group_id/lane/join_target (Option<String>); injected() defaults None; new forked() ctor for siblings
│   ├── task_store.rs            M  Row/SELECT/insert/update/row_to_task carry the 3 new columns
│   ├── router.rs                M  target_to_outcome recognises NodeKind::Fork (Queued at fork) + Join (barrier sentinel)
│   ├── fanout_group.rs          A  FanOutGroup aggregate + LaneVerdict + the completes-once invariant logic (pure)
│   ├── fanout_store.rs          A  FanOutStore: create group, record lane verdict, atomic complete-once barrier
│   ├── pool.rs                  M  fork-expansion on approve-into-fork; join-barrier on approve-into-join
│   ├── lib.rs                   M  pub mod fanout_group; pub mod fanout_store; pub use
│   └── contract_tests.rs        M  Task key set += group_id/lane/join_target
├── app/
│   ├── migrations/006_fanout.sql  A  Task lane columns + fanout_groups + fanout_lanes (append-only)
│   └── src/lib.rs              M  register migration 006 in BOTH lists; bump idempotency-test assertion to 6
src/
├── ipc/pipeline.ts             M  Fork/Join TS interfaces; Pipeline gains forks/joins
├── ipc/pipeline.test.ts        M  loadPipeline graph literal carries forks/joins
├── components/PipelineView.tsx M  render Forks + Joins sections
└── components/PipelineView.test.tsx  M  asserts fork/join render
DOMAIN.md                       M  Fork/Join/Lane (Authoring) + Fan-out group (Runtime) ubiquitous language
docs/context-map.md             M  Pipeline↔Runtime shared kernel → schema_version: 2  (if the file exists)
```

---

## Decisions (resolved ambiguities — autonomous)

- **D1 — `route()` signals the barrier with a new `TaskState::Joining` sentinel.** Adding a `Routed` enum variant would force every existing match on `Routed` to change. Instead `route()` returns the existing `Routed { next_stage, next_state, bump_attempts }` shape, where a `Join` target yields `next_stage = <join id>`, `next_state = TaskState::Joining` (a **new, eighth** TaskState, distinct from the seven lifecycle states). The pool sees `next_state == Joining` after settle and hands off to the `FanOutStore` barrier instead of persisting the task. `Joining` is never persisted as a resting task state — it is the in-process signal "this approve hit the join, resolve the barrier". This keeps `route()` total and pure (F3) and keeps the seven persisted states intact. `TaskState::Joining` parses/serialises to `"joining"` for completeness but no task row ever rests in it.
- **D2 — Atomic group-completion is a single conditional UPDATE owning the invariant.** `FanOutStore::record_and_try_complete(group_id, lane, verdict)` runs in two steps: (1) upsert the lane's verdict into `fanout_lanes` (`INSERT … ON CONFLICT(group_id, lane) DO UPDATE SET verdict=excluded.verdict`), then (2) **only if** all expected lanes now have a recorded verdict, attempt `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0`. `rows_affected() == 1` ⇒ this caller is the sole winner and must create the continuation; `== 0` ⇒ lanes still outstanding **or** another settlement already won ⇒ this caller parks its lane and creates nothing. This is the exact `claim_next_for_stage` single-writer pattern, now owning the `FanOutGroup` *completes-exactly-once* invariant (F1). The "all lanes recorded?" check is a count read, but it is **not** load-bearing for correctness — the `WHERE completed=0` guard is. The count only avoids attempting completion before the last lane.
- **D3 — Lane sibling tasks are full Task rows sharing a `group_id`.** A new `Task::forked(parent, lane_entry_team, group_id, join_target, now)` constructor clones the parent's `project_id`/`pipeline`/`topic`/`target_repo`/`target_scope`/`parent_artifact`, mints a fresh `T-<uuid>` id, sets `current_stage = lane_entry_team`, `state = Queued`, `attempts = 1`, and stamps `group_id`/`lane`/`join_target`. Each sibling then runs the normal claim/settle/route loop (its lane is a linear team chain). The original task is terminated `Done` (forked) once siblings are spawned — its work is finished; the continuation past the join is a *new* task created by the barrier.
- **D4 — The barrier continuation inherits identity via the group, not the lane task.** `fanout_groups` stores `pipeline`, `join_target`, `downstream`. The continuation task is built from the **last-settling lane task** (which carries `project_id`/`pipeline`/`topic`/`target_repo` — all shared across siblings) plus the group's `downstream` (all-approve) or the literal `needs-human` escalation id (any reject/exhaust). It is a fresh `T-<uuid>` with `group_id = None` (it is past the join, back in linear flow), `state = Queued` at `downstream` (or `NeedsHuman` at `needs-human`).
- **D5 — Lane verdict is reduced to approve/not-approve at the barrier.** A lane's terminal team can only reach its `join_target` by **approving** into it (revise loops back within the lane; revise-at-cap and reject route the lane task to `needs-human`, which is **not** the join — that lane never records an approve). So the barrier records a lane verdict of `"approve"` when the lane task approves into the join, and the *absence* of an approve (the lane went to needs-human instead) is what makes the join not-all-approve. **Resolution:** the lane records its verdict into `fanout_lanes` at two moments — (a) on approve-into-join (`verdict = "approve"`), and (b) when a lane task routes to `needs-human` *while carrying a `group_id`* (`verdict = "reject"`). Both moments call `record_and_try_complete`, so a failed lane also advances the barrier (a full barrier: it waits for **all** lanes to settle either way, then aggregates). All-approve ⇒ `downstream`; any `reject` ⇒ `needs-human`.
- **D6 — Crash recovery re-evaluates the group, not the dead worker.** `release_orphaned_running` already requeues orphaned `running` lane tasks; a re-claim re-runs the lane and re-attempts `record_and_try_complete`. The `INSERT … ON CONFLICT … DO UPDATE` makes re-recording a lane's verdict idempotent, and the `WHERE completed=0` guard makes a second completion attempt a no-op (`rows_affected == 0`). So a crash mid-group, on restart, converges to exactly one continuation. No separate recovery scan is needed in v1 — the existing requeue + the idempotent barrier suffice (tested in Task 12).
- **D7 — `Joining` does not widen the persisted state machine.** `can_transition_to` is **not** taught any `Joining` edges; `Joining` is purely the router→pool in-process signal (D1). The pool never calls `transition_to(Joining)` — on an approve-into-join it bypasses `settle_and_route`'s persist and routes into the barrier. This keeps the Task aggregate's seven-state invariant untouched.
- **D8 — Lane linearity is validated structurally, not by graph walk.** A lane "contains no gate/escalation/fork" is checked by walking each lane from its `fork.lanes[i]` entry team, following `on_approve` until reaching the paired join's id, and asserting every intermediate node is a **team**. To keep v1 simple and avoid infinite loops, the walk is bounded by the team count and any non-team / unresolved hop (other than the terminal join) is the `LaneNotLinear` error. A fork/gate/escalation encountered mid-lane trips it.
- **D9 — `ddd-council.json` is untouched.** Its `schema_version: 1` is the *detector config* version (DOMAIN.md "Notes for the detector"), not the pipeline schema. No new context or path is introduced — `fanout_group.rs`/`fanout_store.rs` live under the already-mapped `src-tauri/runtime/**`. The only schema-version bump that matters here is `pipeline::model::SCHEMA_VERSION` and the `docs/context-map.md` kernel note (Task 14).
- **D10 — Fork node has no `Routes`.** A `Fork`/`Join` is a node *kind* but is not a `Team`, so it has no `outputs: Routes`. A team reaches a fork via its `on_approve` pointing at the fork id; the fork's own outedges are its `lanes`. A join's outedge is its single `downstream`. This preserves "routes stay single-target" — multiplicity lives only in `Fork.lanes`.

---

## Task 1: Schema — `Fork` / `Join` structs + `NodeKind` + `Pipeline` fields + `node_ids()` + `SCHEMA_VERSION = 2`

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs`

- [ ] **Step 1: Write the failing test**

Append these tests to the `mod tests` block at the bottom of `src-tauri/pipeline/src/model.rs` (after `pipeline_round_trips_through_serde_json`):

```rust
    #[test]
    fn schema_version_constant_is_two() {
        assert_eq!(SCHEMA_VERSION, 2);
    }

    #[test]
    fn node_ids_includes_forks_and_joins() {
        let p = Pipeline {
            id: "p".into(),
            name: "P".into(),
            description: String::new(),
            schema_version: 2,
            teams: vec![sample_team("research")],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["a".into(), "b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "research".into() }],
        };
        let ids = p.node_ids();
        assert!(ids.contains(&("fork-1".into(), NodeKind::Fork)));
        assert!(ids.contains(&("join-1".into(), NodeKind::Join)));
    }

    #[test]
    fn forks_and_joins_default_to_empty_when_absent() {
        // A v1-shaped JSON (no forks/joins keys) still deserialises.
        let json = r#"{"id":"p","name":"P","schema_version":1,"teams":[],"gates":[],"escalations":[]}"#;
        let p: Pipeline = serde_json::from_str(json).unwrap();
        assert!(p.forks.is_empty());
        assert!(p.joins.is_empty());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib model::`
Expected: FAIL — compile errors: `cannot find type Fork`, `cannot find type Join`, `no variant Fork`, missing fields `forks`/`joins`, and `schema_version_constant_is_two` assertion fails / `schema_version_constant_is_one` now stale.

- [ ] **Step 3: Bump the schema version**

In `src-tauri/pipeline/src/model.rs`, change the constant:

```rust
pub const SCHEMA_VERSION: u32 = 2;
```

And update the now-stale existing test `schema_version_constant_is_one` to match — rename it and fix the assertion (it asserted `== 1`):

```rust
    #[test]
    fn schema_version_constant_is_two_existing() {
        assert_eq!(SCHEMA_VERSION, 2);
    }
```

(Delete the old `schema_version_constant_is_one` body — keep only one constant test; the new `schema_version_constant_is_two` from Step 1 may duplicate it, in which case remove this stale one entirely.)

- [ ] **Step 4: Add the `Fork` and `Join` structs**

In `src-tauri/pipeline/src/model.rs`, after the `Escalation` struct (before `enum NodeKind`), add:

```rust
/// A fork node: fans one task out into parallel **lanes** (DOMAIN.md — "a node
/// that fans one task out into parallel lanes"). Each entry in `lanes` is the
/// entry team id of one lane. A team reaches a fork via its `on_approve`
/// pointing at the fork id; the fork's own out-edges are its lanes — so routes
/// stay single-target and multiplicity lives only here (Decision D10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fork {
    pub id: String,
    /// The entry team id of each parallel lane. A valid fork has ≥2 (validate.rs).
    pub lanes: Vec<String>,
}

/// A join node: a barrier that waits for all lanes, then continues — all must
/// approve, else needs-human (DOMAIN.md). `waits_for` is the terminal team id of
/// each lane; `downstream` is the single node the joined task proceeds to when
/// every lane approves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Join {
    pub id: String,
    /// The terminal team id of each lane (the team that approves into this join).
    pub waits_for: Vec<String>,
    /// Single-target downstream (routes stay single-target).
    pub downstream: String,
}
```

- [ ] **Step 5: Extend `NodeKind`**

Change the `NodeKind` enum to include the two new kinds:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Team,
    Gate,
    Escalation,
    Fork,
    Join,
}
```

- [ ] **Step 6: Add the `Pipeline` fields**

In the `Pipeline` struct, after the `escalations` field, add:

```rust
    #[serde(default)]
    pub forks: Vec<Fork>,
    #[serde(default)]
    pub joins: Vec<Join>,
```

- [ ] **Step 7: Extend `node_ids()`**

In `impl Pipeline`, extend `node_ids()` so it appends forks and joins:

```rust
    pub fn node_ids(&self) -> Vec<(String, NodeKind)> {
        let mut ids = Vec::new();
        ids.extend(self.teams.iter().map(|t| (t.id.clone(), NodeKind::Team)));
        ids.extend(self.gates.iter().map(|g| (g.id.clone(), NodeKind::Gate)));
        ids.extend(self.escalations.iter().map(|e| (e.id.clone(), NodeKind::Escalation)));
        ids.extend(self.forks.iter().map(|f| (f.id.clone(), NodeKind::Fork)));
        ids.extend(self.joins.iter().map(|j| (j.id.clone(), NodeKind::Join)));
        ids
    }
```

- [ ] **Step 8: Fix the existing `node_ids_collects_all_three_kinds` + round-trip tests**

The existing tests `node_ids_collects_all_three_kinds` and `pipeline_round_trips_through_serde_json` construct `Pipeline { … }` literals **without** `forks`/`joins` — they now fail to compile. Add `forks: vec![], joins: vec![],` to both literals (and any other `Pipeline { … }` literal in this file). Update the `schema_version: 1` in those literals to `2` is **not** required (the struct accepts any number; only validate.rs gates it) — leave them as-is.

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p pipeline --lib model::`
Expected: PASS — all `model::` tests green, including `schema_version_constant_is_two`, `node_ids_includes_forks_and_joins`, `forks_and_joins_default_to_empty_when_absent`.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/pipeline/src/model.rs
git commit -m "feat(pipeline): add Fork/Join node kinds + Pipeline.forks/joins; bump SCHEMA_VERSION to 2

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Validation — accept v1 AND v2, reject v1-with-fork/join

**Files:**
- Modify: `src-tauri/pipeline/src/validate.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/pipeline/src/validate.rs` `mod tests`, the existing `valid_pipeline()` helper builds a v1 pipeline; we need v2 helpers. Add to the test module (after the existing `valid_pipeline` fn):

```rust
    use crate::model::{Fork, Join};

    fn lane_team(id: &str, approve: &str) -> Team {
        let mut t = team(id, Some(approve));
        t.outputs.on_revise = None;
        t.outputs.on_reject = Some("needs-human".into());
        t
    }

    /// A valid v2 fork/join pipeline: entry team -> fork -> {lane-a, lane-b} -> join -> done team.
    fn valid_v2_pipeline() -> Pipeline {
        Pipeline {
            id: "p".into(),
            name: "P".into(),
            description: String::new(),
            schema_version: 2,
            teams: vec![
                team("entry", Some("fork-1")),
                lane_team("lane-a", "join-1"),
                lane_team("lane-b", "join-1"),
                team("after", Some("needs-human")),
            ],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into() }],
        }
    }

    #[test]
    fn schema_version_one_is_still_accepted() {
        // valid_pipeline() is v1 with no forks/joins.
        assert_eq!(validate(&valid_pipeline()), Ok(()));
    }

    #[test]
    fn schema_version_two_is_accepted() {
        assert_eq!(validate(&valid_v2_pipeline()), Ok(()));
    }

    #[test]
    fn an_unsupported_version_is_still_rejected() {
        let mut p = valid_v2_pipeline();
        p.schema_version = 99;
        assert!(matches!(validate(&p), Err(PipelineValidationError::UnsupportedSchemaVersion { found: 99, .. })));
    }

    #[test]
    fn v1_with_a_fork_is_rejected() {
        let mut p = valid_v2_pipeline();
        p.schema_version = 1;
        assert_eq!(validate(&p), Err(PipelineValidationError::ForkRequiresV2));
    }
```

Also fix the existing `unsupported_schema_version_is_rejected` test — it asserts `supported: 1`; the error no longer carries a single `supported`. **Resolution:** change that test to match the new `{ found: 99, .. }` shape (see Step 3 for the new error fields). Replace its body with:

```rust
    #[test]
    fn unsupported_schema_version_is_rejected() {
        let mut p = valid_pipeline();
        p.schema_version = 99;
        assert!(matches!(
            validate(&p),
            Err(PipelineValidationError::UnsupportedSchemaVersion { found: 99, .. })
        ));
    }
```

Also fix the existing `valid_pipeline()` literal — it lacks `forks`/`joins`. Add `forks: vec![], joins: vec![],` to it.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p pipeline --lib validate::`
Expected: FAIL — compile errors: missing `forks`/`joins` in literals, `no variant ForkRequiresV2`, and the version-gate logic still rejects v1+fork as well-formed/rejects v2 as unsupported.

- [ ] **Step 3: Add the new error variants + the version gate + the v1-rejects-fork rule**

In `src-tauri/pipeline/src/validate.rs`, replace the `UnsupportedSchemaVersion` variant and add the new variants to `PipelineValidationError`:

```rust
    #[error("unsupported schema_version {found} (this build supports 1 and {max})")]
    UnsupportedSchemaVersion { found: u32, max: u32 },
    #[error("fork/join nodes require schema_version: 2")]
    ForkRequiresV2,
    #[error("fork '{fork}' lane '{lane}' does not resolve to a team")]
    ForkLaneNotTeam { fork: String, lane: String },
    #[error("join '{join}' waits_for '{team}' does not resolve to a team")]
    JoinWaitsForNotTeam { join: String, team: String },
    #[error("fork '{0}' must have at least 2 lanes")]
    ForkTooFewLanes(String),
    #[error("join '{join}' downstream points at unknown node '{target}'")]
    UnresolvedJoinDownstream { join: String, target: String },
    #[error("lane entered at '{entry}' is not linear (encountered non-team '{node}' before join '{join}')")]
    LaneNotLinear { entry: String, node: String, join: String },
    #[error("fork '{fork}' lanes do not match a join's waits_for")]
    ForkJoinMismatch { fork: String },
```

Replace the schema-version check at the top of `validate()`:

```rust
    // schema_version supported: this build supports 1 and SCHEMA_VERSION (=2).
    if p.schema_version != 1 && p.schema_version != SCHEMA_VERSION {
        return Err(PipelineValidationError::UnsupportedSchemaVersion {
            found: p.schema_version,
            max: SCHEMA_VERSION,
        });
    }

    // Only v2 pipelines may contain fork/join nodes.
    if p.schema_version < 2 && (!p.forks.is_empty() || !p.joins.is_empty()) {
        return Err(PipelineValidationError::ForkRequiresV2);
    }
```

- [ ] **Step 4: Run the tests to verify they pass (so far)**

Run: `cd src-tauri && cargo test -p pipeline --lib validate::`
Expected: PASS for `schema_version_*`, `v1_with_a_fork_is_rejected`, `unsupported_schema_version_is_rejected`, `an_unsupported_version_is_still_rejected`, `schema_version_two_is_accepted`. (The lane/resolve rules come in Task 3; `valid_v2_pipeline` already passes the existing reachability since fork/join ids are registered next.)

NOTE: `schema_version_two_is_accepted` may still fail until Task 3 wires fork/join ids into `inbound`/`kinds`. If it fails here with `UnreachableTeam`/`UnresolvedRoute`, that is expected — leave the assertion; Task 3 makes it green. To keep this task self-contained-green, mark `schema_version_two_is_accepted` with `#[ignore]` here and remove the `#[ignore]` in Task 3 Step 1.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/validate.rs
git commit -m "feat(pipeline): validate accepts schema_version 1 and 2; reject fork/join on v1

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Validation — fork/join resolve to teams, ≥2 lanes, reachability, downstream

**Files:**
- Modify: `src-tauri/pipeline/src/validate.rs`

- [ ] **Step 1: Write the failing tests**

Remove the `#[ignore]` from `schema_version_two_is_accepted` (added in Task 2 Step 4). Add these tests to `mod tests`:

```rust
    #[test]
    fn fork_lane_must_resolve_to_a_team() {
        let mut p = valid_v2_pipeline();
        p.forks[0].lanes[0] = "ghost".into();
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::ForkLaneNotTeam { fork: "fork-1".into(), lane: "ghost".into() })
        );
    }

    #[test]
    fn join_waits_for_must_resolve_to_a_team() {
        let mut p = valid_v2_pipeline();
        p.joins[0].waits_for[1] = "ghost".into();
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::JoinWaitsForNotTeam { join: "join-1".into(), team: "ghost".into() })
        );
    }

    #[test]
    fn fork_with_one_lane_is_rejected() {
        let mut p = valid_v2_pipeline();
        p.forks[0].lanes = vec!["lane-a".into()];
        p.joins[0].waits_for = vec!["lane-a".into()];
        assert_eq!(validate(&p), Err(PipelineValidationError::ForkTooFewLanes("fork-1".into())));
    }

    #[test]
    fn join_downstream_must_resolve() {
        let mut p = valid_v2_pipeline();
        p.joins[0].downstream = "ghost".into();
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::UnresolvedJoinDownstream { join: "join-1".into(), target: "ghost".into() })
        );
    }

    #[test]
    fn fork_target_team_is_reachable_via_fork_lane() {
        // lane-a / lane-b receive no team on_approve edge; their only inbound is
        // the fork's lanes. They must still count as reachable.
        let p = valid_v2_pipeline();
        assert_eq!(validate(&p), Ok(()));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p pipeline --lib validate::`
Expected: FAIL — the new resolve/lane-count/downstream errors aren't produced yet; `fork_target_team_is_reachable_via_fork_lane`/`schema_version_two_is_accepted` may fail with `UnreachableTeam` because fork lanes don't yet add inbound edges.

- [ ] **Step 3: Implement the fork/join resolution + reachability rules**

In `src-tauri/pipeline/src/validate.rs`, after the existing gate loop (after the `for gate in &p.gates { … }` block, before the reachability loop), insert:

```rust
    // Fork/join structural rules (v2). The unique-id pass above already registered
    // fork/join ids in `kinds`, so a `Team`-kind check is exact.
    let is_team = |id: &str| kinds.get(id) == Some(&NodeKind::Team);

    for fork in &p.forks {
        if fork.lanes.len() < 2 {
            return Err(PipelineValidationError::ForkTooFewLanes(fork.id.clone()));
        }
        for lane in &fork.lanes {
            if !is_team(lane) {
                return Err(PipelineValidationError::ForkLaneNotTeam {
                    fork: fork.id.clone(),
                    lane: lane.clone(),
                });
            }
            // A fork lane is an inbound edge into its entry team.
            inbound.insert(lane.as_str());
        }
    }

    for join in &p.joins {
        for team in &join.waits_for {
            if !is_team(team) {
                return Err(PipelineValidationError::JoinWaitsForNotTeam {
                    join: join.id.clone(),
                    team: team.clone(),
                });
            }
        }
        if !kinds.contains_key(join.downstream.as_str()) {
            return Err(PipelineValidationError::UnresolvedJoinDownstream {
                join: join.id.clone(),
                target: join.downstream.clone(),
            });
        }
        inbound.insert(join.downstream.as_str());
        // A join id receives inbound edges from its lanes' terminal teams
        // (their on_approve -> join). Register join itself as reachable so the
        // no-orphan pass (if extended to joins later) is satisfied.
        inbound.insert(join.id.as_str());
    }
```

Note: the existing team-route loop already calls `inbound.insert(target)` for any `on_approve` pointing at a fork or join id (they are in `kinds` now), so forks/joins receive inbound edges automatically.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p pipeline --lib validate::`
Expected: PASS — all resolve/lane-count/downstream/reachability tests green, `schema_version_two_is_accepted` green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/validate.rs
git commit -m "feat(pipeline): validate fork/join resolve to teams, >=2 lanes, downstream + reachability

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Validation — lane linearity (no gate/escalation/fork inside a lane)

**Files:**
- Modify: `src-tauri/pipeline/src/validate.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
    use crate::model::Gate as GateNode;

    #[test]
    fn a_gate_inside_a_lane_is_rejected() {
        let mut p = valid_v2_pipeline();
        // Make lane-a route through a gate before the join: lane-a -> gate-x -> join-1.
        p.teams[1].outputs.on_approve = Some("gate-x".into());
        p.gates.push(GateNode { id: "gate-x".into(), label: "X".into(), downstream: "join-1".into() });
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::LaneNotLinear { entry: "lane-a".into(), node: "gate-x".into(), join: "join-1".into() })
        );
    }

    #[test]
    fn a_nested_fork_inside_a_lane_is_rejected() {
        let mut p = valid_v2_pipeline();
        // lane-a -> fork-2 (nested fork) is illegal.
        p.teams[1].outputs.on_approve = Some("fork-2".into());
        p.forks.push(Fork { id: "fork-2".into(), lanes: vec!["lane-b".into(), "after".into()] });
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::LaneNotLinear { entry: "lane-a".into(), node: "fork-2".into(), join: "join-1".into() })
        );
    }

    #[test]
    fn a_multi_team_linear_lane_is_accepted() {
        let mut p = valid_v2_pipeline();
        // lane-a -> lane-a2 -> join-1 (two teams in the lane, still linear).
        p.teams[1].outputs.on_approve = Some("lane-a2".into());
        p.teams.push(lane_team("lane-a2", "join-1"));
        p.joins[0].waits_for = vec!["lane-a2".into(), "lane-b".into()];
        assert_eq!(validate(&p), Ok(()));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p pipeline --lib validate::`
Expected: FAIL — `LaneNotLinear` is never produced; the gate/nested-fork lanes are wrongly accepted (or fail with a different error).

- [ ] **Step 3: Implement the lane-walk linearity check (Decision D8)**

In `src-tauri/pipeline/src/validate.rs`, add a helper above `validate()`:

```rust
/// Walk a single lane from its entry team, following `on_approve`, until the
/// paired join id is reached. Every intermediate hop must be a team (D8). The
/// walk is bounded by team count to avoid loops.
fn check_lane_linear(
    p: &Pipeline,
    kinds: &HashMap<&str, NodeKind>,
    entry: &str,
    join_id: &str,
) -> Result<(), PipelineValidationError> {
    let mut current = entry.to_string();
    for _ in 0..=p.teams.len() {
        if current == join_id {
            return Ok(());
        }
        // The current hop must be a team (the entry and every intermediate).
        if kinds.get(current.as_str()) != Some(&NodeKind::Team) {
            return Err(PipelineValidationError::LaneNotLinear {
                entry: entry.to_string(),
                node: current.clone(),
                join: join_id.to_string(),
            });
        }
        let team = p.teams.iter().find(|t| t.id == current).unwrap();
        match team.outputs.on_approve.as_deref() {
            Some(next) => current = next.to_string(),
            // No on_approve and not the join => lane never reaches the join.
            None => {
                return Err(PipelineValidationError::LaneNotLinear {
                    entry: entry.to_string(),
                    node: current.clone(),
                    join: join_id.to_string(),
                })
            }
        }
    }
    // Bound exceeded without hitting the join: treat as non-linear.
    Err(PipelineValidationError::LaneNotLinear {
        entry: entry.to_string(),
        node: current,
        join: join_id.to_string(),
    })
}
```

Then, inside `validate()`, in the `for join in &p.joins` loop (after the downstream check), pair each fork's lanes with this join and walk them. Since a fork and join pair by matching lane sets, find the fork whose lanes' terminal teams are this join's `waits_for`. **Simpler v1 rule:** every fork pairs with exactly one join, matched by `join.waits_for` being the terminal teams of `fork.lanes`. Replace the lane-walk wiring — add this block **after** the fork loop and **before** the join loop's downstream insert is fine; cleanest is a dedicated pass after both loops:

```rust
    // Lane linearity: for each fork, walk every lane to its paired join. The
    // paired join is the one whose `waits_for` size matches the fork's lanes and
    // that each lane reaches. v1 assumes one fork ↔ one join.
    for fork in &p.forks {
        // Find the join this fork feeds: a join such that walking every lane
        // reaches it. Try each join; the first all-lanes-reach match is the pair.
        let paired = p.joins.iter().find(|j| {
            fork.lanes.iter().all(|lane| check_lane_linear(p, &kinds, lane, &j.id).is_ok())
        });
        match paired {
            Some(_join) => {} // all lanes are linear to this join
            None => {
                // No join is reachable linearly from all lanes — surface the
                // first lane's failure against the fork's first declared join (or
                // a generic mismatch when there are no joins).
                if let Some(j) = p.joins.first() {
                    for lane in &fork.lanes {
                        check_lane_linear(p, &kinds, lane, &j.id)?;
                    }
                }
                return Err(PipelineValidationError::ForkJoinMismatch { fork: fork.id.clone() });
            }
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p pipeline --lib validate::`
Expected: PASS — `a_gate_inside_a_lane_is_rejected`, `a_nested_fork_inside_a_lane_is_rejected`, `a_multi_team_linear_lane_is_accepted`, and all prior tests green.

- [ ] **Step 5: Run the whole pipeline crate to confirm no regressions**

Run: `cd src-tauri && cargo test -p pipeline`
Expected: PASS — every pipeline test (model, parse, validate, template, store, contract) green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/validate.rs
git commit -m "feat(pipeline): validate lane linearity (no gate/escalation/fork inside a lane)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Parse — a v2 fork/join YAML round-trips

**Files:**
- Modify: `src-tauri/pipeline/src/parse.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/parse.rs` `mod tests`:

```rust
    const V2_FORKJOIN: &str = r#"
id: parallel-demo
name: Parallel Demo
schema_version: 2
teams:
  - id: entry
    name: Entry
    prompt: prompts/entry.md
    runner: { kind: claude-cli, model: m, effort: { mode: standard } }
    scope: {}
    outputs: { on_approve: fork-1 }
  - id: reviewer-a
    name: Reviewer A
    prompt: prompts/a.md
    runner: { kind: claude-cli, model: m, effort: { mode: standard } }
    scope: {}
    outputs: { on_approve: join-1 }
  - id: reviewer-b
    name: Reviewer B
    prompt: prompts/b.md
    runner: { kind: claude-cli, model: m, effort: { mode: standard } }
    scope: {}
    outputs: { on_approve: join-1 }
forks:
  - id: fork-1
    lanes: [reviewer-a, reviewer-b]
joins:
  - id: join-1
    waits_for: [reviewer-a, reviewer-b]
    downstream: needs-human
escalations:
  - id: needs-human
"#;

    #[test]
    fn parses_a_v2_fork_join_pipeline() {
        let p = parse_pipeline(V2_FORKJOIN).unwrap();
        assert_eq!(p.schema_version, 2);
        assert_eq!(p.forks.len(), 1);
        assert_eq!(p.forks[0].lanes, vec!["reviewer-a", "reviewer-b"]);
        assert_eq!(p.joins.len(), 1);
        assert_eq!(p.joins[0].waits_for, vec!["reviewer-a", "reviewer-b"]);
        assert_eq!(p.joins[0].downstream, "needs-human");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib parse::parses_a_v2_fork_join_pipeline`
Expected: FAIL — at the time of writing the test is added before confirming; it should actually PASS once Task 1 added the fields. If it FAILS it is a serde shape bug — fix the YAML to match the structs. (This task is a guard test; if it passes immediately, that is acceptable — the test still locks the wire shape.)

- [ ] **Step 3: Confirm it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib parse::`
Expected: PASS — `parses_a_v2_fork_join_pipeline` + existing parse tests green.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/pipeline/src/parse.rs
git commit -m "test(pipeline): a v2 fork/join YAML parses into the typed model

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Migration `006_fanout.sql` — Task lane columns + fanout tables

**Files:**
- Create: `src-tauri/app/migrations/006_fanout.sql`
- Modify: `src-tauri/app/src/lib.rs` (register migration 6 in both lists; bump idempotency assertion)

- [ ] **Step 1: Create the migration file**

Create `src-tauri/app/migrations/006_fanout.sql`:

```sql
-- 006_fanout.sql — Sub-project 2 (parallel flow: fork/join). Adds the Task lane
-- columns and the fan-out group barrier tables. Append-only; migrations 001–005
-- are never edited. The fanout_groups.completed flag is the single-row guard for
-- the *completes-exactly-once* barrier invariant (FanOutGroup aggregate).

-- Lane membership on a task. NULL for ordinary linear tasks; set on the sibling
-- tasks a fork spawns.
ALTER TABLE tasks ADD COLUMN group_id    TEXT;   -- the fan-out group this task belongs to
ALTER TABLE tasks ADD COLUMN lane        TEXT;   -- which lane (entry team id)
ALTER TABLE tasks ADD COLUMN join_target TEXT;   -- the join node this lane reports to

-- The fan-out group: the consistency boundary for the barrier. One row per fork
-- expansion. `completed` is the conditional-UPDATE guard.
CREATE TABLE IF NOT EXISTS fanout_groups (
  id           TEXT PRIMARY KEY,
  pipeline     TEXT NOT NULL,
  join_target  TEXT NOT NULL,          -- the join id all lanes report to
  downstream   TEXT NOT NULL,          -- where an all-approve continuation goes
  completed    INTEGER NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL DEFAULT 0
);

-- One row per lane in a group, carrying that lane's settled verdict. Unique on
-- (group_id, lane) so re-recording a lane (crash recovery) is idempotent.
CREATE TABLE IF NOT EXISTS fanout_lanes (
  group_id  TEXT NOT NULL,
  lane      TEXT NOT NULL,
  verdict   TEXT NOT NULL,             -- 'approve' | 'reject'
  PRIMARY KEY (group_id, lane),
  FOREIGN KEY (group_id) REFERENCES fanout_groups(id)
);

CREATE INDEX IF NOT EXISTS idx_tasks_group ON tasks(group_id);
CREATE INDEX IF NOT EXISTS idx_fanout_lanes_group ON fanout_lanes(group_id);
```

- [ ] **Step 2: Register migration 6 in both lists in `app/src/lib.rs`**

In `src-tauri/app/src/lib.rs`, in `run_migrations`'s `const MIGRATIONS`, append:

```rust
        (6, include_str!("../migrations/006_fanout.sql")),
```

And in the `tauri-plugin-sql` `migrations` vec (around line 257, after the `005_usage.sql` entry), append a matching `Migration { version: 6, description: "fanout — task lane columns + fanout_groups/fanout_lanes", sql: include_str!("../migrations/006_fanout.sql"), kind: MigrationKind::Up }` (copy the exact struct shape of the 005 entry).

- [ ] **Step 3: Bump the idempotency-test assertion**

Find the `run_migrations` idempotency test (asserts `version == 5`, around line 627) and change it to `6`:

```rust
        assert_eq!(version, 6, "all six migrations recorded");
```

- [ ] **Step 4: Run the app migration test**

Run: `cd src-tauri && cargo test -p app run_migrations`
Expected: PASS — migrations apply idempotently; `user_version == 6`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/migrations/006_fanout.sql src-tauri/app/src/lib.rs
git commit -m "feat(app): migration 006 — task lane columns + fanout_groups/fanout_lanes

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Task aggregate — lane fields + `forked()` constructor + `Joining` state

**Files:**
- Modify: `src-tauri/runtime/src/task.rs`

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/runtime/src/task.rs` `mod tests`:

```rust
    #[test]
    fn injected_task_has_no_lane_fields() {
        let task = t();
        assert_eq!(task.group_id, None);
        assert_eq!(task.lane, None);
        assert_eq!(task.join_target, None);
    }

    #[test]
    fn forked_sibling_inherits_lineage_and_carries_lane_fields() {
        let parent = t(); // research, project p, pipeline pipe, topic topic
        let sib = Task::forked(&parent, "lane-a", "G-1", "join-1", 500);
        assert_ne!(sib.id, parent.id);
        assert!(sib.id.0.starts_with("T-"));
        assert_eq!(sib.project_id, parent.project_id);
        assert_eq!(sib.pipeline, parent.pipeline);
        assert_eq!(sib.topic, parent.topic);
        assert_eq!(sib.current_stage, "lane-a");
        assert_eq!(sib.state, TaskState::Queued);
        assert_eq!(sib.attempts, 1);
        assert_eq!(sib.group_id.as_deref(), Some("G-1"));
        assert_eq!(sib.lane.as_deref(), Some("lane-a"));
        assert_eq!(sib.join_target.as_deref(), Some("join-1"));
    }

    #[test]
    fn joining_state_string_round_trips() {
        assert_eq!(TaskState::parse("joining"), Some(TaskState::Joining));
        assert_eq!(TaskState::Joining.as_str(), "joining");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime --lib task::`
Expected: FAIL — `no variant Joining`, `no function forked`, missing fields `group_id`/`lane`/`join_target`.

- [ ] **Step 3: Add the `Joining` state**

In `src-tauri/runtime/src/task.rs`, add `Joining` to `TaskState` (after `Braked`):

```rust
    Braked,
    /// Transient router→pool signal: an approve hit a join; the pool resolves the
    /// barrier. Never rests on a persisted task row (Decision D1/D7). Serialises
    /// to "joining" for completeness only.
    Joining,
```

Extend `as_str()`:

```rust
            TaskState::Braked => "braked",
            TaskState::Joining => "joining",
```

Extend `parse()`:

```rust
            "braked" => TaskState::Braked,
            "joining" => TaskState::Joining,
```

Note: do **not** add any `Joining` edge to `can_transition_to` (D7) — it is never persisted.

- [ ] **Step 4: Add the lane fields to `Task`**

In the `Task` struct, after `updated_at`, add:

```rust
    pub updated_at: i64,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub lane: Option<String>,
    #[serde(default)]
    pub join_target: Option<String>,
```

- [ ] **Step 5: Default the fields in `injected()` and add `forked()`**

In `Task::injected`, add the three fields to the returned struct literal:

```rust
            created_at: now_unix,
            updated_at: now_unix,
            group_id: None,
            lane: None,
            join_target: None,
```

Add the `forked` constructor in `impl Task` (after `injected`):

```rust
    /// Construct a lane sibling task for a fork expansion (Decision D3). Inherits
    /// the parent's lineage (project/pipeline/topic/repo/scope/parent_artifact),
    /// mints a fresh id, and is queued at the lane's entry team carrying its
    /// group/lane/join membership.
    pub fn forked(
        parent: &Task,
        lane_entry_team: &str,
        group_id: &str,
        join_target: &str,
        now_unix: i64,
    ) -> Self {
        Self {
            id: TaskId(format!("T-{}", uuid::Uuid::new_v4())),
            project_id: parent.project_id.clone(),
            pipeline: parent.pipeline.clone(),
            topic: parent.topic.clone(),
            target_repo: parent.target_repo.clone(),
            target_scope: parent.target_scope.clone(),
            current_stage: lane_entry_team.to_string(),
            state: TaskState::Queued,
            attempts: 1,
            parent_artifact: parent.parent_artifact.clone(),
            review_artifact: None,
            created_at: now_unix,
            updated_at: now_unix,
            group_id: Some(group_id.to_string()),
            lane: Some(lane_entry_team.to_string()),
            join_target: Some(join_target.to_string()),
        }
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime --lib task::`
Expected: PASS — all `task::` tests green.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/runtime/src/task.rs
git commit -m "feat(runtime): Task lane fields + forked() ctor + transient Joining state

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: TaskStore — persist the lane columns

**Files:**
- Modify: `src-tauri/runtime/src/task_store.rs`

- [ ] **Step 1: Update the in-memory test pool to run migration 006**

The store tests' `fresh_pool()` currently runs `001` + `003`. The lane columns live in `006`. In `src-tauri/runtime/src/task_store.rs` `mod tests`, extend `fresh_pool()` to also apply `006_fanout.sql` (after `003_runtime.sql`):

```rust
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
```

- [ ] **Step 2: Write the failing test**

Add to `mod tests`:

```rust
    #[tokio::test]
    async fn insert_get_round_trips_lane_fields() {
        let store = TaskStore::new(fresh_pool().await);
        let parent = task("research");
        let sib = Task::forked(&parent, "lane-a", "G-1", "join-1", 500);
        store.insert(&sib).await.unwrap();
        let back = store.get(&sib.id).await.unwrap();
        assert_eq!(back.group_id.as_deref(), Some("G-1"));
        assert_eq!(back.lane.as_deref(), Some("lane-a"));
        assert_eq!(back.join_target.as_deref(), Some("join-1"));
    }

    #[tokio::test]
    async fn linear_task_has_null_lane_fields() {
        let store = TaskStore::new(fresh_pool().await);
        let t = task("research");
        store.insert(&t).await.unwrap();
        let back = store.get(&t.id).await.unwrap();
        assert_eq!(back.group_id, None);
        assert_eq!(back.lane, None);
        assert_eq!(back.join_target, None);
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime --lib task_store::insert_get_round_trips_lane_fields task_store::linear_task_has_null_lane_fields`
Expected: FAIL — the columns aren't read/written; `group_id` comes back `None` even after inserting a forked task (or a column-count mismatch panics).

- [ ] **Step 4: Extend the `Row` tuple type**

In `src-tauri/runtime/src/task_store.rs`, extend the `Row` type alias with three trailing `Option<String>`:

```rust
type Row = (
    String, String, String, String, Option<String>, Option<String>,
    String, String, i64, Option<String>, Option<String>, i64, i64,
    Option<String>, Option<String>, Option<String>,
);
```

- [ ] **Step 5: Extend `row_to_task`**

Add the three fields to the `Task { … }` literal in `row_to_task`:

```rust
            created_at: r.11,
            updated_at: r.12,
            group_id: r.13,
            lane: r.14,
            join_target: r.15,
```

- [ ] **Step 6: Extend `SELECT`, `insert`, `update`**

Change the `SELECT` const to add the three columns at the end:

```rust
    const SELECT: &'static str =
        "SELECT id, project_id, pipeline, topic, target_repo, target_scope, current_stage,
         state, attempts, parent_artifact, review_artifact, created_at, updated_at,
         group_id, lane, join_target FROM tasks";
```

In `insert`, add the three columns + three `?` + three binds:

```rust
        sqlx::query(
            "INSERT INTO tasks (id, project_id, pipeline, topic, target_repo, target_scope,
             current_stage, state, attempts, parent_artifact, review_artifact, created_at, updated_at,
             group_id, lane, join_target)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        // …existing binds…
        .bind(task.created_at)
        .bind(task.updated_at)
        .bind(&task.group_id)
        .bind(&task.lane)
        .bind(&task.join_target)
        .execute(&self.pool)
        .await?;
```

In `update`, add the three columns to the `SET` clause + three binds (before the `WHERE id=?` bind):

```rust
        let res = sqlx::query(
            "UPDATE tasks SET current_stage=?, state=?, attempts=?, parent_artifact=?,
             review_artifact=?, updated_at=?, group_id=?, lane=?, join_target=? WHERE id=?",
        )
        .bind(&task.current_stage)
        .bind(task.state.as_str())
        .bind(task.attempts as i64)
        .bind(&task.parent_artifact)
        .bind(&task.review_artifact)
        .bind(task.updated_at)
        .bind(&task.group_id)
        .bind(&task.lane)
        .bind(&task.join_target)
        .bind(&task.id.0)
        .execute(&self.pool)
        .await?;
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime --lib task_store::`
Expected: PASS — all `task_store::` tests green (round-trip + null-fields + the pre-existing claim/update tests).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/runtime/src/task_store.rs
git commit -m "feat(runtime): TaskStore persists group_id/lane/join_target lane columns

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Router — recognise `Fork` and `Join` targets

**Files:**
- Modify: `src-tauri/runtime/src/router.rs`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/runtime/src/router.rs` `mod tests`, the `pipe()` helper builds a v1 pipeline literal — it needs `forks`/`joins` to compile (Task 1 added the fields). First add `forks: vec![], joins: vec![],` to the `pipe()` literal. Then add a v2 pipeline + tests:

```rust
    use pipeline::model::{Fork, Join};

    fn pipe_v2() -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 2,
            teams: vec![
                team("entry", Some("fork-1"), None, Some("needs-human")),
                team("lane-a", Some("join-1"), None, Some("needs-human")),
                team("lane-b", Some("join-1"), None, Some("needs-human")),
                team("after", Some("done"), None, Some("needs-human")),
            ],
            gates: vec![],
            escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into() }],
        }
    }

    #[test]
    fn approve_into_fork_queues_at_the_fork() {
        // The pool expands the fork; route() just resolves the fork target to a
        // queued outcome at the fork id (the pool reads NodeKind to expand).
        let r = route(&pipe_v2(), "entry", Verdict::Approve, 1).unwrap();
        assert_eq!(r.next_stage, "fork-1");
        assert_eq!(r.next_state, TaskState::Queued);
        assert!(!r.bump_attempts);
    }

    #[test]
    fn approve_into_join_yields_the_joining_barrier_outcome() {
        let r = route(&pipe_v2(), "lane-a", Verdict::Approve, 1).unwrap();
        assert_eq!(r.next_stage, "join-1");
        assert_eq!(r.next_state, TaskState::Joining);
        assert!(!r.bump_attempts);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime --lib router::`
Expected: FAIL — `target_to_outcome` returns `UnknownStage("fork-1")` / `UnknownStage("join-1")` because `kind_of` returns `Fork`/`Join` but the match has no arm for them.

- [ ] **Step 3: Extend `target_to_outcome`**

In `src-tauri/runtime/src/router.rs`, add the two arms to the `match kind_of(p, target)`:

```rust
    match kind_of(p, target) {
        Some(NodeKind::Team) => Ok((target.to_string(), TaskState::Queued)),
        Some(NodeKind::Gate) => Ok((target.to_string(), TaskState::Gated)),
        Some(NodeKind::Escalation) => Ok((target.to_string(), TaskState::NeedsHuman)),
        // A fork target queues the task at the fork id; the POOL reads the
        // NodeKind::Fork and expands it into lane siblings (route stays pure).
        Some(NodeKind::Fork) => Ok((target.to_string(), TaskState::Queued)),
        // A join target is the barrier sentinel (Decision D1/F3): the pool/store
        // resolve it. route() stays total and never returns many.
        Some(NodeKind::Join) => Ok((target.to_string(), TaskState::Joining)),
        None => {
            if target == "done" {
                Ok((target.to_string(), TaskState::Done))
            } else {
                Err(RouteError::UnknownStage(target.to_string()))
            }
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime --lib router::`
Expected: PASS — `approve_into_fork_queues_at_the_fork`, `approve_into_join_yields_the_joining_barrier_outcome`, and all existing router tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runtime/src/router.rs
git commit -m "feat(runtime): router resolves Fork (queue) + Join (Joining barrier sentinel) targets

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: `FanOutGroup` aggregate (`fanout_group.rs`) — the completes-once invariant (pure)

**Files:**
- Create: `src-tauri/runtime/src/fanout_group.rs`
- Modify: `src-tauri/runtime/src/lib.rs` (register the module)

- [ ] **Step 1: Write the failing test**

Create `src-tauri/runtime/src/fanout_group.rs` with the test module first:

```rust
//! The FanOutGroup aggregate (Runtime; root `group_id`). Owns the named
//! invariant: *"exactly one continuation task is emitted when all lanes of a
//! fan-out group have settled, and only once."* (vet F1). This module is the
//! PURE core — the expected lanes, each lane's verdict, the completed flag, and
//! the aggregation rule (all-approve ⇒ downstream, else ⇒ needs-human). The
//! atomic single-writer persistence guard lives in fanout_store.rs.

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> FanOutGroup {
        FanOutGroup {
            id: "G-1".into(),
            pipeline: "pipe".into(),
            join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into()],
            completed: false,
        }
    }

    #[test]
    fn not_all_settled_until_every_lane_has_a_verdict() {
        let g = group();
        let one = vec![LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve }];
        assert!(!g.all_lanes_settled(&one));
        let both = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Approve },
        ];
        assert!(g.all_lanes_settled(&both));
    }

    #[test]
    fn all_approve_continues_to_downstream() {
        let g = group();
        let settled = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Approve },
        ];
        assert_eq!(g.continuation(&settled), Continuation::Downstream("after".into()));
    }

    #[test]
    fn any_reject_continues_to_needs_human() {
        let g = group();
        let settled = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Reject },
        ];
        assert_eq!(g.continuation(&settled), Continuation::NeedsHuman);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p runtime --lib fanout_group::`
Expected: FAIL — module not declared / `cannot find type FanOutGroup`, `LaneVerdict`, `Continuation`.

- [ ] **Step 3: Implement the pure aggregate**

Prepend to `src-tauri/runtime/src/fanout_group.rs`:

```rust
use agent_bus_core::Verdict;
use serde::{Deserialize, Serialize};

/// One lane's settled verdict within a group. A lane reaches the barrier only by
/// approving into its join (then `Approve`), or by routing to needs-human while
/// carrying a group_id (then `Reject`) — see Decision D5. Revise never reaches
/// the barrier (it loops within the lane).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneVerdict {
    pub lane: String,
    pub verdict: Verdict,
}

/// The fan-out group aggregate root. The consistency boundary for the barrier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanOutGroup {
    pub id: String,
    pub pipeline: String,
    pub join_target: String,
    pub downstream: String,
    pub expected_lanes: Vec<String>,
    pub completed: bool,
}

/// Where the single continuation past the barrier goes once all lanes settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Continuation {
    /// All lanes approved ⇒ proceed to the join's downstream node.
    Downstream(String),
    /// Any lane rejected/exhausted ⇒ escalate the joined task.
    NeedsHuman,
}

impl FanOutGroup {
    /// True once every expected lane has a recorded verdict.
    pub fn all_lanes_settled(&self, recorded: &[LaneVerdict]) -> bool {
        self.expected_lanes
            .iter()
            .all(|lane| recorded.iter().any(|r| &r.lane == lane))
    }

    /// The continuation given all lanes' verdicts: all-approve ⇒ downstream,
    /// else ⇒ needs-human (the all-must-approve-else-needs-human rule).
    pub fn continuation(&self, recorded: &[LaneVerdict]) -> Continuation {
        let all_approve = recorded.iter().all(|r| r.verdict == Verdict::Approve);
        if all_approve && self.all_lanes_settled(recorded) {
            Continuation::Downstream(self.downstream.clone())
        } else {
            Continuation::NeedsHuman
        }
    }
}
```

- [ ] **Step 4: Register the module**

In `src-tauri/runtime/src/lib.rs`, after `pub mod pool; pub use pool::*;`, add:

```rust
pub mod fanout_group;
pub use fanout_group::*;
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p runtime --lib fanout_group::`
Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/fanout_group.rs src-tauri/runtime/src/lib.rs
git commit -m "feat(runtime): FanOutGroup aggregate — pure completes-once + all-approve aggregation

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: `FanOutStore` (`fanout_store.rs`) — the atomic barrier (THE CRUX)

**Files:**
- Create: `src-tauri/runtime/src/fanout_store.rs`
- Modify: `src-tauri/runtime/src/lib.rs` (register the module)

This is the load-bearing task: the *completes-exactly-once* barrier guarded by `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0`. Tests include the concurrent double-settle crux.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/runtime/src/fanout_store.rs` with the test module first:

```rust
//! FanOutStore — SQLite persistence + the barrier for the FanOutGroup aggregate.
//! The completes-exactly-once invariant is enforced by the same conditional-
//! UPDATE single-writer pattern claim_next_for_stage uses: `UPDATE fanout_groups
//! SET completed=1 WHERE id=? AND completed=0` — rows_affected == 1 is the sole
//! winner that creates the continuation (vet F1).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fanout_group::{Continuation, FanOutGroup};
    use agent_bus_core::Verdict;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
        pool
    }

    fn group() -> FanOutGroup {
        FanOutGroup {
            id: "G-1".into(),
            pipeline: "pipe".into(),
            join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into()],
            completed: false,
        }
    }

    #[tokio::test]
    async fn first_lane_records_and_does_not_complete() {
        let store = FanOutStore::new(fresh_pool().await);
        store.create(&group()).await.unwrap();
        let outcome = store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        assert_eq!(outcome, BarrierOutcome::Parked);
    }

    #[tokio::test]
    async fn last_lane_completes_once_with_all_approve_downstream() {
        let store = FanOutStore::new(fresh_pool().await);
        store.create(&group()).await.unwrap();
        store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        let outcome = store.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap();
        assert_eq!(outcome, BarrierOutcome::Completed(Continuation::Downstream("after".into())));
    }

    #[tokio::test]
    async fn one_reject_completes_to_needs_human() {
        let store = FanOutStore::new(fresh_pool().await);
        store.create(&group()).await.unwrap();
        store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        let outcome = store.record_and_try_complete("G-1", "lane-b", Verdict::Reject).await.unwrap();
        assert_eq!(outcome, BarrierOutcome::Completed(Continuation::NeedsHuman));
    }

    #[tokio::test]
    async fn concurrent_double_settle_yields_exactly_one_completion() {
        // THE CRUX: two callers both observe "all lanes settled" and both attempt
        // completion. The WHERE completed=0 guard means exactly one wins.
        let store = std::sync::Arc::new(FanOutStore::new(fresh_pool().await));
        store.create(&group()).await.unwrap();
        store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        // Simulate two settlements of the SAME final lane racing (e.g. a crash
        // re-claim duplicating the settle): run two record_and_try_complete for
        // lane-b concurrently.
        let s1 = store.clone();
        let s2 = store.clone();
        let h1 = tokio::spawn(async move { s1.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap() });
        let h2 = tokio::spawn(async move { s2.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap() });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());
        let completions = [&a, &b].iter().filter(|o| matches!(o, BarrierOutcome::Completed(_))).count();
        assert_eq!(completions, 1, "exactly one caller completes the barrier");
    }

    #[tokio::test]
    async fn re_recording_a_lane_is_idempotent() {
        // Crash recovery: a lane re-claims and re-records its verdict. The
        // ON CONFLICT upsert keeps it idempotent; a second completion attempt is
        // a no-op (WHERE completed=0 already 0 rows).
        let store = FanOutStore::new(fresh_pool().await);
        store.create(&group()).await.unwrap();
        store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        store.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap();
        // Re-record lane-b after completion — must not complete a second time.
        let again = store.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap();
        assert_eq!(again, BarrierOutcome::Parked);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime --lib fanout_store::`
Expected: FAIL — module not declared / `cannot find type FanOutStore`, `BarrierOutcome`.

- [ ] **Step 3: Implement the store + the atomic barrier**

Prepend to `src-tauri/runtime/src/fanout_store.rs`:

```rust
use crate::fanout_group::{Continuation, FanOutGroup, LaneVerdict};
use agent_bus_core::Verdict;
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FanOutStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("group not found: {0}")]
    NotFound(String),
    #[error("corrupt verdict string in db: {0}")]
    BadVerdict(String),
}

/// The result of recording a lane verdict at the barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarrierOutcome {
    /// Recorded; the group is not complete by this caller (lanes still
    /// outstanding, or another settlement already won). This lane parks.
    Parked,
    /// This caller is the sole winner of the completes-once guard and must
    /// create the single continuation.
    Completed(Continuation),
}

fn verdict_str(v: Verdict) -> &'static str {
    match v {
        Verdict::Approve => "approve",
        Verdict::Revise => "revise",
        Verdict::Reject => "reject",
    }
}

fn parse_verdict(s: &str) -> Option<Verdict> {
    Some(match s {
        "approve" => Verdict::Approve,
        "revise" => Verdict::Revise,
        "reject" => Verdict::Reject,
        _ => return None,
    })
}

pub struct FanOutStore {
    pool: SqlitePool,
}

impl FanOutStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Persist a new fan-out group (one row per fork expansion).
    pub async fn create(&self, g: &FanOutGroup) -> Result<(), FanOutStoreError> {
        sqlx::query(
            "INSERT INTO fanout_groups (id, pipeline, join_target, downstream, completed, created_at)
             VALUES (?,?,?,?,0,0)",
        )
        .bind(&g.id)
        .bind(&g.pipeline)
        .bind(&g.join_target)
        .bind(&g.downstream)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load a group + its expected lanes. `expected_lanes` is reconstructed from
    /// the group's stored lanes... but the group row does not store the lane set,
    /// so we read the expected lanes from a separate source. v1 stores the
    /// expected lane set implicitly: the pool seeds one `fanout_lanes` row per
    /// lane with verdict='' (pending) at create time? NO — see Decision: we store
    /// expected lanes as the keys present after seeding. To keep `create` simple,
    /// the pool passes `expected_lanes` into `record_and_try_complete` via the
    /// group load. Here we read them from fanout_lanes seeded rows.
    pub async fn load(&self, group_id: &str) -> Result<FanOutGroup, FanOutStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, i64)>(
            "SELECT pipeline, join_target, downstream, completed FROM fanout_groups WHERE id = ?",
        )
        .bind(group_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| FanOutStoreError::NotFound(group_id.to_string()))?;
        let lanes: Vec<(String,)> =
            sqlx::query_as("SELECT lane FROM fanout_lanes WHERE group_id = ? ORDER BY lane")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(FanOutGroup {
            id: group_id.to_string(),
            pipeline: row.0,
            join_target: row.1,
            downstream: row.2,
            expected_lanes: lanes.into_iter().map(|(l,)| l).collect(),
            completed: row.3 != 0,
        })
    }

    /// Seed the expected lane set with pending placeholders so `load` knows the
    /// full lane set before any verdict is recorded. Called by the pool right
    /// after `create`, once per lane. A pending row has verdict='pending'.
    pub async fn seed_lane(&self, group_id: &str, lane: &str) -> Result<(), FanOutStoreError> {
        sqlx::query(
            "INSERT INTO fanout_lanes (group_id, lane, verdict) VALUES (?,?,'pending')
             ON CONFLICT(group_id, lane) DO NOTHING",
        )
        .bind(group_id)
        .bind(lane)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record this lane's settled verdict, then attempt to complete the group
    /// exactly once. The single-row conditional UPDATE is the guard (vet F1).
    pub async fn record_and_try_complete(
        &self,
        group_id: &str,
        lane: &str,
        verdict: Verdict,
    ) -> Result<BarrierOutcome, FanOutStoreError> {
        // 1. Upsert this lane's verdict (idempotent on re-record — D6).
        sqlx::query(
            "INSERT INTO fanout_lanes (group_id, lane, verdict) VALUES (?,?,?)
             ON CONFLICT(group_id, lane) DO UPDATE SET verdict=excluded.verdict",
        )
        .bind(group_id)
        .bind(lane)
        .bind(verdict_str(verdict))
        .execute(&self.pool)
        .await?;

        // 2. Read all lane verdicts; bail if any lane is still pending.
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT lane, verdict FROM fanout_lanes WHERE group_id = ?")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await?;
        let any_pending = rows.iter().any(|(_, v)| v == "pending");
        if rows.is_empty() || any_pending {
            return Ok(BarrierOutcome::Parked);
        }

        // 3. All lanes settled — attempt the completes-once guard.
        let res = sqlx::query("UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0")
            .bind(group_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            // Another settlement already won, or already completed (idempotent).
            return Ok(BarrierOutcome::Parked);
        }

        // 4. This caller is the sole winner — compute the continuation.
        let g = self.load(group_id).await?;
        let recorded: Vec<LaneVerdict> = rows
            .into_iter()
            .map(|(lane, v)| {
                parse_verdict(&v)
                    .map(|verdict| LaneVerdict { lane, verdict })
                    .ok_or_else(|| FanOutStoreError::BadVerdict(v.clone()))
            })
            .collect::<Result<_, _>>()?;
        Ok(BarrierOutcome::Completed(g.continuation(&recorded)))
    }
}
```

NOTE on the `load` doc-comment confusion: the seeding approach (`seed_lane`) is the resolution — `create` writes the group row, then the pool calls `seed_lane` once per expected lane. `load` reads the lane set from those seeded rows. A `'pending'` row is treated as not-yet-settled in step 2. Clean up the long doc comment on `load` to just: "Load a group + its expected lanes (seeded at create time)."

- [ ] **Step 4: Register the module**

In `src-tauri/runtime/src/lib.rs`, after the `fanout_group` lines, add:

```rust
pub mod fanout_store;
pub use fanout_store::*;
```

- [ ] **Step 5: Adjust the tests to seed lanes**

The tests in Step 1 call `create` then `record_and_try_complete` directly without seeding — with the `seed_lane` design, step 2's "any pending" check needs the lane set seeded first, OR the first record of a non-seeded lane would make `rows` contain only that one lane and wrongly look "all settled". **Resolution:** seed in each test right after `create`. Update each test's setup to seed both lanes:

```rust
        store.create(&group()).await.unwrap();
        store.seed_lane("G-1", "lane-a").await.unwrap();
        store.seed_lane("G-1", "lane-b").await.unwrap();
```

Add those two `seed_lane` lines after every `store.create(&group()).await.unwrap();` in the test module.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime --lib fanout_store::`
Expected: PASS — all five barrier tests green, including `concurrent_double_settle_yields_exactly_one_completion` (exactly one `Completed`).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/runtime/src/fanout_store.rs src-tauri/runtime/src/lib.rs
git commit -m "feat(runtime): FanOutStore atomic barrier — completes-exactly-once guard (vet F1 crux)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Pool — fork expansion (spawn one sibling per lane; original terminates forked)

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`

- [ ] **Step 1: Update the pool test pool to run migration 006 + add a `FanOutStore` to `PoolContext`**

The pool's `fresh_pool()` runs `001` + `003`; lane columns + fanout tables live in `006`. In `src-tauri/runtime/src/pool.rs` `mod tests`, extend `fresh_pool()` to also apply `006_fanout.sql`.

Add a `fanout: Arc<FanOutStore>` field to `PoolContext` (the pool needs the barrier). At the top of `pool.rs`, import:

```rust
use crate::fanout_store::{BarrierOutcome, FanOutStore};
use crate::fanout_group::{Continuation, FanOutGroup};
use pipeline::model::NodeKind;
```

Add the field to `PoolContext` (after `tasks`):

```rust
    pub tasks: Arc<TaskStore>,
    /// The fan-out barrier store (sub-project 2). Linear pipelines never touch it.
    pub fanout: Arc<FanOutStore>,
```

Update the `ctx_with(...)` test helper to construct it:

```rust
            tasks: Arc::new(TaskStore::new(pool.clone())),
            fanout: Arc::new(FanOutStore::new(pool)),
```

(Note: `ctx_with` takes `pool: SqlitePool` by value; change its body to `pool.clone()` for `TaskStore` and pass `pool` to `FanOutStore`, OR clone twice — pick whichever compiles; `SqlitePool` is `Clone`.)

- [ ] **Step 2: Write the failing test**

Add to `mod tests`:

```rust
    use pipeline::model::{Fork, Join};

    fn pipeline_v2_forkjoin() -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 2,
            teams: vec![
                team("entry", Some("fork-1"), None),
                team("lane-a", Some("join-1"), None),
                team("lane-b", Some("join-1"), None),
                team("after", Some("done"), None),
            ],
            gates: vec![],
            escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into() }],
        }
    }

    #[tokio::test]
    async fn approve_into_fork_spawns_one_sibling_per_lane_and_terminates_original() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        // entry approves -> fork-1 -> expand.
        process_one_claim(&ctx, &p.teams[0]).await.unwrap();

        // original is terminated (Done, forked).
        let original = ctx.tasks.get(&t.id).await.unwrap();
        assert_eq!(original.state, TaskState::Done);

        // two sibling tasks exist, queued at lane-a and lane-b, sharing a group.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert_eq!(queued.len(), 2);
        let mut stages: Vec<String> = queued.iter().map(|q| q.current_stage.clone()).collect();
        stages.sort();
        assert_eq!(stages, vec!["lane-a".to_string(), "lane-b".to_string()]);
        let group = queued[0].group_id.clone().unwrap();
        assert!(queued.iter().all(|q| q.group_id.as_deref() == Some(group.as_str())));
        assert!(queued.iter().all(|q| q.join_target.as_deref() == Some("join-1")));
    }
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p runtime --lib pool::approve_into_fork_spawns_one_sibling_per_lane_and_terminates_original`
Expected: FAIL — fork expansion isn't implemented; after `entry` approves, the original is routed to `fork-1` Queued (not Done) and no siblings exist.

- [ ] **Step 4: Implement fork expansion in `settle_and_route`**

In `src-tauri/runtime/src/pool.rs`, modify `settle_and_route` so that after computing `routed`, before persisting, it checks whether `routed.next_stage` is a fork node and expands. Replace the body of `settle_and_route` with:

```rust
async fn settle_and_route(
    ctx: &PoolContext,
    task: &mut Task,
    verdict: agent_bus_core::Verdict,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let routed = route(&ctx.pipeline, &task.current_stage, verdict, task.attempts)
        .map_err(PoolError::Route)?;

    // FORK EXPANSION: an approve whose target is a Fork node fans the task out
    // into one sibling task per lane, then terminates the original (forked).
    if let Some(fork) = ctx.pipeline.forks.iter().find(|f| f.id == routed.next_stage) {
        let join = ctx
            .pipeline
            .joins
            .iter()
            .find(|j| fork.lanes.iter().all(|lane| lane_reaches(&ctx.pipeline, lane, &j.id)))
            .ok_or(PoolError::Route(RouteError::NoRoute))?;
        let group_id = format!("G-{}", uuid::Uuid::new_v4());
        let group = FanOutGroup {
            id: group_id.clone(),
            pipeline: task.pipeline.clone(),
            join_target: join.id.clone(),
            downstream: join.downstream.clone(),
            expected_lanes: fork.lanes.clone(),
            completed: false,
        };
        ctx.fanout.create(&group).await?;
        let now = now_unix();
        for lane in &fork.lanes {
            ctx.fanout.seed_lane(&group_id, lane).await?;
            let sib = Task::forked(task, lane, &group_id, &join.id, now);
            ctx.tasks.insert(&sib).await?;
        }
        // Terminate the original: its work is done; the continuation past the
        // join is a fresh task the barrier creates.
        task.state = TaskState::Done;
        task.current_stage = fork.id.clone();
        task.updated_at = now;
        ctx.tasks.update(task).await?;
        return Ok(Some((fork.id.clone(), TaskState::Done)));
    }

    // JOIN BARRIER: an approve whose target is a Join node hits the barrier.
    if routed.next_state == TaskState::Joining {
        return resolve_barrier(ctx, task, agent_bus_core::Verdict::Approve).await;
    }

    if routed.bump_attempts {
        let _ = task.bump_attempts();
    }
    let now = now_unix();

    // A lane task routing to needs-human reports a reject verdict to its barrier
    // before parking (Decision D5).
    if task.group_id.is_some() && routed.next_state == TaskState::NeedsHuman {
        let _ = resolve_barrier(ctx, task, agent_bus_core::Verdict::Reject).await?;
    }

    task.state = routed.next_state;
    task.current_stage = routed.next_stage.clone();
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    Ok(Some((routed.next_stage, routed.next_state)))
}

/// Walk a lane from its entry team following on_approve to the join id (mirrors
/// the validator's linearity walk; here it pairs a fork with its join).
fn lane_reaches(p: &Pipeline, entry: &str, join_id: &str) -> bool {
    let mut current = entry.to_string();
    for _ in 0..=p.teams.len() {
        if current == join_id {
            return true;
        }
        match p.teams.iter().find(|t| t.id == current) {
            Some(t) => match t.outputs.on_approve.as_deref() {
                Some(next) => current = next.to_string(),
                None => return false,
            },
            None => return false,
        }
    }
    false
}
```

`resolve_barrier` is implemented in Task 13 — for THIS task, add a minimal stub so the crate compiles and the fork-spawn test passes (the join path isn't exercised yet):

```rust
/// Resolve a lane settlement against its fan-out group barrier. Full body lands
/// in Task 13; this records the lane verdict and, if this caller completes the
/// group, creates the single continuation task.
async fn resolve_barrier(
    ctx: &PoolContext,
    task: &mut Task,
    verdict: agent_bus_core::Verdict,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let group_id = task.group_id.clone().ok_or(PoolError::Route(RouteError::NoRoute))?;
    let lane = task.lane.clone().unwrap_or_default();
    let now = now_unix();
    let outcome = ctx.fanout.record_and_try_complete(&group_id, &lane, verdict).await?;
    // Park this lane task as terminal for the lane.
    task.state = TaskState::Done;
    task.current_stage = task.join_target.clone().unwrap_or_else(|| task.current_stage.clone());
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    if let BarrierOutcome::Completed(cont) = outcome {
        let (stage, state) = match cont {
            Continuation::Downstream(ds) => (ds, TaskState::Queued),
            Continuation::NeedsHuman => ("needs-human".to_string(), TaskState::NeedsHuman),
        };
        let mut next = Task::injected(
            task.project_id.clone(),
            task.pipeline.clone(),
            stage.clone(),
            task.topic.clone(),
            task.target_repo.clone(),
            now,
        );
        next.state = state;
        next.parent_artifact = task.parent_artifact.clone();
        ctx.tasks.insert(&next).await?;
        return Ok(Some((stage, state)));
    }
    Ok(Some((task.current_stage.clone(), TaskState::Done)))
}
```

Add `FanOutStoreError` to `PoolError`:

```rust
    #[error(transparent)]
    FanOut(#[from] crate::fanout_store::FanOutStoreError),
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p runtime --lib pool::approve_into_fork_spawns_one_sibling_per_lane_and_terminates_original`
Expected: PASS — original Done, two siblings queued at lane-a/lane-b sharing one group with join_target join-1.

- [ ] **Step 6: Run the whole pool module to confirm no regressions**

Run: `cd src-tauri && cargo test -p runtime --lib pool::`
Expected: PASS — every pool test green (the linear approve/revise/rate-limit/usage tests are unaffected because they have empty `forks`/`joins`).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(runtime): pool fork-expansion — spawn one sibling per lane, terminate original forked

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 13: Pool — join barrier end-to-end (all-approve → downstream; one-reject → needs-human; partial parks; restart idempotent)

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`

The `resolve_barrier` stub from Task 12 is already functional; this task adds the end-to-end tests that drive the full fork→lanes→join flow through `process_one_claim` and lock the four barrier behaviours, then hardens `resolve_barrier` if a test exposes a gap.

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/runtime/src/pool.rs` `mod tests`:

```rust
    /// Drive both lanes to approve through process_one_claim; the last lane's
    /// settle must create exactly one continuation queued at `after`.
    #[tokio::test]
    async fn all_lanes_approve_creates_one_downstream_continuation() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        // entry -> fork -> two lane siblings
        process_one_claim(&ctx, &p.teams[0]).await.unwrap();
        // run lane-a then lane-b
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approves into join (parks)
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b approves into join (completes)

        // exactly one continuation queued at `after`.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let at_after: Vec<_> = queued.iter().filter(|q| q.current_stage == "after").collect();
        assert_eq!(at_after.len(), 1, "exactly one continuation past the join");
        assert_eq!(at_after[0].group_id, None, "continuation is back in linear flow");
    }

    #[tokio::test]
    async fn one_lane_reject_routes_join_to_needs_human() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        // lane-b rejects; lane-a approves.
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        // Use a runner that approves for lane-a and the entry, rejects for lane-b.
        // Simplest: per-stage runner via a small dispatcher.
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approve (parks)
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b reject -> barrier reject

        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1, "the joined task escalates to needs-human");
        assert_eq!(nh[0].current_stage, "needs-human");
    }

    #[tokio::test]
    async fn partial_completion_parks_without_continuing() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // only lane-a settles

        // No continuation yet (lane-b still queued).
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().all(|q| q.current_stage != "after"), "no continuation while a lane is outstanding");
        assert!(queued.iter().any(|q| q.current_stage == "lane-b"), "lane-b still queued");
    }
```

Add a small per-stage test runner above the test runner structs (it approves except for `lane-b`, which it rejects), near `RecordingRunner`:

```rust
    /// A runner that returns a seeded reject for the `lane-b` team and approve
    /// for everything else — used to exercise the one-reject barrier path.
    struct StageRunner { reject: RunnerOutput }
    impl StageRunner { fn new(reject: RunnerOutput) -> Self { Self { reject } } }
    #[async_trait::async_trait]
    impl Runner for StageRunner {
        async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
            if req.team_id == "lane-b" {
                Ok(self.reject.clone())
            } else {
                Ok(RunnerOutput { verdict: Verdict::Approve, artifact_path: Some("a.md".into()), final_text: "VERDICT: approve".into(), usage: RunnerUsage::default() })
            }
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail (or reveal gaps)**

Run: `cd src-tauri && cargo test -p runtime --lib pool::`
Expected: The fork-spawn test still passes; the three new tests should pass IF the Task 12 `resolve_barrier` stub is correct. If `one_lane_reject_routes_join_to_needs_human` fails (e.g. the reject lane goes to `needs-human` via the team route *and* the barrier — double-handling), that is the gap to fix in Step 3.

- [ ] **Step 3: Harden `resolve_barrier` + the reject path**

Confirm the reject path is single: a lane task whose team rejects routes (via `route()`) to `needs-human` with `next_state == NeedsHuman`. In `settle_and_route` the `if task.group_id.is_some() && routed.next_state == TaskState::NeedsHuman` block calls `resolve_barrier(.., Reject)` which itself marks the lane task `Done` at the join and (when it completes the group) creates the needs-human continuation. But then the code falls through and *also* sets `task.state = NeedsHuman` and updates — double-write. **Fix:** when the barrier-reject branch runs, `return` its outcome instead of falling through:

```rust
    if task.group_id.is_some() && routed.next_state == TaskState::NeedsHuman {
        return resolve_barrier(ctx, task, agent_bus_core::Verdict::Reject).await;
    }
```

(Replace the `let _ = resolve_barrier(...)?;` line from Task 12 Step 4 with this early `return`.)

Re-run: `cd src-tauri && cargo test -p runtime --lib pool::`
Expected: PASS — all four barrier end-to-end tests green.

- [ ] **Step 4: Add the restart-idempotency test**

Add to `mod tests`:

```rust
    #[tokio::test]
    async fn restart_mid_group_re_evaluates_idempotently() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a parks
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b completes -> 1 continuation

        // Simulate a crash/restart re-claiming a lane and re-settling it: directly
        // re-record lane-b at the barrier. The completed guard makes it a no-op.
        let group = {
            let done = ctx.tasks.list_by_state(TaskState::Done).await.unwrap();
            done.iter().find_map(|d| d.group_id.clone()).unwrap()
        };
        let again = ctx.fanout.record_and_try_complete(&group, "lane-b", Verdict::Approve).await.unwrap();
        assert_eq!(again, BarrierOutcome::Parked, "re-settle after completion creates nothing");

        // still exactly one continuation at `after`.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert_eq!(queued.iter().filter(|q| q.current_stage == "after").count(), 1);
    }
```

Run: `cd src-tauri && cargo test -p runtime --lib pool::restart_mid_group_re_evaluates_idempotently`
Expected: PASS — the re-settle is `Parked`; exactly one continuation remains.

- [ ] **Step 5: Run the whole runtime crate**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS — every runtime test green (task, task_store, router, pool, fanout_group, fanout_store, contract_tests, worker, brake, revision, api).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(runtime): join barrier end-to-end — all-approve->downstream, reject->needs-human, partial parks, restart idempotent

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 14: Wire `FanOutStore` into the composition root + contract tests + DOMAIN.md / context-map

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (build a `FanOutStore`, add it to `PoolContext` in `spawn_worker_loops`)
- Modify: `src-tauri/runtime/src/contract_tests.rs` (Task key set += lane fields)
- Modify: `DOMAIN.md`
- Modify: `docs/context-map.md` (if present)

- [ ] **Step 1: Extend the Runtime contract test**

In `src-tauri/runtime/src/contract_tests.rs`, the `full_task()` helper builds a `Task`; set the three new fields so they serialise, and add them to the expected key set in `task_key_set_matches_ts`:

In `full_task()`, after `t.state = TaskState::Gated;`:

```rust
    t.group_id = Some("G-1".into());
    t.lane = Some("lane-a".into());
    t.join_target = Some("join-1".into());
```

In `task_key_set_matches_ts`, add the three keys to the `set(&[ … ])`:

```rust
            "updated_at",
            "group_id",
            "lane",
            "join_target",
```

- [ ] **Step 2: Run the contract test to verify it fails then fix**

Run: `cd src-tauri && cargo test -p runtime --lib contract_tests::task_key_set_matches_ts`
Expected: First FAIL (key set mismatch — actual has the three new keys, expected doesn't until you add them). After adding the keys in Step 1, re-run → PASS.

- [ ] **Step 3: Wire `FanOutStore` into `spawn_worker_loops`**

In `src-tauri/app/src/lib.rs`, in `spawn_worker_loops`, construct a `FanOutStore` once and add it to each `PoolContext`. The function already has access to a pool via `tasks` — but `TaskStore` doesn't expose its pool. The `tasks` Arc is built at the call site from `pool.clone()`. **Resolution:** pass the pool into `spawn_worker_loops` (add a `pool: sqlx::SqlitePool` parameter), build `let fanout = Arc::new(runtime::fanout_store::FanOutStore::new(pool.clone()));` once, and set `fanout: fanout.clone()` in each `PoolContext`. Update the single call site (around line 406) to pass `pool.clone()`.

Concretely, add the parameter:

```rust
fn spawn_worker_loops(
    handle: tauri::AppHandle,
    pipeline: Arc<Pipeline>,
    tasks: Arc<TaskStore>,
    brake: Arc<Brake>,
    project_root: String,
    usage_sink: Option<Arc<dyn agent_bus_core::UsageSink>>,
    revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>>,
    pool: sqlx::SqlitePool,
) {
    let runner: Arc<dyn runners::output::Runner> = Arc::new(ClaudeCliRunner::new());
    let fanout = Arc::new(runtime::fanout_store::FanOutStore::new(pool));
    for team in pipeline.teams.clone() {
        let ctx = PoolContext {
            pipeline: pipeline.clone(),
            runner: runner.clone(),
            tasks: tasks.clone(),
            fanout: fanout.clone(),
            brake: brake.clone(),
            // …rest unchanged…
```

Update the call site:

```rust
                    spawn_worker_loops(handle.clone(), pipe.clone(), tasks.clone(), brake.clone(), project_root, Some(usage_sink.clone()), revision_reader, pool.clone());
```

- [ ] **Step 4: Build the whole workspace**

Run: `cd src-tauri && cargo build`
Expected: PASS — the app compiles with the new `PoolContext.fanout` field wired.

- [ ] **Step 5: Update DOMAIN.md ubiquitous language**

In `DOMAIN.md`, under `### Pipeline Authoring`, after the `Route` entry, add:

```markdown
- **Fork** — a node that fans one task out into parallel lanes (routes stay single-target; the multiplicity is the fork's `lanes`)
- **Join** — a barrier node that waits for all lanes, then continues — all must approve, else needs-human
- **Lane** — a linear team chain between a fork and its join; named to avoid colliding with git/worktree *branch*
```

Under `### Runtime`, after the `Brake` entry, add:

```markdown
- **Fan-out group** — the `FanOutGroup` aggregate (root `group_id`) owning the *completes-exactly-once* barrier invariant; the lane sibling Tasks reference it
```

- [ ] **Step 6: Bump the shared-kernel note in context-map**

If `docs/context-map.md` exists, find the `Pipeline ↔ Runtime` shared-kernel entry and update its `schema_version` to `2`. Run `grep -n "schema_version" docs/context-map.md` first; if no such entry exists, skip this step (the bump is recorded in `pipeline::model::SCHEMA_VERSION` and DOMAIN.md).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/app/src/lib.rs src-tauri/runtime/src/contract_tests.rs DOMAIN.md docs/context-map.md
git commit -m "feat(app): wire FanOutStore into worker pool; lock Task lane-field contract; DOMAIN.md fork/join/fan-out language

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 15: Frontend — `Pipeline` TS interface + `PipelineView` render fork/join + contract tests

**Files:**
- Modify: `src/ipc/pipeline.ts`
- Modify: `src/ipc/pipeline.test.ts`
- Modify: `src/components/PipelineView.tsx`
- Modify: `src/components/PipelineView.test.tsx`

- [ ] **Step 1: Write the failing IPC contract test**

In `src/ipc/pipeline.test.ts`, extend the `loadPipeline` graph literal to carry `forks`/`joins` and assert they round-trip. Replace the `graph` object + assertion in the `loadPipeline` test:

```ts
    const graph = {
      id: "ddd-spec-plan-impl",
      name: "DDD",
      description: "",
      schema_version: 2,
      teams: [],
      gates: [],
      escalations: [],
      forks: [{ id: "fork-1", lanes: ["a", "b"] }],
      joins: [{ id: "join-1", waits_for: ["a", "b"], downstream: "after" }],
    };
    invokeMock.mockResolvedValueOnce(graph);
    const result = await loadPipeline("/p", "ddd-spec-plan-impl");
    expect(invokeMock).toHaveBeenCalledWith("pipeline_load", {
      project_root: "/p",
      id: "ddd-spec-plan-impl",
    });
    expect(result.forks[0].lanes).toEqual(["a", "b"]);
    expect(result.joins[0].downstream).toBe("after");
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/ipc/pipeline.test.ts`
Expected: FAIL — TypeScript: `Property 'forks' does not exist on type 'Pipeline'`.

- [ ] **Step 3: Add the TS interfaces + fields**

In `src/ipc/pipeline.ts`, after the `Escalation` interface, add:

```ts
export interface Fork {
  id: string;
  lanes: string[];
}

export interface Join {
  id: string;
  waits_for: string[];
  downstream: string;
}
```

And add to the `Pipeline` interface (after `escalations`):

```ts
  escalations: Escalation[];
  forks: Fork[];
  joins: Join[];
```

- [ ] **Step 4: Run the IPC test to verify it passes**

Run: `npm test -- src/ipc/pipeline.test.ts`
Expected: PASS.

- [ ] **Step 5: Write the failing viewer test**

In `src/components/PipelineView.test.tsx`, add a test that a pipeline with forks/joins renders them. Look at an existing render test for the harness (it renders `<PipelineView pipeline={…} />` and queries text). Add:

```tsx
  it("renders fork lanes and join downstream", () => {
    const pipeline = {
      id: "p",
      name: "Parallel",
      description: "",
      schema_version: 2,
      teams: [],
      gates: [],
      escalations: [],
      forks: [{ id: "fork-1", lanes: ["lane-a", "lane-b"] }],
      joins: [{ id: "join-1", waits_for: ["lane-a", "lane-b"], downstream: "after" }],
    };
    render(<PipelineView pipeline={pipeline} />);
    expect(screen.getByText(/fork-1/)).toBeInTheDocument();
    expect(screen.getByText(/lane-a, lane-b/)).toBeInTheDocument();
    expect(screen.getByText(/join-1/)).toBeInTheDocument();
    expect(screen.getByText(/→ after/)).toBeInTheDocument();
  });
```

(Match the existing test file's imports — `render`, `screen` from `@testing-library/react`, and the `PipelineView` import. If existing render-test literals omit `forks`/`joins`, add `forks: [], joins: []` to them so they keep type-checking.)

- [ ] **Step 6: Run the viewer test to verify it fails**

Run: `npm test -- src/components/PipelineView.test.tsx`
Expected: FAIL — the Forks/Joins sections aren't rendered; `getByText(/fork-1/)` throws.

- [ ] **Step 7: Render fork/join in `PipelineView`**

In `src/components/PipelineView.tsx`, after the Escalations section's closing block (after the escalations `.map`), add:

```tsx
      <div style={sectionTitle}>Forks ({pipeline.forks.length})</div>
      {pipeline.forks.map((f) => (
        <div key={f.id} style={card}>
          <div style={{ color: "var(--text)", fontWeight: 500 }}>{f.id}</div>
          <div style={meta}>lanes: {f.lanes.join(", ")}</div>
        </div>
      ))}

      <div style={sectionTitle}>Joins ({pipeline.joins.length})</div>
      {pipeline.joins.map((j) => (
        <div key={j.id} style={card}>
          <div style={{ color: "var(--text)", fontWeight: 500 }}>{j.id}</div>
          <div style={meta}>
            waits for: {j.waits_for.join(", ")} · downstream → {j.downstream}
          </div>
        </div>
      ))}
```

- [ ] **Step 8: Run the viewer test to verify it passes**

Run: `npm test -- src/components/PipelineView.test.tsx`
Expected: PASS — fork-1, lane-a, lane-b, join-1, → after all rendered.

- [ ] **Step 9: Run the full frontend suite**

Run: `npm test`
Expected: PASS — all frontend tests green (any pre-existing `Pipeline` literals in other tests may need `forks: [], joins: []` added; fix any type errors that surface).

- [ ] **Step 10: Commit**

```bash
git add src/ipc/pipeline.ts src/ipc/pipeline.test.ts src/components/PipelineView.tsx src/components/PipelineView.test.tsx
git commit -m "feat(frontend): Pipeline TS forks/joins + PipelineView fork/join sections + contract tests

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 16: Full green sweep + pipeline contract test (if present)

**Files:**
- Modify: `src-tauri/pipeline/src/contract_tests.rs` (if it locks the `Pipeline`/node key sets)

- [ ] **Step 1: Check the pipeline contract test**

Run: `grep -n "forks\|joins\|node_ids\|key_set\|NodeKind" src-tauri/pipeline/src/contract_tests.rs`
If the file locks the `Pipeline` serialized key set or `NodeKind` strings, extend it: add `"forks"`, `"joins"` to the `Pipeline` expected key set, and add `fork`/`join` to any `NodeKind` string assertion. If it does not exist or does not lock these, skip — no change needed.

- [ ] **Step 2: If extended, run it**

Run: `cd src-tauri && cargo test -p pipeline --lib contract_tests::`
Expected: PASS.

- [ ] **Step 3: Full Rust workspace test sweep**

Run: `cd src-tauri && cargo test`
Expected: PASS — every crate green (pipeline, runtime, app, and all others). No live `claude` is invoked anywhere (FakeRunner + in-memory pools throughout).

- [ ] **Step 4: Full frontend sweep**

Run: `npm test`
Expected: PASS — all frontend tests green.

- [ ] **Step 5: Clippy (matches the repo's discipline)**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: PASS — no warnings. Fix any clippy nits introduced (e.g. needless clones in the new code) without changing behaviour.

- [ ] **Step 6: Commit (only if Step 1 or Step 5 changed files)**

```bash
git add -A
git commit -m "test(parallel-flow): extend pipeline contract test + clippy clean for fork/join

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-review (against the spec)

**Spec coverage:**
- Schema `Fork`/`Join`/`NodeKind`/`Pipeline.forks/joins`/`node_ids()`/`SCHEMA_VERSION=2` — Task 1.
- Validation (resolve-to-team, ≥2 lanes, lane linearity, v1-rejects-fork, downstream, reachability, accept v1+v2) — Tasks 2–4.
- Parse v2 fork/join — Task 5.
- Migration 006 (lane columns + fanout_groups + fanout_lanes, both migration lists, idempotency assertion) — Task 6.
- Task lane fields + `forked()` + `Joining` — Task 7; TaskStore persistence — Task 8.
- Router Fork/Join targets — Task 9.
- `FanOutGroup` aggregate (pure) — Task 10; `FanOutStore` atomic barrier + the concurrent double-settle crux — Task 11.
- Pool fork-expansion — Task 12; join barrier end-to-end (all-approve→downstream, one-reject→needs-human, partial parks, restart idempotent) — Task 13.
- Composition-root wiring + Runtime contract test + DOMAIN.md/context-map — Task 14.
- Frontend `Pipeline` TS + `PipelineView` + contract tests — Task 15.
- Full green sweep + pipeline contract — Task 16.

**Placeholder scan:** No TBD/TODO; every code step shows full code; every test step shows the exact command + expected output. The one long doc-comment in `fanout_store.rs::load` is explicitly flagged to be trimmed (Task 11 Step 3 note).

**Type consistency:** `FanOutGroup`/`LaneVerdict`/`Continuation` (Task 10) are consumed identically in `FanOutStore` (Task 11) and the pool (Tasks 12–13). `BarrierOutcome::{Parked, Completed}` is the single result type across store + pool. `Task::forked` signature `(parent, lane_entry_team, group_id, join_target, now)` matches its call in the pool. `TaskState::Joining` is produced by `route()` (Task 9) and consumed by `settle_and_route` (Task 12). `PoolContext.fanout` is added in Task 12 and wired at the root in Task 14. The `record_and_try_complete(group_id, lane, verdict)` signature is identical in store tests, pool calls, and the restart test.

---

## Roadmap — what this unblocks

- **Sub-project 3 — the wizard** can now let users design *and run* pipelines with parallel branches: the schema (fork/join) and Runtime (fan-out group + barrier) both support them, and the Wiring step renders fork/join via the updated `PipelineView`.
- **v1.1 nested groups / non-linear lanes** — v1 keeps lanes flat-linear and the fan-out group non-nested (Decision D8/D10). A future version can let a lane contain a gate or a nested fork by making `FanOutGroup` recursive and the lane-walk hierarchical; the `completed`-guard barrier pattern generalises unchanged.
- **v1.1 early-cancel on first reject** — v1 is a full barrier (in-flight siblings finish even after one lane fails). A future option could cancel outstanding lane tasks on the first reject; the `FanOutGroup` aggregate is the natural owner of that policy.
```
