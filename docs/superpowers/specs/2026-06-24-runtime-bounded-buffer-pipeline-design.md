# Spec — Runtime: bounded-buffer assembly-line pipeline

*Design doc. Brainstormed 2026-06-24 (visual companion). Headline Runtime redesign. **Supersedes A6** (Start/Stop becomes trivial on top of this) and **absorbs L1** (the agent output contract). Independent of **A4** (authoring-time, unaffected). Large — decomposes into several implementation plans (see "Decomposition" at the end).*

## Why this exists

The brainstorm surfaced a fundamental gap between how the runtime is built and how the operator wants teams to work.

**Built today:** one injected task flows the whole pipeline as a single unit; exactly one worker per team (`workers.max` is parsed but ignored, `scale_team` is a stub); one run produces one artifact and routes the *same* task forward; the only multiplication is *static* fork/join lanes whose count is fixed in the YAML.

**The intended model:** a **bounded-buffer assembly line**. The entry stage (research) *generates* many work-items (candidates) from scanning the target repo; each downstream stage is a **worker pool** that pulls one item at a time from a **capacity-limited store**, transforms it, and pushes to the next store. When a store is full, its upstream **stops** (backpressure) until a downstream worker frees a slot. Items flow independently as a tree (1 start → M candidates → M specs → …). This is Kanban with WIP limits.

The current single-task model cannot express this. This spec redesigns the Runtime around bounded buffers + backpressure + worker pools + a generator stage, while keeping fork/join for *within-item* parallelism.

## Decisions (from the brainstorm)

1. **Bounded-buffer assembly line.** Stages are connected by capacity-limited **stores**; a full store applies backpressure to its upstream.
2. **One item per worker.** A team is a pool of up to `workers.max` workers, each processing a single item at a time.
3. **Generator stage (the entry team).** Has no input store; produces items by scanning, in **capacity-bounded passes, loop-until-dry** (a pass that finds nothing new ends the generator).
4. **Item identity = a stable candidate key.** The system tracks all keys found in a run and hands the generator the "already-found" set each pass; the generator returns only new keys. The key is also the work-item's identity for lineage, stores, and dedup.
5. **Backpressure timing: block-before-claim.** A worker reserves a downstream store slot *before* claiming its input item — no agent run is started that can't be placed.
6. **Coexistence: replace, keep fork/join.** The single-task linear model is retired. Fork/join (P1–P3) is **kept** for *within-item* parallelism (split ONE item into parallel sub-reviews that rejoin — e.g. DDD/security/UX reviewers on one plan).
7. **Capacity per input store**, author-set in the pipeline (with a sensible default). The generator has no input store.
8. **Worker pools honor `workers.max`.** Spawn up to `workers.max` worker loops per team; `scale_team` becomes real.
9. **Output contract (absorbs L1):** a run emits a **list of items**, each `{ key, artifact_path, verdict? }`. The contract text is owned by the **Runners ACL** (the dual to the existing `parse_verdict`/`parse_artifact`).
10. **Gates are bounded stores too.** Each item is gated individually; the gate store has a capacity so humans aren't flooded (backpressure reaches through gates).
11. **Run completion:** a run is done when the generator is dry AND all stores are empty AND no workers are running.
12. **Lineage/board:** item key + parent links form the run tree; the board groups by stage and shows store occupancy (e.g. "2/3").
13. **Two distinct "multiples":** pool workers = *identical* agents draining a queue (throughput); fork lanes = *different* roles on the *same* item (perspectives). Distinct-focus reviewers are lane teams, never a single pool.
14. **Default join policy for a review fan: collect-all, revise-once** (full barrier; bundle every critique back to the writer if any lane is non-approve). Authors may opt into `cancel_on_reject` (P2) or `quorum` (P3) per join.

## Domain model & ubiquitous language (Runtime context)

