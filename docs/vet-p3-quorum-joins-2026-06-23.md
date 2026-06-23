---
id: vet-p3-quorum-joins-2026-06-23
verb: vet
mode: vet
lens: strategic
register: brief
target: plans/2026-06-23-plan-p3-quorum-joins.md
date: 2026-06-23
verdict: SOUND WITH FIXES
---

# Vet — P3 Quorum joins (N-of-M)

Reviewed: the plan (`plans/2026-06-23-plan-p3-quorum-joins.md`) against `DOMAIN.md`
and the affected code (`pipeline/src/{model,validate}.rs`,
`runtime/src/{fanout_group,fanout_store,pool}.rs`, `src/ipc/pipeline.ts`).

DDD soundness only. Decomposition / testability are upstream-owned and not re-reviewed.

## Summary

The design is DDD-sound. The quorum decision lives where it must — on the
`FanOutGroup` aggregate (`quorum_continuation`), the consistency boundary that
already owns the *completes-exactly-once* barrier invariant and P2's
`early_cancel_continuation`. The exactly-once invariant is preserved: the new
`record_and_try_quorum` asks the aggregate first, then arbitrates with the SAME
`UPDATE … SET completed=1 WHERE completed=0` single-writer guard. The schema
change is additive (`Option<u32>`, `#[serde(default, skip_serializing_if)]`, no
`SCHEMA_VERSION` bump) exactly as P2's `cancel_on_reject`. Validation bounds
(`1..=waits_for.len()`) are correct. The quorum/cancel_on_reject precedence is
coherent (quorum governs success; cancel_on_reject ignored when quorum set).

Three findings, all minor/doc-level. None block; all are cheap to apply in-plan.

## Findings

### F1 [low] §E ubiquitous-language-drift — "Join" definition still says "all must approve"

**What:** `DOMAIN.md` line 71 defines **Join** as "a barrier node that waits for
all lanes, then continues — all must approve, else needs-human." P2 added an
**Early-cancel** policy and P3 adds **Quorum**; the base **Join** gloss now names
only the *default* policy and reads as if it were the only one. (P2 left this
unamended too.)

**Cited:** Plan Task 7 (DOMAIN.md edit) adds a **Quorum** bullet but leaves the
**Join** line as-is; `DOMAIN.md:71`.

**Why it matters:** The ubiquitous language is the model. A reader taking the
**Join** definition at face value would not know two opt-in resolution policies
now exist. Low blast radius (doc only), but the term is the contract.

**Amendment:** In Task 7 Step 1, also soften the **Join** line to name the
default and point at the policies, e.g. *"a barrier node that waits for its lanes,
then continues — by default all-must-approve-else-needs-human; opt-in policies
**Quorum** (N-of-M) and **Early-cancel** vary the resolution."*

**Status:** resolved — amendment folded into Task 7 Step 1 (Join line softened + Quorum bullet).

### F2 [low] §E primitive-leak — quorum semantics ride a bare `Option<u32>` with no named guard

**What:** `Join.quorum: Option<u32>` carries an invariant (`1..=lanes`) enforced
only in `validate.rs`. The aggregate method `quorum_continuation(recorded, quorum: u32)`
trusts the caller to pass a validated value; an out-of-range quorum reaching the
aggregate would silently mis-resolve (e.g. `quorum = 0` → instant downstream on
zero approvals; `quorum > lanes` → never reaches the success branch, only ever
needs-human).

**Cited:** Plan Task 3 (`quorum_continuation` signature) + Task 2 (validation is
the sole guard).

**Why it matters:** The invariant lives only at the validation seam, not with the
value. In practice the store always loads a validated pipeline (validate runs at
load/save — `validate.rs:86` comment), so the aggregate never sees an invalid
quorum; the risk is theoretical. Introducing a `Quorum` value object for one
call site would be over-abstraction (YAGNI) and the room rejects it.

**Amendment:** Keep `Option<u32>`. Add a one-line doc-comment on
`quorum_continuation` stating its precondition (caller passes a validated
`1..=expected_lanes.len()`), so the trust boundary is explicit. The
`saturating_sub` in the plan's impl already prevents underflow panics — keep it.

**Status:** resolved — precondition doc-comment added to `quorum_continuation` in Task 3 Step 3; no value object introduced (YAGNI).

### F3 [info] §E boundary-ownership — confirm precedence belongs to the pool, decision to the aggregate

**What:** The quorum/cancel_on_reject precedence (DD7: "quorum set ⇒ ignore
cancel_on_reject") is decided in `pool.rs::resolve_barrier` (Task 5: `early_cancel =
quorum.is_none() && …`), while the resolution *rule* is on the aggregate. This is
the right split — the pool is the application service that picks which aggregate
method to drive; the aggregate owns each rule — but it is worth stating so a
future reader doesn't push the precedence into the aggregate (which would make the
aggregate aware of a sibling policy it shouldn't know about).

**Cited:** Plan Task 5 Step 3 (pool branch selection) + DD7.

**Why it matters:** Keeps the aggregate single-rule and the policy-selection in the
one place that reads the `Join` node. No change needed; just don't drift it.

**Amendment:** None (informational). The plan as written places it correctly.

**Status:** resolved — placement confirmed correct; no change required.

## Verdict

**SOUND WITH FIXES.** F1 + F2 are one-line doc amendments folded into Tasks 3 and
7; F3 is confirmation, no change. The aggregate ownership, exactly-once guard,
additive schema, validation bounds, and policy precedence are all DDD-correct.
Proceed to implementation.
