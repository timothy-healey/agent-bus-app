# Spec — Parallel flow: fork / join (sub-project 2 of the brainstorming new-project wizard)

*Design doc. Brainstormed + DDD-vetted 2026-06-23. Second of three sub-projects; its own plan → implementation cycle. Independent of sub-project 1 (`llm_chat`).*

## Why this exists

The wizard (sub-project 3) lets users design pipelines with **parallel lanes** (e.g. two reviewers at once that rejoin). Today the model is strictly linear: a Route is a single edge (`on_approve: Option<String>`), the router returns one `next_stage`, and a Task occupies one stage at a time. This sub-project adds fan-out/fan-in to the **schema** and the **Runtime** so such pipelines can actually run. It is independently valuable — any pipeline (including the bundled DDD one) can use it.

## Decisions (from brainstorm 2026-06-23)

- **Shape: explicit `fork` + `join` node kinds** (chosen over list-valued routes). Routes stay single-target; parallelism is modeled as node kinds alongside `gate`/`escalation`. Preserves DOMAIN.md's "a Route is one edge pointing at another node" and the router's "one next_stage per node" everywhere except the one explicit `fork` node.
- **Join semantics: all-must-approve, else needs-human.** A join proceeds to its `downstream` only when **every** branch approves. A branch's revises loop **within that branch** up to the attempts cap (3). If any branch rejects or exhausts its revises, the whole joined task routes to **needs-human**. The join is a **full barrier**: it waits for all branches to settle, then aggregates (in-flight siblings are not cancelled on an early failure — simpler and predictable). *(v1.1 update — P2: this full-barrier behavior is now the DEFAULT; a join may opt into `cancel_on_reject: true`, which cancels outstanding lanes and resolves to needs-human on the first lane failure. The `FanOutGroup` aggregate owns the policy; the completes-once guard is unchanged.)* *(v1.1 update — P3: a join may also opt into `quorum: Some(n)`, proceeding to `downstream` as soon as N of M lanes approve and resolving to needs-human once N is impossible. The `FanOutGroup` aggregate owns the rule (`quorum_continuation`); the completes-once guard is unchanged. When set, quorum governs success and `cancel_on_reject` is ignored.)*
- **Lane contents: linear team chains, no gates / no nested forks.** A **lane** (a fork→join parallel path; named to avoid colliding with git/worktree *branch*) is one or more teams in sequence (each with its normal revise loop). Gates and forks live at the top level only. This keeps the fan-out group **flat** (no nested groups) in v1.

## DDD decisions (from the council vet 2026-06-23)

- **Ownership split.** `fork`/`join` **node definitions** belong to **Pipeline Authoring** (the schema). The **fan-out group, barrier, and verdict aggregation** belong to **Runtime** (the task lifecycle). 
- **Shared-kernel bump.** `Pipeline ↔ Runtime` is a shared kernel keyed on `schema_version`. Adding fork/join is a breaking change → **`schema_version` 1 → 2**, reviewed by both contexts. `validate.rs` accepts both 1 and 2; only v2 pipelines may contain fork/join.
- **Barrier invariant owned by a `FanOutGroup` aggregate** (vet F1). Named invariant: *"exactly one continuation task is emitted when all lanes of a fan-out group have settled, and only once."* The invariant spans the **lane sibling set**, so it needs an owning aggregate. `FanOutGroup` (root `group_id`) holds the expected lanes, each lane's settled verdict, and a `completed` flag; it is the consistency boundary for the barrier. The continuation is created by a method on the group guarded by `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0` — the same single-row conditional-UPDATE atomic guard the existing `claim` uses (rows-affected = the single writer). This gives the invariant a home and removes the count-then-create race. It is a tiny aggregate (one invariant), not a god-aggregate, and does not span Task+WorkerPool.
- **Router stays pure.** `route()` remains single-valued for teams/gates. **Fork expansion** (spawn N siblings) is a **pool** operation; **join resolution** is the **store barrier**. Neither smears multiplicity into the pure router.

## Schema changes (Pipeline Authoring — `pipeline` crate)

`model.rs`:
- `NodeKind` gains `Fork` and `Join`.
- New nodes:
  ```rust
  pub struct Fork { pub id: String, pub lanes: Vec<String> }             // lane = the entry team id of each parallel lane
  pub struct Join { pub id: String, pub waits_for: Vec<String>, pub downstream: String }  // waits_for = the terminal team id of each lane
  ```
- `Pipeline` gains `#[serde(default)] forks: Vec<Fork>` and `joins: Vec<Join>`; `node_ids()` includes them.
- `SCHEMA_VERSION = 2`.

