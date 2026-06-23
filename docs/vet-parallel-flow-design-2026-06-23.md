---
id: 2026-06-23-parallel-flow-spec-vet
target: docs/superpowers/specs/2026-06-23-parallel-flow-design.md
date: 2026-06-23
mode: vet
lens: strategic · design · workshop
operator: tim.healey@splose.com
---

# Vet — parallel-flow spec (sub-project 2)

Pre-build DDD review of the fork/join design against `DOMAIN.md`, the `Pipeline`
model, and the Runtime aggregate/atomic-claim code. Three findings — one medium
(revises a spec decision, pending operator), two low.

## Acknowledged sound

- **`schema_version` 1→2 kernel bump** — fork/join is a breaking change to the
  Pipeline↔Runtime shared kernel; bumping the version (both contexts review) is the
  designed mechanism, used correctly.
- **Router stays pure** — fork expansion in the pool, join resolution in the store;
  `route()` is never asked to return many.
- **Ownership split** — fork/join node *definitions* in Pipeline Authoring; fan-out
  group + barrier in Runtime.

## Findings

### F1 [medium] invariants-belong-to-aggregates (→ §D leaked-invariant) — the barrier invariant has no owning aggregate

```
cited spec section: "DDD decisions → Barrier invariant, no new aggregate" +
                    "Runtime changes → task_store.rs atomic join-completion"
affected code:      runtime/src/task_store.rs (claim_next_for_stage — the single-row
                    conditional-UPDATE atomic pattern), runtime/src/task.rs (Task fields)
```

**What.** The spec models the fan-out group as fields on Task (`group_id`, `branch`,
`join_target`) plus an "atomic join-completion in `TaskStore`… concurrency-guarded so
only one settlement creates the continuation." Two problems: (a) the *exactly-one-
continuation* invariant spans the **sibling set**, so no single Task aggregate owns it —
a homeless invariant; (b) the existing atomic guard is a single-row conditional UPDATE
(`WHERE state='queued'`), but join-completion is a *count-then-create* across rows, which
races (two last-settling siblings each read "all done" and each create a continuation).

**Why it matters.** A rule with no owning aggregate is violated on whatever write path
forgets it; here the violation is a duplicated continuation — the pipeline forks back into
two flows past the join.

**Suggested amendment.** Introduce a small **`FanOutGroup` aggregate** (root `group_id`):
expected branches, per-branch verdicts, and a `completed` flag. The continuation is a
method on the group, guarded by `UPDATE fanout_groups SET completed=1 WHERE id=? AND
completed=0` (rows-affected = the single-writer guard — the *same* pattern as `claim`).
Gives the invariant a home and fixes the race in one move. It is a tiny aggregate, not a
god-aggregate.

**Status:** resolved — operator chose the `FanOutGroup` aggregate; spec amended (DDD decisions + Runtime changes + testing + DOMAIN.md sections).

### F2 [low] off-language-naming — "Branch" overloads git "branch"

```
cited spec section: "Decisions → Branch contents"; "DOMAIN.md updates → Branch"
```

**What.** The app already has **worktrees** (git branches). Naming a fork→join path a
"branch" overloads the term across two concepts.

**Suggested amendment.** Rename to **"lane"** (or spell out "parallel branch") before it
becomes a type/field name (`Fork.lanes`, `task.lane`).

**Status:** resolved — operator chose "lane"; renamed throughout the spec.

### F3 [low] contradicts-spec (internal) — approving into a join node is underspecified

```
cited spec section: "Runtime changes → router.rs unchanged"
affected code:      runtime/src/router.rs (target_to_outcome / kind_of)
```

**What.** A branch's terminal team has `on_approve → <join id>`; `route()` →
`target_to_outcome` will encounter a `Join` node kind it doesn't handle. The spec says the
join is "handled by the store" but doesn't say how the approve-into-join is recognised.

**Suggested amendment.** State explicitly: `route()` maps a `Join` target to a **barrier
outcome** (a new `Routed` variant or sentinel state) that the pool/store resolve — keeping
`route()` total and the Join node kind consistent.

**Status:** resolved — spec now states `route()` maps a `Join` target to a barrier `Routed` outcome the pool/store resolve.

## Roll-up

F1 is the load-bearing one and it reverses a spec decision, so it pauses for the operator.
F2/F3 are cheap amendments. None require a redesign — all amend the spec in place.
