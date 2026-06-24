---
id: vet-runtime-reactivation-2026-06-24
verb: vet
mode: vet
lens: strategic · critique · brief
target: plans/2026-06-24-plan-runtime-reactivation.md
date: 2026-06-24
contexts_touched: [Runtime, Workspace, Pipeline Authoring, Conversational Control]
---

# Vet — Runtime Re-Activation

Reviewing `plans/2026-06-24-plan-runtime-reactivation.md` for DDD design soundness
before build. General plan quality (decomposition/TDD/sequencing) is out of scope.
Focus areas requested: (1) interior mutability preserves the two-aggregate model,
(2) the worker manager is a composition-root concern with no new cross-context edge,
(3) the generation guard is race-safe + idempotent, (4) naming is on-language.

## Summary

The design is **sound**. Interior mutability is correctly scoped to "which pipeline
is active" and does not merge the Task and WorkerPool aggregates or open a cross-root
transaction. The manager is correctly placed at the composition root (the `app` crate)
and hands Runtime only resolved values — no new cross-context edge. The generation
guard is race-safe under the stated ordering and idempotent under rapid re-activation.
One finding (F1) on naming (`Manager` is an explicit off-language cue) and two
clarifying notes (F2, F3) that strengthen invariants the plan already mostly honors.

---

### F1 [medium] Off-language naming — `WorkerManager` / `LoopDeps`

**What:** The plan introduces `WorkerManager` (Task 3) as the type that owns the
activation lifecycle. `Manager` is the canonical off-language placeholder (§E
Off-language naming / §C the-names-lie): it names a technical role, not a domain
concept. DOMAIN.md → Runtime / Operator language gives us the real verb — **activate**
(the OHS command is `activate_project`) — and the real nouns — `Pipeline`,
`ActivePipeline`. There is no domain concept called a "worker manager."

**Cited plan section:** Task 3 (`WorkerManager` struct + `LoopDeps`), Task 4–5
(state-managed, `activate_project` reaches it).

**Why it matters:** The language lives in the code. A type called `WorkerManager`
puts a non-domain word at the center of the bug fix and obscures that this thing's
whole job is to **activate** a pipeline's runtime (swap the active pipeline + spawn
that pipeline's worker loops). The good news: the *method* is already on-language
(`activate`), and the snapshot type (`ActivePipeline`) is on-language. Only the owner
noun strains.

**Suggested amendment:** Rename `WorkerManager` → **`PipelineActivator`** (it
activates a pipeline: swaps `ActivePipeline` + spawns the pipeline's worker loops;
on-language with the `activate` verb and the `Pipeline`/`ActivePipeline` nouns).
Rename the helper bundle `LoopDeps` → **`WorkerDeps`** (the collaborators a worker
loop needs) — `Deps` is acceptable as an internal composition-root struct, but tie it
to the `Worker`/loop concept rather than the generic `Loop`. The command stays
`activate_project`; state type updates to `Arc<PipelineActivator>`. No structural
change — pure rename.

**Status:** resolved — plan amended (Task 3/4/5 use `PipelineActivator` + `WorkerDeps`).

---

### F2 [low] Two-aggregate model — confirm `ActivePipeline` holds no store handles

**What:** The plan's `ActivePipeline` carries `pipeline: Arc<Pipeline>`, `project_id`,
`project_root`, `project_target_repo` — all resolved values, no stores. This is
correct and preserves the Task vs WorkerPool split (the swap is "which pipeline is
active," per the DOMAIN.md note that Runtime consumes resolved strings and "never
learns about the Project type"). The risk is *drift*: a future edit adding a
`TaskStore`/`FanOutStore`/Workspace handle to `ActivePipeline` would quietly turn the
value-snapshot into a god-aggregate spanning both Runtime aggregates.

**Cited plan section:** Task 2 (`ActivePipeline` struct + doc comment).

**Why it matters:** The invariant "`ActivePipeline` is a resolved value snapshot, not
an aggregate" is the thing keeping the two-aggregate model intact through the swap.
It should be stated as an invariant in the type's doc, not left implicit.

**Suggested amendment:** The plan's Task 2 doc comment already says "NOT an aggregate
root: holds no stores, only resolved values." Keep that line verbatim and treat it as
a load-bearing invariant. No code change beyond what the plan specifies.

**Status:** resolved — invariant already stated in Task 2 doc comment; no change needed.

---

### F3 [low] Generation guard — make the swap-before-bump ordering explicit + check at the top of the loop

**What:** Race-safety hinges on two ordering facts the plan states in `## Decisions`
but should also enforce in code: (a) `activate()` swaps `ActivePipeline` *before*
bumping the generation, so any reader/loop that observes the new generation also
observes the new active state; (b) each loop checks the generation at the **top** of
the iteration (before claiming), so a retired loop never issues a fresh claim against
the old pipeline. Idempotency under rapid re-activation follows: each `activate`
`fetch_add`s a unique generation, loops capture their own `my_gen` by value, and only
the latest generation's loops survive — earlier ones break on next poll (≤ the 500 ms
cadence). Old loops capture their pipeline by clone at spawn, so a retiring loop never
reads the newly-activated pipeline.

**Cited plan section:** Task 3 (`activate` swaps then `fetch_add`; `spawn_team_loop`
loop body checks `generation.load(...) != my_gen` at the top), `## Decisions`
(generation-guard design).

**Why it matters:** This is the correctness core of the bug fix. The plan gets it
right; the note is to keep the swap-before-bump order and the top-of-loop check as
written and not "tidy" them into bump-before-swap or a mid-loop check, either of which
would reintroduce a window where a loop runs the wrong pipeline.

**Suggested amendment:** None — the plan as written is correct. Treat the ordering
(swap → bump; check at loop top) as a load-bearing invariant; the existing per-claim
atomicity of `claim_next_for_stage` covers the benign overlap of an old loop finishing
one in-flight claim. `SeqCst` on both the bump and the load is the right (conservative)
choice for a rare-write path.

**Status:** resolved — no change; ordering invariant confirmed.

---

## Verdict

**Vet PASS** with one applied rename (F1). The two-aggregate model holds (F2), the
activator is a composition-root concern handing Runtime only resolved values (no new
cross-context edge), and the generation guard is race-safe + idempotent (F3). Proceed
to build with `PipelineActivator` / `WorkerDeps` naming.
