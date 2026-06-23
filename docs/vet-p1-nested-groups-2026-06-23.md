---
id: vet-p1-nested-groups-2026-06-23
verb: vet
target: plans/2026-06-23-plan-p1-nested-groups.md
lens: strategic · vet · brief
date: 2026-06-23
contexts: [Runtime, Pipeline Authoring]
verdict: SOUND (4 findings — 3 doc/naming clarity, 1 design-guard; all applied to the plan)
---

# Vet — P1 Nested groups / non-linear lanes

**Scope reviewed:** the proposed change in `plans/2026-06-23-plan-p1-nested-groups.md` against `DOMAIN.md` (Runtime + Pipeline Authoring contexts, the *Fan-out group* / *Lane* / *Join* language) and the affected code: `runtime/src/{fanout_group.rs, fanout_store.rs, pool.rs}`, `pipeline/src/{validate.rs, draft.rs}`, `app/migrations/006_fanout.sql`, `app/src/lib.rs`.

## Room summary

The design is **sound and stays inside the two contexts that own the concepts**. Making `FanOutGroup` recursive via a parent reference (`parent_group_id` + `parent_lane`) is the textbook generalization the parallel-flow spec foresaw (`plan-parallel-flow.md` §"v1.1 nested groups"): the same `completed` conditional-UPDATE barrier applies independently at each level, and a child's resolution feeds its parent lane through the *same* barrier method one level up. No kernel change, no new shared type, no cross-context reach.

The frictions worth recording (architect vs engineer):

- **Architect:** does a parent link turn the fan-out group into a god-aggregate spanning a whole tree? **Engineer:** no — the link is a *reference by id*, not embedding. Child completion (`record_and_try_complete` on the child row) and parent settlement (`record_and_try_complete` on the parent row) are **two separate single-row-guarded writes**, not one transaction across two roots. That is exactly the DDD shape for cross-aggregate consistency (reference, settle eventually, each root guards its own invariant). **F1** makes this explicit so a future reader doesn't "optimize" it into a shared transaction.
- **Engineer:** termination. The recursion (`finish_group → settle_parent_lane → finish_group`) climbs one parent per hop; the lane-walk functions are node-count-bounded. Fine — but the validation depth bound (3) and the runtime have no shared constant, and the runtime does not itself re-check depth. **F2.**
- **Domain expert (Operator/Architect):** the language. `DOMAIN.md` names *Fan-out group*, *Lane*, *Join* but not *child/parent group* or *nested lane*. The new code names are faithful, but the ubiquitous-language section should gain the terms so the recursion is canon. **F3.**
- **Engineer:** the new pool helpers (`expand_fork`, `finish_group`, `spawn_continuation`, `settle_parent_lane`) — are these *adds where a refactor fits* (§E)? **Verdict: no, they are refactor-first** — each *extracts* an existing inline block in `resolve_barrier`/`settle_and_route`. **F4** records the one risk: the extraction must be behaviour-preserving for the root path.

No finding rises to a redesign; all four amend the plan or docs.

---

### F1 [medium] god-aggregate / consistency-boundary — make "no cross-root transaction" an explicit, tested invariant

**What:** The plan's recursive completion (Task 6: `finish_group` → `settle_parent_lane` → `record_and_try_complete` on the parent group) is correct, but its *safety* rests on each group row being its own single-writer guard with no shared transaction. The plan asserts this in the Self-Review but the code carries no marker, and a well-meaning future change could wrap child-complete + parent-settle in one `BEGIN…COMMIT` "for atomicity", which would re-introduce the cross-root coupling the two-aggregate model forbids and could deadlock on the two `fanout_groups` rows.

**Cited plan section:** Task 6 Step 3 (`finish_group`/`settle_parent_lane`); Self-Review "no cross-root transaction".

**Cited code:** `fanout_store.rs:145` (the `UPDATE … WHERE id=? AND completed=0` guard) — this is the single-writer per row; nothing serialises two rows together.

**Why it matters (§E unowned-shared-type sibling / consistency-boundary):** the exactly-once invariant is *per group row*. The whole correctness argument is "each level's barrier is independent". If that becomes implicit, it rots.

**Suggested amendment (applied):** Task 6 Step 3 doc-comment on `settle_parent_lane` already states "the same completes-once guard one level up" and "recurse via finish_group" — strengthen it to state explicitly: *"each call is its own single-row guard; child-complete and parent-settle are deliberately NOT one transaction (no cross-root coupling) — do not merge them."* Add the existing concurrency test `nested_completion_is_exactly_once_against_a_concurrent_parent_straggler` (already in Task 6 Step 1) as the executable proof of this invariant at the nested level.

**Status:** resolved (plan Task 6 carries the concurrency test + the comment is strengthened in the implementation note).

---

### F2 [low] bounded-recursion guard — share the depth bound between validation and runtime; document why the runtime needn't re-check

