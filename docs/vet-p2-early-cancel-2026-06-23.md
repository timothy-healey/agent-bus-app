---
id: vet-p2-early-cancel-2026-06-23
verb: vet
target: plans/2026-06-23-plan-p2-early-cancel.md
date: 2026-06-23
lens: strategic · vet · brief
---

# VET — P2 Early-cancel outstanding lanes

**Verdict: SOUND WITH FIXES**

The strategic skeleton is right: the early-cancel decision is keyed to the
`FanOutGroup` barrier, the exactly-once guard is reused verbatim, no new
`TaskState`, no cross-aggregate write, schema stays additive. Three findings —
one mid (the policy/aggregation rule is duplicated in the store instead of
expressed on the `FanOutGroup` aggregate root), two low (a forbidden Task
transition encoded by raw SQL without a named home; spec language drift) — keep
it from being clean.

---

### F1 [mid] Adds where a refactor fits — needs-human rule duplicated in the store, not on the aggregate root

**What.** The plan places `record_failure_and_early_cancel` on `FanOutStore`
(plan Task 3, §"File Structure", DD2) and inside it **hardcodes** the
continuation:

```rust
// plan lines 288-289
// 3. Sole winner: a failed lane forces the needs-human continuation.
Ok(BarrierOutcome::Completed(Continuation::NeedsHuman))
```

But the rule "any non-approve ⇒ needs-human" already has a home: it is the pure
`FanOutGroup` aggregate root's invariant, expressed in
`fanout_group.rs:51-58` (`FanOutGroup::continuation` — `all_approve && all_settled
⇒ Downstream, else ⇒ NeedsHuman`). The full-barrier path *asks the aggregate*
(`fanout_store.rs:155-164` loads the group and calls `g.continuation(&recorded)`);
the early path *restates the conclusion in the store*. DD2 claims the policy
"lives in the `FanOutGroup` aggregate" — but `FanOutGroup` (the type in
`fanout_group.rs`) is never touched by the plan; only the store is. The decision
that "a failed lane forces needs-human early" is a *domain* rule about the
group, currently smeared into the persistence layer next to the SQL.

**Why it matters.** Two copies of the all-approve-else-needs-human rule. If the
aggregation rule ever changes (e.g. a "majority-approve" join), the full path
changes in `continuation()` and the early path silently keeps returning
`NeedsHuman` from the store. Signal: §E *adds where a refactor fits* / §D *domain
logic in the persistence seam*. It is also a latent §D *leaked invariant*: the
aggregate is named as the owner but does not enforce this branch.

**Recommended fix.** Give the early-cancel decision a named method on the
`FanOutGroup` aggregate root and have the store call it, mirroring how the full
path calls `continuation()`. Concretely, add to `fanout_group.rs`:

```rust
impl FanOutGroup {
    /// The continuation when ONE lane fails under an early-cancel join: the
    /// group resolves to needs-human without the other lanes settling. This is
    /// the early-cancel arm of the all-approve-else-needs-human rule.
    pub fn early_cancel_continuation(&self) -> Continuation {
        Continuation::NeedsHuman
    }
}
```

Then in `record_failure_and_early_cancel`, on winning the guard, return
`Ok(BarrierOutcome::Completed(self.load(group_id).await?.early_cancel_continuation()))`
instead of a bare `Continuation::NeedsHuman`. The rule now lives on the root; the
store stays a guard + dispatcher, exactly as it is for the full path. (Trivial
today since both arms are `NeedsHuman` — that is precisely why it is cheap to put
the seam in the right place before a future join policy makes them diverge.)

**Status:** open

---

### F2 [low] Off-state-machine transition encoded in raw SQL with no named home (Queued → Done is illegal per the Task aggregate)

**What.** `cancel_outstanding_lanes` (plan Task 4) parks lanes with a direct
conditional UPDATE:

```sql
-- plan lines 387-391
UPDATE tasks SET state='done', current_stage=COALESCE(join_target, current_stage), updated_at=?
WHERE group_id=? AND state IN ('queued','revising')
```

