# B5 — Generation + discovery + template (G3, G5, G4, G15)

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Design source: `docs/v1.1-backlog.md` items **G3/G5/G4/G15**. Final build chunk. Backend-heavy (`design_session`, `skills`, `seed_template`) + small frontend wiring. Layers on B1–B4.

**Goal:** Make Generate produce a well-formed, prefilled, well-connected pipeline (reviewer roles + routes + prompt bodies); make skills discoverable during creation; add a per-node "regenerate prompt"; and ship a bundled DDD template.

**Architecture:** Extend the kickoff one-shot (`design_session.rs`) to emit a richer draft; make `list_skills`/`useSkillCatalog` resolve global `~/.claude` without a project + wire the wizard canvas catalog; reuse the `Step::Prompts`/`PromptSlice` logic for per-node regenerate; add a seed template.

---

## Reconciliation (B1–B4)
B4 added a canvas warning when a reviewer-role team lacks revise/decline routes (the LF1/LF2 signal). G3 must make Generate satisfy that **by construction** (reviewers come with `role:"reviewer"` + revise + decline→needs-human). `outcomeLabel` (draftFlow) is the canonical outcome vocab. Skills wiring must reach the wizard's `PipelineCanvas` (now mounted in `AuthoringLayout`).

## Tasks

- [ ] **Task 1 — Skills during creation (G4).** Make `list_skills` accept an **optional** project (null → global `~/.claude` roots only; project sources merged when present). `useSkillCatalog(projectId | null)` loads the **global** catalog even when `projectId` is null (don't short-circuit to empty). Pass a catalog (+ refresh) to the **wizard's** `PipelineCanvas` (currently only `PipelineEditor` passes it). Tests: global catalog loads with no project; wizard canvas receives it. Commit.
- [ ] **Task 2 — Generate emits prompt bodies (G5).** Extend the kickoff one-shot (`kickoff_system_prompt` + the kickoff slice) so each generated team carries a `prompt_body` (teams + prompts in one Generate), so the canvas/NodeDrawer is prefilled. Keep the bounded-repair discipline. Tests: kickoff produces teams with non-empty prompts. Commit.
- [ ] **Task 3 — Generate emits well-formed review structure (G3).** Strengthen the kickoff so recommended graphs include **reviewer teams with `role:"reviewer"` set explicitly** and their **approve + revise (→ writer) + decline (→ needs-human)** routes wired (so B4's under-connected-reviewer badge is satisfied by construction), plus a human gate where appropriate. Set `Team.role` explicitly (don't rely on the name regex). Tests: a generated draft has reviewer roles + all three routes + no under-connected-reviewer warnings. Commit.
- [ ] **Task 4 — Per-node "regenerate prompt" (G5).** A `NodeDrawer` action (via the `NodeContextMenu.actions` extension point and/or a drawer button) that regenerates one team's `prompt_body` by running the existing `Step::Prompts`/`PromptSlice` design-session logic for that team (behind the `llm_chat` seam). Busy/result states. Tests with a fake chat runner. Commit.
- [ ] **Task 5 — Bundled DDD template (G15).** Add an A2 seed template (`pipeline/src/seed_template.rs`) for: research → spec → spec-review → plan → plan-review → implement → code-review → hand-off to human. A complete creatable `DraftPipeline` seed: reviewer-role teams with revise/decline routes, the spec-approval **human gate**, sensible store capacities, prompt bodies — passing hard validation after `to_pipeline()` on the current model. Surfaced in the Basics template picker. Test: the seed is complete + creatable (validates). Commit.
- [ ] **Task 6 — Impeccable pass (frontend-facing).** Over the regenerate-prompt affordance + the template picker + the wizard skills wiring: tokens, `ui/*`, focus-visible, busy/empty/error states, a11y. Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-B5`

## Spec coverage
- Skills during creation (G4) → Task 1. ✓
- Generate prompt bodies (G5) → Task 2; per-node regenerate (G5) → Task 4. ✓
- Well-formed review structure from Generate (G3) → Task 3. ✓
- Bundled DDD template (G15) → Task 5. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. Live `claude`/chat paths stay structural-only (fakes in tests). Keep the `DraftPipeline`/`to_pipeline` contracts intact; the template must be creatable on the current role/store/gate model. Reuse the existing `Step::Prompts`/`PromptSlice` + `llm_chat` seam for regenerate (don't fork the design session).