- **Run** — one execution of a pipeline: the tree of work-items produced from a single Start. Aggregate root; owns the *completes-exactly-once* invariant.
- **Work-item** (the flowing unit; the current `Task` repurposed) — a unit of work identified by its **candidate key**, belonging to a Run, at a stage, holding its artifact + lineage (parent key).
- **Candidate key** — the stable identity of a work-item (e.g. a target file/class path, or a slug the generator assigns). Dedup + lineage key.
- **Store** — a capacity-limited buffer that holds work-items waiting for a stage. Aggregate root; owns the invariant *occupancy never exceeds capacity*. The backpressure consistency boundary.
- **Source / generator stage** — a stage with no input store that produces items by scanning, loop-until-dry. Owns a **found-key ledger** per run.
- **Transformer stage** — every non-source stage; pulls one input item, emits one item (1→1) or drops it (1→0).
- **Worker pool** — the ≤`workers.max` workers of a team; each processes one item at a time.
- **Slot reservation / backpressure** — block-before-claim: reserve a downstream store slot atomically, then claim the input.
- **Fork / Join** (kept) — *within-item* parallelism: one item splits into lanes (different roles) that rejoin at a barrier.

> NOTE: the linear "Task moves stage to stage" language is retired. The flowing unit is a **work-item within a Run**; "inject a topic" becomes **Start a run**.

## Aggregates & invariants (DDD)

- **Store** *(new aggregate)* — root `(run_id, stage)`; holds `capacity` + `occupancy`. Invariant: **occupancy ≤ capacity, always.** Operations are atomic conditional UPDATEs (the same single-writer guard pattern as the existing `claim`/fan-out `completed` guards):
  - `reserve` → `UPDATE stores SET occupancy = occupancy + 1 WHERE id = ? AND occupancy < capacity` (rows-affected = won a slot; 0 = full → backpressure).
  - `release` (run failed/dropped before commit) → `occupancy = occupancy - 1`.
  - `commit` an item into a reserved slot (link the work-item to the store).
  - `take` (a worker pulls the next item) → frees a slot for the upstream (decrement on pull, or on the item leaving the store).
- **Run** *(new aggregate)* — root `run_id`; tracks the generator-dry flag and a `completed` flag. Invariant: **the run completes exactly once**, when generator-dry AND all stores empty AND no running workers (guarded by `UPDATE runs SET completed=1 WHERE id=? AND completed=0`).
- **Work-item** (Task repurposed) — gains `run_id` + `item_key`; keeps stage/state/artifact/parent. Its claim remains the atomic `queued → running` guard.
- **FanOutGroup** (kept, P1–P3) — within-item fork/join barrier, unchanged in its completes-once guarantee; now nested inside a Run.
- **Generator ledger** *(new)* — per `(run_id, source-stage)`, the set of found candidate keys. Append-only; the dedup + dry-detection source of truth.

The router stays pure (single-valued for transformers/gates); fan-out at the source and fork expansion are pool operations; the store reservation is the backpressure barrier — none smear multiplicity into the router (the discipline established by the parallel-flow spec).

## Worker protocol (block-before-claim)

