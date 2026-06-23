# Spec — Parallel flow: fork / join (sub-project 2 of the brainstorming new-project wizard)

*Design doc. Brainstormed + DDD-vetted 2026-06-23. Second of three sub-projects; its own plan → implementation cycle. Independent of sub-project 1 (`llm_chat`).*

## Why this exists

The wizard (sub-project 3) lets users design pipelines with **parallel branches** (e.g. two reviewers at once that rejoin). Today the model is strictly linear: a Route is a single edge (`on_approve: Option<String>`), the router returns one `next_stage`, and a Task occupies one stage at a time. This sub-project adds fan-out/fan-in to the **schema** and the **Runtime** so such pipelines can actually run. It is independently valuable — any pipeline (including the bundled DDD one) can use it.

## Decisions (from brainstorm 2026-06-23)

- **Shape: explicit `fork` + `join` node kinds** (chosen over list-valued routes). Routes stay single-target; parallelism is modeled as node kinds alongside `gate`/`escalation`. Preserves DOMAIN.md's "a Route is one edge pointing at another node" and the router's "one next_stage per node" everywhere except the one explicit `fork` node.
- **Join semantics: all-must-approve, else needs-human.** A join proceeds to its `downstream` only when **every** branch approves. A branch's revises loop **within that branch** up to the attempts cap (3). If any branch rejects or exhausts its revises, the whole joined task routes to **needs-human**. The join is a **full barrier**: it waits for all branches to settle, then aggregates (in-flight siblings are not cancelled on an early failure — simpler and predictable).
- **Branch contents: linear team chains, no gates / no nested forks.** A branch is one or more teams in sequence (each with its normal revise loop). Gates and forks live at the top level only. This keeps the fan-out group **flat** (no nested groups) in v1.

## DDD decisions (from the council vet 2026-06-23)

- **Ownership split.** `fork`/`join` **node definitions** belong to **Pipeline Authoring** (the schema). The **fan-out group, barrier, and verdict aggregation** belong to **Runtime** (the task lifecycle). 
- **Shared-kernel bump.** `Pipeline ↔ Runtime` is a shared kernel keyed on `schema_version`. Adding fork/join is a breaking change → **`schema_version` 1 → 2**, reviewed by both contexts. `validate.rs` accepts both 1 and 2; only v2 pipelines may contain fork/join.
- **Barrier invariant, no new aggregate.** Named invariant: *"exactly one continuation task is emitted when all branches of a fan-out group have settled, and only once."* The siblings are instances of the **same** Task aggregate (not a Task+WorkerPool cross-root transaction), so this is a read-across-siblings + single-writer continuation. Enforced by an **atomic join-completion in `TaskStore`**, reusing the existing atomic-claim pattern (the last sibling to settle atomically wins the right to create the single continuation). No `FanOutGroup` aggregate.
- **Router stays pure.** `route()` remains single-valued for teams/gates. **Fork expansion** (spawn N siblings) is a **pool** operation; **join resolution** is the **store barrier**. Neither smears multiplicity into the pure router.

## Schema changes (Pipeline Authoring — `pipeline` crate)

`model.rs`:
- `NodeKind` gains `Fork` and `Join`.
- New nodes:
  ```rust
  pub struct Fork { pub id: String, pub branches: Vec<String> }          // branch = the entry team id of each parallel branch
  pub struct Join { pub id: String, pub waits_for: Vec<String>, pub downstream: String }  // waits_for = the terminal team id of each branch
  ```
- `Pipeline` gains `#[serde(default)] forks: Vec<Fork>` and `joins: Vec<Join>`; `node_ids()` includes them.
- `SCHEMA_VERSION = 2`.

`validate.rs` adds rules (v2 only):
- Every `fork.branches[i]` and `join.waits_for[i]` resolves to a **team** node.
- A fork has ≥2 branches; a join `waits_for` matches the set of terminal teams of the fork it pairs with.
- Each branch is a **linear team chain**: branch teams' `on_approve` eventually reach the join; **no gate, escalation, or fork** appears inside a branch.
- `downstream` resolves to any node kind (team/gate/escalation).
- Reachability + no-orphan checks extended to fork/join.
- A v1 pipeline containing fork/join is rejected ("forks require schema_version: 2").

## Runtime changes (`runtime` crate)

`task.rs` — Task aggregate gains group fields (nullable; absent for linear tasks):
- `group_id: Option<String>` — the fan-out group this task belongs to.
- `branch: Option<String>` — which branch (entry team id).
- `join_target: Option<String>` — the join node this branch reports to.

Migration `006_fanout.sql` adds these columns (append-only; prior migrations untouched).

`pool.rs` — fork handling: when a task settles `approve` and its `on_approve` target is a **fork** node, instead of one continuation the pool spawns **one sibling task per branch** (shared fresh `group_id`, each with its `branch` + `join_target`), and the original task terminates as forked. Each sibling then runs its branch linearly via the normal claim/settle/route loop.

`task_store.rs` — atomic join-completion: when a sibling settles into its `join_target`, an atomic operation records the branch result and checks the group:
- If **not all** branches have settled → the sibling parks (terminal for that branch); no continuation yet.
- If **all** branches have settled and **all approved** → atomically create **exactly one** continuation task at `join.downstream` (state per `target_to_outcome`), inheriting the original task's identity/lineage. Concurrency-guarded so only one settlement creates it.
- If all settled and **any** branch rejected / hit the cap → create exactly one continuation at **needs-human**.

`router.rs` — unchanged for teams/gates. Fork/join are handled by pool + store as above (route() is never asked to return many).

Crash recovery (F4 policy) extends naturally: orphaned `running` sibling tasks are released and re-claimed; the barrier re-evaluates from persisted branch results on restart.

## Frontend

- `PipelineView` renders `fork`/`join` nodes (the viewer already groups by node kind; add the two kinds + the branch fan).
- Typed pipeline IPC: the `Pipeline` TS interface gains `forks`/`joins` (contract tests extended per the sub-project-1 discipline).

## Testing (all green, no live `claude`)

- **Validation:** fork/join resolve to teams; ≥2 branches; branch linearity (reject gate/fork inside a branch); v1-rejects-fork; reachability.
- **Pool fork-spawn:** one approving task → N sibling tasks with a shared `group_id` + correct `branch`/`join_target`; original terminates forked.
- **Store barrier (the crux):** last sibling triggers **exactly one** continuation; a simulated concurrent double-settle still yields exactly one (atomicity); all-approve → downstream; one-reject → needs-human; partial completion parks without continuing.
- **Contract tests** for the extended `Pipeline` shape.

## DOMAIN.md / config updates (apply when this lands)

- Pipeline Authoring ubiquitous language: **Fork** ("a node that fans one task out into parallel branches"), **Join** ("a barrier node that waits for all branches, then continues — all must approve, else needs-human"), **Branch** ("a linear team chain between a fork and its join").
- Runtime ubiquitous language: **Fan-out group** ("the set of sibling tasks created by a fork; tracked by `group_id`"), and the barrier invariant noted above.
- Bump the `Pipeline ↔ Runtime` shared-kernel entry to `schema_version: 2` in `docs/context-map.md`.

## Roadmap — what this unblocks

- **Sub-project 3 — the wizard** can now let users design (and run) pipelines with parallel branches, since both the schema and Runtime support them. The wizard's Wiring step renders fork/join via the updated viewer.
