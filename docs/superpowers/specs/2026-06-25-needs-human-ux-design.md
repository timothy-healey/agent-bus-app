# Spec — H: needs_human UX (L2 operator actions + L3 failure detail)

*Design doc. Brainstormed 2026-06-25 (visual companion). Promotes backlog **L2** (operator actions on `needs_human`/failed cards) + **L3** (failure/verdict detail). L1 is already shipped (the output contract, runtime ④b/④d); this closes the human-in-the-loop frontier. Its own plan → implementation cycle.*

## Why this exists

A `needs_human` card is a dead-end today: the CardDrawer action bar is `disabled={!gated}` (so a non-`gated` task offers no action), and the card shows only "no review artifact yet" — no *why*. The `invocation_audit` trail (R3, now populated by the engine after R) records each invocation's outcome but **nothing reads it**. So when a run escalates, the operator can neither see what happened nor recover. This adds the read path (L3) and the recovery actions (L2).

## Decisions (from the brainstorm)

1. **Two kinds of `needs_human`, distinguished by the last audit outcome.** A *failure escalation* (gate reject, revises exhausted, operational failure) ⇒ recovery actions; an *intentional hand-off* (the pipeline's terminal "hand off to human" reached via a clean approve) ⇒ "ready for you" / accept. Derived from the latest `invocation_audit` outcome + the stage.
2. **Recovery actions (failure):** **Retry** (requeue at the failed stage), **Approve & advance** (force to the downstream), **Abandon** (drop the item). (Edit-context-and-retry deferred — YAGNI.)
3. **Hand-off actions:** **Accept** (mark the deliverable done) + **Send back** (re-route to an earlier stage).
4. **L3 detail = the audit trail on the card.** A read command surfaces each invocation; the latest outcome is the card's headline reason.

## L3 — Failure / verdict detail (read path)

- **`list_invocations(task_id) -> Vec<InvocationRow>`** — a new Runtime OHS read command over `invocation_audit` (`runtime/src/invocation_audit.rs` gains a `list_for_task`). `InvocationRow { invocation_id, team_id, model, attempts, started_at, settled_at, outcome }` where `outcome` is `verdict:approve|revise|reject` or `error:<class>` (`rate_limited`/`spawn`/`no_result`/`model_unavailable`/`other`) + usage tokens. Sealed read; the audit idiom doesn't leak — only the `InvocationRow` DTO crosses (wire-contract test).
- **Frontend:** `ipc/runtime.ts` `listInvocations` + `InvocationRow` type; a CardDrawer **history panel** listing invocations newest-first (team · model · attempt · outcome · usage · age). The **latest** invocation's outcome is surfaced prominently as the card's reason line (e.g. "declined — reviewer rejected, revises exhausted" / "model unavailable").

## Classification — failure vs hand-off

A `needs_human` card is classified from its **last audit outcome**: `error:*` or `reject` or a source-revise (revises exhausted) ⇒ **failure escalation**; a clean `approve` into the terminal hand-off escalation node ⇒ **hand-off ("ready for you")**. Pure helper over the invocation list + the pipeline's escalation/terminal node, frontend-side (the board already has the pipeline). No new persisted field.

## L2 — Recovery (commands + state-aware action bar)

New Runtime OHS commands (each emits `task-changed`; re-evaluates run completion via the existing `try_finish_run`):

- **`retry_task(task_id)`** — requeue the work-item at the stage that escalated it (the **last audit invocation's `team_id`**), `state → queued`, attempts reset to 1, `current_stage` ← that team; ensure its input store exists. The worker pool re-claims it. A completed Run is re-opened (clear `generator_dry`/`completed` as needed so the loop drives it again — or rely on the loop seeing a queued task).
- **`force_advance(task_id)`** — operator override = "approved at the failed stage": reserve+commit the item into the failed stage's `on_approve` downstream **store** (block-before-claim) and free its current parking; if the downstream is full, surface backpressure (don't lose the item).
- **`abandon_task(task_id)`** — mark the work-item `done` (kept for history/lineage, removed from active lanes).
- **`accept_task(task_id)`** — hand-off: mark the deliverable `done` (a distinct label from abandon for the UI, same terminal effect; kept for lineage).
- **Send back** reuses the existing revise/route path to an earlier stage (or `retry_task` semantics targeting a chosen stage) — v1 may map "Send back" to retry at the upstream writer.

**CardDrawer action bar — state-aware** (remove the blanket `disabled={!gated}`):
- `gated` → reject / revise / approve (today's behavior, unchanged).
- `needs_human` + failure → **Retry · Approve & advance · Abandon**.
- `needs_human` + hand-off → **Accept · Send back**.
- other states → no destructive actions (read-only).

The actions are wired in `App.tsx` (like the existing gate verdict handlers) with error surfacing (the project-delete pattern: catch → dismissible alert, no unhandled rejection).

## Frontend

- `CardDrawer`: the history panel (L3) + the state-aware action bar (L2) + the classification-driven framing (failure vs ready-for-you).
- `ipc/runtime.ts`: `listInvocations`, `retryTask`, `forceAdvance`, `abandonTask`, `acceptTask` + the `InvocationRow` type.
- `App.tsx`: handlers passing through to the commands, with the catch-and-surface error pattern.

## Testing (all via fakes / in-memory pool — no live claude)

- **L3 read:** `list_for_task` over a fixture `invocation_audit` returns rows newest-first with the right outcome encoding; `InvocationRow` wire-contract test.
- **Classification:** pure helper — error/reject/exhausted ⇒ failure; clean approve into terminal ⇒ hand-off.
- **L2 commands:** `retry_task` → item queued at the last-failed stage, attempts reset, run re-opened; `force_advance` → item committed into the downstream store (+ backpressure when full); `abandon_task`/`accept_task` → `done`. Run-completion re-evaluated.
- **Frontend:** the action bar renders the correct set per state (gated/failure/hand-off); the history panel lists invocations; handlers surface errors (no unhandled rejection).

## DDD notes

- **L3 read** is a Runtime OHS over the existing `InvocationAuditStore` (R3) — no new aggregate, the audit idiom sealed behind `InvocationRow`.
- **L2 commands** are operator actions on the **Task (work-item)** aggregate + the **Store**/**Run** aggregates — they reuse the existing transitions (queue/route/done) + the reserve/commit guards; no new invariant. The `needs_human` → `queued`/`done`/downstream edges are explicit operator-driven transitions in the Task state machine.
- **Classification** is a display concern (frontend), not a persisted state — derived from the audit trail.

## Out of scope / non-goals

- Edit-context-and-retry (deferred).
- A full audit *browser* beyond the per-card history panel.
- Auto-recovery / auto-retry policies (operator-initiated only, matching the human-in-the-loop ethos).
- Multi-select bulk recovery (per-card in v1).

## Relationship to other items

- **L1** (output contract) already shipped (④b/④d) — this is L2 + L3.
- Builds on **R** (the engine now populates `invocation_audit`, so L3 has data) and **G6** (the `model_unavailable` error class surfaces here as an outcome).