**Transformer worker loop** (per worker, up to `workers.max` per team):
1. Identify the output target store (the downstream stage's input store, or a gate store).
2. **Reserve** a slot there (atomic). Full → sleep/backoff and retry (backpressure; no input claimed).
3. **Claim** one item from this stage's input store (atomic `queued → running`). None available → release the reservation, idle.
4. **Run** the agent (the ACL seam: `invoke`/`invoke_stream`).
5. On success: parse the output item(s), **commit** into the reserved downstream slot, free this stage's input slot. On failure: **release** the reservation; the item follows the existing operational-failure path (audited error class; bounded retry → needs-human).

**Source/generator loop** (the entry team, loop-until-dry):
1. Compute free slots in the downstream store; if zero, backpressure (wait).
2. Reserve up to *K* = free slots.
3. Run the generator agent, passing the **already-found key set**; it returns up to *K* **new** items (keys not in the ledger).
4. Commit each new item; append keys to the ledger.
5. A pass that returns **zero new keys** marks the generator **dry** (retire the source loops; contributes to run-completion).

`workers.max` loops are spawned per team at activation; `scale_team` adjusts the live count up to max.

## Output contract (absorbs L1)

The `VERDICT:`/`ARTIFACT:` convention currently lives *only* in the parser + tests — nothing tells the live agent to emit it (the L1 DOA bug). This spec fixes it as part of the model:

- The **Runners ACL** owns an output-contract preamble — the dual to `parse_verdict`/`parse_artifact`, co-located in the runners crate — composed into the system prompt at the pool (where stage/keys/already-found are known).
- An agent emits a **list of items**; per item: a stable `key`, an `artifact` path (the agent writes the file; the worker verifies it exists), and — for **reviewer** roles — a `verdict` (`approve`/`revise`/`reject`). Producer roles emit items with implicit forward.
- The **source** emits only items whose `key` is not in the already-found set.
- **Artifact write access:** the worker provides the exact artifact path(s) and ensures the agent's scope (`--add-dir` + permission) includes the artifacts location (resolving the L1 "spec-writers has `writes: []` / wrong cwd" gap). Artifacts live under `${project}/artifacts/<stage>/<key>...`.

## Gates as bounded stores

A gate is a store whose **consumer is the human**. Items awaiting a verdict occupy gate-store slots; its capacity bounds the human backlog (the source won't outrun review). On the operator verdict: **approve** → reserve + commit into the gate's downstream store; **revise** → back to the producing stage (with the feedback bundle); **reject** → escalate. Per-item, as today's gate commands already are.

## Fork / join integration (within-item parallelism)

Fork/join (P1–P3) is retained and now lives inside a Run. An item entering a fork splits into lane sub-items (different-role teams), each with its own store/backpressure; the join is the barrier that aggregates. **Default for a review fan: collect-all, revise-once** — full barrier, wait for every lane, and if any lane is non-approve, bundle all lane critiques and route the item back to the writer once (reusing the revision-bundle reader). Authors may set `cancel_on_reject` (P2) or `quorum` (P3) per join. Pool throughput (`workers.max`, identical agents) and fork lanes (distinct roles) are orthogonal and composable.

## Run lifecycle

- **Start a run** (replaces "inject a topic"): create a Run, seed the source stage, activate worker pools. No operator-supplied topic is required — the team prompts are the work (the A6 insight). An optional free-text "run context" may still be supplied for genuinely reusable pipelines, but it is not the default.
- **Stop**: the brake (exists) pauses all workers; resume = un-brake.
- **Complete**: detected by the Run aggregate (generator dry + stores empty + no running workers); emits a `run-changed`/completion event.

## Schema changes (Pipeline Authoring — `pipeline` crate)

- `Team` gains `store: { capacity: u32 }` (its input store; default e.g. `workers.max` or a small constant when omitted) and uses the existing `workers: { default, max }`.
- A stage is the **source** when it has no upstream (the entry team); validated as exactly one source in v1 (mid-pipeline generators are a future extension).
- `SCHEMA_VERSION` **2 → 3** — the linear→assembly-line interpretation is a breaking semantic change even though the new fields are additive; `validate.rs` accepts v3 and rejects the now-removed linear-only assumptions. (v2 fork/join pipelines are migrated/loaded under the new interpretation; the council reviews the kernel bump, as with the v1→v2 fork/join change.)
- Validation: capacities ≥ 1; one source; reachability over stores; fork/join rules unchanged.

## Persistence / migrations (`runtime` crate)

Append-only migrations (current max is 010; this lands as a sequence):
- `stores` table — `(run_id, stage)` PK, `capacity`, `occupancy` (the backpressure aggregate).
- `runs` table — `run_id` PK, `pipeline`, `project_id`, generator-dry flag, `completed`.
- `generator_ledger` table — `(run_id, stage, candidate_key)` rows (dedup/dry).
- `tasks` (work-items) gains `run_id` + `item_key` columns (lineage already present).
- The linear single-task routing is replaced by the store-driven flow; fork/join tables (006/008) are retained.

## Frontend / board

- **Board** groups by stage; each lane shows **store occupancy** (e.g. "2/3") and **pool activity** (n/`max` workers busy). Cards are work-items labelled by `item_key`.
- A **Run** is the unit the board scopes to; a run selector + a "Start run" / brake control replace the inject-centric flow (the A6 surface, now trivial).
- `run-changed`/`task-changed` events drive refresh (hyphenated names, per the event-name fix).

## Testing (all green, no live `claude`)

- **Store aggregate:** reserve respects capacity (concurrent over-reserve yields exactly `capacity` winners via the conditional UPDATE); release/commit/take occupancy accounting; backpressure (reserve fails when full).
- **Generator:** loop-until-dry over a `FakeRunner` scripted to return new-then-empty key batches; dedup against the ledger; reserve-K bounded by downstream capacity; dry detection retires the source.
- **Worker pool:** `workers.max` concurrent claimers each take distinct items; block-before-claim never starts a run without a reserved slot (assert via a full downstream store → no claim).
- **Run completion:** completes exactly once when dry + empty + idle; a concurrent double-check yields one completion (the `completed` guard).
- **Output contract:** parser reads a list of items `{key, artifact, verdict?}`; the contract preamble is composed into the system prompt (asserted, mirroring DS-Schema's embed test); missing/short output → bounded repair or operational-failure path.
- **Gates-as-stores:** per-item gate occupancy + backpressure; approve/revise/reject routing.
- **Fork/join:** retained P1–P3 suites; the review-fan default (collect-all, revise-once with bundled feedback) asserted.
- **Caveat:** the live `claude` generator/transformer runs are structural-only (FakeRunner/fixtures), as with every live-path item.

## DDD notes

- **New aggregates** (`Store`, `Run`, generator ledger) each own exactly one invariant — tiny aggregates, not god-objects; they do not merge Task and WorkerPool.
- **Backpressure invariant** (occupancy ≤ capacity) gets a home (`Store`) and an atomic guard, mirroring the fan-out `completed` guard — no count-then-act race.
- **Router stays pure**; multiplicity lives in pool/store operations.
- **Output contract** is a Runners-ACL concern (the dual to the parser), sealed from Runtime.
- **Ubiquitous language** updates: register Run / Work-item / Store / Source vs Transformer stage / Backpressure / Candidate key in DOMAIN.md; retire the linear-Task language; bump the `Pipeline ↔ Runtime` shared kernel to `schema_version: 3` in `docs/context-map.md`.

## What this removes / supersedes

- The single-task linear flow (inject one topic → one unit stage-to-stage) is **removed**.
- **A6** is superseded (Start/Stop is trivial here; the topic-prefill question is moot — prompts are the work).
- **L1** is absorbed (the output contract is specified here).
- Fork/join (P1–P3) and the invocation audit (R3), live-log (R4), usage (R5), keychain (S1) seams are **retained** and compose in.

## Decomposition (for writing-plans)

This is too large for one plan. Suggested plan sequence, each independently testable:
1. **Schema + validation** — `store.capacity`, source designation, `SCHEMA_VERSION` 3, validate.rs.
2. **Store aggregate + migrations** — `stores`/`runs`/`generator_ledger` tables; reserve/release/commit/take with the atomic guard; Run aggregate + completion.
3. **Worker pools + block-before-claim** — spawn `workers.max` loops; reserve→claim→run→commit/release; retire one-loop-per-team; `scale_team` real.
4. **Generator loop** — loop-until-dry, found-ledger, reserve-K, dry detection.
5. **Output contract (L1)** — Runners-ACL preamble + list-of-items parser + artifact write-path/scope fix.
6. **Gates as bounded stores** — per-item gate occupancy + backpressure + verdict routing.
7. **Fork/join integration** — review-fan default (collect-all, revise-once) over the new model.
8. **Run lifecycle + frontend** — Start/Stop (A6), run selector, board store-occupancy/pool indicators, run-completion events.

## Relationship to other items

- **A4** (skill autocomplete) is independent and can proceed autonomously in parallel.
- **A6** folds into plan 8 here (Start/Stop).
- **L1** folds into plan 5 here.
- **R6** (UI project/run switcher) pairs naturally with plan 8.