`validate.rs` adds rules (v2 only):
- Every `fork.lanes[i]` and `join.waits_for[i]` resolves to a **team** node.
- A fork has ≥2 lanes; a join `waits_for` matches the set of terminal teams of the fork it pairs with.
- Each lane is a **linear team chain**: lane teams' `on_approve` eventually reach the join; **no gate, escalation, or fork** appears inside a lane.
- `downstream` resolves to any node kind (team/gate/escalation).
- Reachability + no-orphan checks extended to fork/join.
- A v1 pipeline containing fork/join is rejected ("forks require schema_version: 2").

## Runtime changes (`runtime` crate)

`task.rs` — Task gains lane fields (nullable; absent for linear tasks):
- `group_id: Option<String>` — the fan-out group this task belongs to.
- `lane: Option<String>` — which lane (entry team id).
- `join_target: Option<String>` — the join node this lane reports to.

`fanout_group.rs` — **new `FanOutGroup` aggregate** (root `group_id`): the expected lanes, each lane's settled verdict, and a `completed` flag. Owns the *completes-exactly-once* barrier invariant; it is the consistency boundary.

Migration `006_fanout.sql` adds the Task lane columns **and** a `fanout_groups` table (`id` PK, `pipeline`, `join_target`, `downstream`, `completed INTEGER DEFAULT 0`) plus a `fanout_lanes` child table of `(group_id, lane, verdict)` rows. Append-only; prior migrations untouched.

`pool.rs` — fork handling: when a task settles `approve` and its `on_approve` target is a **fork** node, the pool creates a `FanOutGroup` and spawns **one sibling task per lane** (shared `group_id`, each with its `lane` + `join_target`); the original task terminates as forked. Each sibling then runs its lane linearly via the normal claim/settle/route loop.

`fanout_store.rs` — the barrier, guarded by the group row: when a lane's terminal team approves into its `join_target`, the store records that lane's verdict, then attempts to complete the group with `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0`:
- rows-affected = 0 → not all lanes are in yet, or another settlement already won → no continuation here.
- rows-affected = 1 (this caller won, all lanes settled) → create **exactly one** continuation: at `join.downstream` if **all lanes approved**, else at **needs-human**. Inherits the original task's identity/lineage.
- A lane settling while others still run records its verdict and parks (terminal for that lane).

`router.rs` — unchanged for teams/gates. A `Join` target resolves to a **barrier `Routed` outcome** (vet F3) the pool/store handle; `route()` stays total and is never asked to return many.

Crash recovery (F4 policy) extends naturally: orphaned `running` lane tasks are released and re-claimed; the barrier re-evaluates from the persisted `FanOutGroup` (lane verdicts + `completed` flag) on restart, and the `completed` guard keeps recovery idempotent.

## Frontend

- `PipelineView` renders `fork`/`join` nodes (the viewer already groups by node kind; add the two kinds + the branch fan).
- Typed pipeline IPC: the `Pipeline` TS interface gains `forks`/`joins` (contract tests extended per the sub-project-1 discipline).

## Testing (all green, no live `claude`)

- **Validation:** fork/join resolve to teams; ≥2 lanes; lane linearity (reject gate/fork inside a lane); v1-rejects-fork; reachability.
- **Pool fork-spawn:** one approving task → N sibling tasks with a shared `group_id` + correct `lane`/`join_target` + a `FanOutGroup` recording the expected lanes; original terminates forked.
- **Barrier (the crux):** last lane triggers **exactly one** continuation; a simulated concurrent double-settle still yields exactly one (the `WHERE completed=0` guard); all-approve → downstream; one-reject → needs-human; partial completion parks without continuing; restart mid-group re-evaluates from the persisted group idempotently.
- **Contract tests** for the extended `Pipeline` shape (forks/joins).

## DOMAIN.md / config updates (apply when this lands)

- Pipeline Authoring ubiquitous language: **Fork** ("a node that fans one task out into parallel lanes"), **Join** ("a barrier node that waits for all lanes, then continues — all must approve, else needs-human"), **Lane** ("a linear team chain between a fork and its join; named to avoid colliding with git/worktree *branch*").
- Runtime ubiquitous language: **Fan-out group** — the `FanOutGroup` aggregate (root `group_id`) owning the *completes-exactly-once* barrier invariant; the lane sibling Tasks reference it.
- Bump the `Pipeline ↔ Runtime` shared-kernel entry to `schema_version: 2` in `docs/context-map.md`.

## Roadmap — what this unblocks

- **Sub-project 3 — the wizard** can now let users design (and run) pipelines with parallel branches, since both the schema and Runtime support them. The wizard's Wiring step renders fork/join via the updated viewer.