**What:** Validation enforces `MAX_NESTING_DEPTH = 3` (Task 7); the runtime's recursive `finish_group`/`lane_reaches` rely on the graph already being validated (depth-bounded, fork/join-paired) to terminate. The plan does not state that the runtime trusts validation for the bound, nor does it co-locate the constant.

**Cited plan section:** Task 5 (`lane_reaches` bound = node count), Task 7 (`MAX_NESTING_DEPTH` in `validate.rs`), DD-P1-5.

**Cited code:** `pool.rs:336` (`lane_reaches` bounded by `p.teams.len()`), `validate.rs:54` (the `for _ in 0..=p.teams.len()` bound).

**Why it matters:** a runtime that recurses on an *unvalidated* pipeline (no path does today — `store::load` validates first) could in principle climb an unbounded/cyclic parent chain. The node-count bound in the walk functions already prevents infinite loops, so this is defensive-doc, not a hole.

**Suggested amendment (applied):** add a one-line note to DD-P1-7/Task 5 that the runtime walk is **node-count-bounded for termination** and **trusts `validate.rs` for the depth bound** (Runtime only ever runs validated pipelines — `store::load` resolves+validates). Keep `MAX_NESTING_DEPTH` in `validate.rs` (Pipeline Authoring owns the authoring rule); the runtime does not duplicate it. No shared constant needed — the contexts own different concerns (authoring rule vs termination).

**Status:** resolved (documented as the rationale; no code coupling added — correct boundary).

---

### F3 [low] off-language naming — register "nested / child fan-out group" and "parent lane" in the ubiquitous language

**What:** `DOMAIN.md` Runtime language defines *Fan-out group*, *Lane*, *Early-cancel*, *Quorum* but has no entry for the P1 concepts the code introduces (`parent_group_id`, `parent_lane`, child group, nested lane). The names in the plan are faithful, but the canon should carry them so the recursion is part of the documented model, like P2/P3 were.

**Cited plan section:** Tasks 2–7 (the new field/method names); DD-P1-1/DD-P1-2.

**Cited code:** `DOMAIN.md` → Runtime ubiquitous language (Fan-out group / Early-cancel / Quorum entries).

**Why it matters (§E off-language / §C):** P2 and P3 each added a language entry; P1 generalizes the aggregate itself and deserves one. Without it the next reader sees `parent_group_id` in code with no canon backing.

**Suggested amendment (applied):** add a **Nested fan-out group** entry to `DOMAIN.md` Runtime language after the Quorum entry: *"a fork nested inside a lane spawns a child `FanOutGroup` carrying `parent_group_id` + `parent_lane`; its completion settles the parent lane's verdict (downstream⇒approve, needs-human⇒reject) through the parent group's barrier — the same completes-exactly-once guard, one level up. Bounded to a max nesting depth of 3 (Pipeline Authoring rule)."* Also relax the *Lane* entry's "linear team chain" wording to "a chain between a fork and its join, which under P1 may itself contain a gate or a nested fork". (Done in this vet's follow-up edit to DOMAIN.md.)

**Status:** resolved (DOMAIN.md updated alongside the plan).

---

### F4 [low] adds-where-a-refactor-fits — confirm the pool helpers are extractions, not parallel new code

**What:** Task 4/6 add `expand_fork`, `finish_group`, `spawn_continuation`, `settle_parent_lane`. The *Refactor before you add* law requires these be reshapings of the existing inline blocks (the fork-expansion block in `settle_and_route`, the continuation block at the end of `resolve_barrier`), not a second parallel implementation left beside the old one.

**Cited plan section:** Task 4 Step 3, Task 6 Step 3 ("delete the now-inlined continuation block").

**Cited code:** `pool.rs:276–306` (inline fork expansion), `pool.rs:401–425` (inline continuation block).

**Why it matters:** if the old inline blocks are left in place, the root path and nested path diverge — two homes for "create the continuation", a classic drift bug.

**Suggested amendment (applied):** the plan already instructs "delete the now-inlined continuation block" (Task 6 Step 3) and routes the root path through `spawn_continuation`. Make the same explicit for the fork block: Task 4 replaces the inline `if let Some(fork)` body with the `expand_fork` call (it does). Reviewer checklist at Task 9: grep `pool.rs` for a second `Task::injected(...).state = state` continuation site — there must be exactly one (`spawn_continuation`).

**Status:** resolved (plan deletes the inline blocks; Task 9 gains the single-continuation-site check).

---

## Verdict

**SOUND.** The change is correctly scoped to Runtime + Pipeline Authoring, introduces no kernel/cross-context coupling, and preserves the completes-exactly-once invariant per group row at every level via reference-by-id (no cross-root transaction). All four findings are doc/naming/guard clarifications, applied to the plan and `DOMAIN.md`; none require redesign. Cleared to implement.