This is sound on the two-aggregate model — it writes only the `tasks` table
(the Task aggregate); it never touches `fanout_groups` or `WorkerPool`, so no
cross-aggregate write (DOMAIN.md "Two aggregates in Runtime (Task + WorkerPool)
joined by reference"). The plan correctly cites the precedent: the barrier
itself sets `task.state = TaskState::Done` by hand (`pool.rs:368`).

The wrinkle: `queued → done` is a transition the Task state machine **forbids**.
`Task::can_transition_to` (`task.rs:163-173`) allows from `Queued` only
`Running`/`Braked`; there is no `Queued → Done` edge, and `revising → done` is
likewise absent. So the existing hand-park precedent (`pool.rs:368`) is a
`running → done` park (which *is* legal: `task.rs:165`). The plan's new UPDATE
parks `queued`/`revising` rows — i.e. it introduces a state move the aggregate's
own legality table would reject. The raw UPDATE is the *only* place this move
exists; it is invisible to `can_transition_to`.

**Why it matters.** Low blast radius (the rows are terminal-by-intent and the
group is already `completed`), but the Task aggregate is named as the owner of
the state machine (`task.rs:1-4`) and this is a legal-by-fiat bypass of it. A
reader auditing "what can reach `Done`?" via `can_transition_to` will not see
this path. Signal: §D *leaked invariant* (a state move outside the aggregate's
guard).

**Recommended fix.** Keep the bulk conditional UPDATE (whole-aggregate-set park
in one statement is the right shape — do not loop `transition_to` per row), but
make the new edge *visible in the aggregate*: add `(Queued, Done)` and
`(Revising, Done)` to `can_transition_to` as explicit early-cancel-park edges
(with a comment naming the early-cancel park), OR add a doc-comment on
`cancel_outstanding_lanes` and on the state machine that the early-cancel park is
a sanctioned out-of-band terminal write, cross-referencing `pool.rs:368`. The
first is preferable — it puts the truth of "these states can be parked to Done"
back in the aggregate that claims to own the state machine. Either way, the plan
should add an assertion to the Task-state tests that this edge is intended, so it
is not later "fixed" as a bug.

**Status:** open

---

### F3 [low] Contradicts the spec's stated full-barrier rationale — reconcile the language

**What.** The parallel-flow spec states the join is a **full barrier** and,
verbatim, "*in-flight siblings are not cancelled on an early failure — simpler
and predictable*"
(`docs/superpowers/specs/2026-06-23-parallel-flow-design.md:12`). The plan adds
exactly the cancellation the spec ruled out. This is fine as an *opt-in* evolution
(`cancel_on_reject` defaults false; the spec's default behaviour is preserved
byte-for-byte, plan DD1/Task 5 test 3), but the spec is now stale and the plan
does not say so.

Naming check against DOMAIN.md / spec ubiquitous language (§C):
- `cancel_on_reject`, `cancel_outstanding_lanes` — `Lane` and `Join` are canon
  (DOMAIN.md:72, spec:78). "Cancel outstanding lanes" reads cleanly in the
  ubiquitous language. **OK.**
- `record_failure_and_early_cancel` — "early-cancel" is a *new* term not in
  DOMAIN.md. It is descriptive and not a technical placeholder (not
  Manager/Helper/Processor), so it is acceptable, but it should be promoted to
  the Runtime ubiquitous language so it is canon rather than ad-hoc.
- Minor: the method name says `record_failure_*` while it takes a `Verdict` and
  the trigger is specifically `Verdict::Reject` (plan DD6, mapped from both
  explicit reject and revise-cap escalation per D5). "Failure" is the right
  domain word (it spans both), but ensure the doc-comment keeps tying it to the
  `Reject`-at-barrier signal so "failure" is not read as a third verdict.

**Why it matters.** Low — it is drift between an approved spec and the plan that
supersedes it. Signal: §E *contradicts the spec*. Left unreconciled, a future
reader trusts the spec's "not cancelled" sentence.

**Recommended fix.** (a) Add one line to the spec (or a P2 addendum) recording
that early-cancel is now an opt-in per-join policy, superseding the
"not cancelled" sentence for `cancel_on_reject` joins; the full barrier remains
the default. (b) Add **Early-cancel** to DOMAIN.md's Runtime ubiquitous language
(e.g. "*Early-cancel — an opt-in per-join policy where one failing lane resolves
the fan-out group to needs-human immediately, parking the outstanding lanes,
instead of waiting for the full barrier*"). No code change.

**Status:** open

---

## Confirmed sound (no finding)

- **Exactly-once for BOTH paths.** The early path reuses the identical guard
  `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0` (plan line
  279 == `fanout_store.rs:145`). Both paths arbitrate on the one `completed=0`
  row guard; a concurrent straggler sees `rows_affected==0` and parks (plan DD3,
  test "early_cancel_is_exactly_once_against_a_concurrent_straggler"). The
  single-writer invariant in `fanout_group.rs:1-6` is preserved. The continuation
  is created in the pool only inside `if let BarrierOutcome::Completed(..)`
  (pool.rs:372 / plan line 561) — so exactly one continuation, both paths.
- **Lane key consistency.** `expected_lanes = fork.lanes` and the recorded lane
  is `task.lane` (the entry-team id), both seeded/recorded with the same key
  (`pool.rs:289,294-296,364`); the early path upserts the same `(group_id, lane)`
  row. No seed/record key mismatch.
- **Two-aggregate model intact.** Cancellation writes only the `tasks` table; the
  barrier completion writes only `fanout_groups`. No transaction spans both
  roots; they stay joined by `group_id` reference (DOMAIN.md Runtime).
- **Schema additivity.** `Join.cancel_on_reject` with `#[serde(default)]` false
  is additive-optional; existing v1/v2 pipelines deserialize unchanged. No
  `SCHEMA_VERSION` bump is warranted — the wire format only *grows* an optional
  field, and the shared-kernel bump rule (spec:18, "breaking change →
  schema_version bump") does not trip because nothing existing changes meaning.
  Sound.
- **Don't-kill-running (DD5).** `running` lanes are excluded from the cancel
  UPDATE and their later settle no-ops on the completed guard. Consistent with
  the aggregate model and the spec's worker model.

---

## Resolution (applied to plans/2026-06-23-plan-p2-early-cancel.md)

- **F1 — RESOLVED.** Plan Task 3 now adds `FanOutGroup::early_cancel_continuation(&self) -> Continuation` on the aggregate root (with its own failing-test-first steps 0a–0d); `record_failure_and_early_cancel` calls `g.early_cancel_continuation()` after winning the guard instead of hardcoding `NeedsHuman`. The aggregation rule has one home on the aggregate.
- **F2 — RESOLVED.** Plan Task 4 now adds explicit `(Queued, Done)` and `(Revising, Done)` early-cancel-park edges to `Task::can_transition_to` (steps 0a–0d) plus a `early_cancel_park_edges_are_legal` test asserting the edge is intended. The bulk conditional UPDATE shape is retained.
- **F3 — RESOLVED.** Plan Task 7 (docs) adds an **Early-cancel** term to DOMAIN.md's Runtime ubiquitous language and an addendum to the parallel-flow spec's Decisions bullet noting full-barrier is now the default superseded by the opt-in `cancel_on_reject`.
