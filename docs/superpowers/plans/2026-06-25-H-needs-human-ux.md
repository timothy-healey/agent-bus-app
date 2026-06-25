# H — needs_human UX: L2 recovery actions + L3 failure detail

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Design source: `docs/superpowers/specs/2026-06-25-needs-human-ux-design.md` (read first). Promotes backlog L2 + L3. Builds on R (engine populates `invocation_audit`) + G6 (`model_unavailable` outcome).

**Goal:** A `needs_human` card shows *why* (the audit trail, latest outcome as the headline) and offers state-aware recovery — Retry / Approve&advance / Abandon for a failure escalation, Accept / Send-back for an intentional hand-off.

**Architecture:** A Runtime OHS read over `InvocationAuditStore` (L3) + four operator-action commands on the Task/Store/Run aggregates (L2), reusing existing transitions/guards. Frontend: a CardDrawer history panel + a state-aware action bar + a pure failure-vs-handoff classifier; handlers use the catch-and-surface error pattern.

**Tech Stack:** Rust (sqlx), React/TS, vitest.

---

## Tasks (TDD)

- [ ] **Task 1 — L3 read path.** `InvocationAuditStore::list_for_task(task_id) -> Vec<InvocationRow>` (newest-first) + `InvocationRow { invocation_id, team_id, model, attempts, started_at, settled_at, outcome: String, input_tokens, output_tokens }` where `outcome` encodes `verdict:approve|revise|reject` or `error:<class>`. New Runtime OHS `list_invocations(task_id)` command (register in `generate_handler!`). Serde wire-contract test + a store test over a fixture audit. Commit.
- [ ] **Task 2 — L2 commands (Runtime).** Add operator-action commands (mirror the gate-verdict command pattern; each emits `task-changed` + re-runs `try_finish_run`):
  - `retry_task(task_id)` — requeue the item at the stage that escalated it (the last `invocation_audit` row's `team_id`): `state→queued`, `current_stage←that team`, `attempts←1`, `StoreRepo::ensure` its store; re-open the Run if completed.
  - `force_advance(task_id)` — reserve+commit the item into the failed stage's `on_approve` downstream store (block-before-claim; surface backpressure if full), free its parking.
  - `abandon_task(task_id)` / `accept_task(task_id)` — `state→done` (kept for lineage; distinct labels, same terminal effect).
  Tests (in-memory pool + a seeded escalated work-item + audit row): retry→queued at the failed stage w/ reset attempts + run re-opened; advance→downstream store committed (+ backpressure when full); abandon/accept→done; completion re-evaluated. Commit.
- [ ] **Task 3 — Classification helper (frontend, pure).** `classifyNeedsHuman(invocations, pipeline) -> "failure" | "handoff"`: latest outcome `error:*`/`reject`/exhausted-revise ⇒ failure; clean `approve` into the terminal escalation ⇒ handoff. Unit tests. Commit.
- [ ] **Task 4 — Frontend IPC.** `ipc/runtime.ts`: `InvocationRow` type + `listInvocations`, `retryTask`, `forceAdvance`, `abandonTask`, `acceptTask`. Wire-contract test for `InvocationRow`. Commit.
- [ ] **Task 5 — CardDrawer history panel + state-aware actions.** Add a history panel listing invocations (team · model · attempt · outcome · usage · age), latest outcome as the card's reason line. Replace the blanket `disabled={!gated}` action bar with a state-aware one: `gated` → reject/revise/approve (unchanged); `needs_human`+failure → Retry/Approve&advance/Abandon; `needs_human`+handoff → Accept/Send back; else read-only. Component tests for each state's action set + the panel. Commit.
- [ ] **Task 6 — App wiring + error surfacing.** `App.tsx` handlers calling the new commands, using the catch→dismissible-alert pattern (the project-delete fix) so a failed action never throws unhandled. `listInvocations` loaded for the open card. Tests. Commit.
- [ ] **Task 7 — Impeccable pass (frontend-facing).** Over the history panel + action bar: tokens, `ui/Button` (destructive = `danger` + confirm on Abandon), focus-visible, a11y (panel as a list, reason as `role` appropriate), empty/loading states, copy (no em-dashes/exclamations). Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`; `bun run build`
- Tag: `git tag plan-H`

## Spec coverage
- L3 read + history panel + headline reason → Tasks 1,4,5. ✓
- Classification (failure vs hand-off) → Task 3,5. ✓
- L2 recovery commands + state-aware bar → Tasks 2,5,6. ✓
- Error surfacing (no unhandled rejection) → Task 6. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. Read command seals the audit idiom behind `InvocationRow`. L2 commands reuse existing Task/Store/Run transitions + guards (no new aggregate/invariant). Operator-initiated only (no auto-retry). Live claude stays structural-only.
