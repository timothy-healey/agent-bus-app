# B1 — Canvas shell + navigation (G10, G14, G13)

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Design source: `docs/v1.1-backlog.md` items **G10/G14/G13** (decided 2026-06-25, cluster C3/C4). Frontend restructure of the authoring surface. First build chunk — it restructures the host the later chunks layer on.

**Goal:** Promote the pipeline canvas from a modal to a **full-page authoring view** (new-project + edit-mode), with a **left navigation column** (clickable nav tree + project switcher) replacing linear-only back/next, and **delete-project surfaced in the switcher**.

**Architecture:** A shared full-page authoring layout: left nav column (nav tree: Basics / Canvas / Review + the project switcher with delete) + a main pane hosting the current step (`PipelineCanvas` stays the controlled `{draft,onChange}` component). Both `NewProjectWizard` and `PipelineEditor` render this layout instead of a modal. No backend change beyond reusing the existing `workspace_remove_project`.

**Tech Stack:** React/TS, existing `ui/*` + tokens + `useModalA11y` (focus mgmt now for a view, not a modal).

---

## Tasks

- [ ] **Task 1 — Full-page authoring layout (G10).** Extract a shared `AuthoringLayout` (left nav column + main pane) and move the new-project flow out of the modal: `NewProjectWizard` becomes a full-page view (Basics → Canvas → Review as panes in the main area, not a centered modal). `PipelineEditor` (edit-mode) opens the same full-page view. Preserve the controlled `{draft,onChange}` contract + create/save paths. Keep Escape/focus behaviour appropriate for a full-page view (not a modal trap). Update wizard/editor tests for the new structure. Commit.
- [ ] **Task 2 — Left-nav column + clickable nav tree (G14).** In `AuthoringLayout`'s left column: a clickable nav tree (Basics / Canvas / Review) that navigates directly to a step (not just next/back), with the current step marked and steps gated by validity where appropriate (ties to B2's G11 — for now allow free navigation, B2 refines gating). Keep continue/back buttons beneath the tree. Component tests. Commit.
- [ ] **Task 3 — Project switcher with delete (G13).** Put the project switcher in the left nav column (or topbar, whichever the layout makes natural) and add a **delete-project** action with a confirm, calling the existing `removeProject`/`workspace_remove_project`. Keep the Settings→Projects remove too. After delete, select the next project (or empty state). Tests (mock the command + confirm). Commit.
- [ ] **Task 4 — Impeccable pass (frontend-facing).** Run the impeccable skill over the new full-page layout + left nav + switcher: tokens (no px/hex literals), `ui/Button`, focus-visible, a11y (nav tree as a list/tree with proper roles + keyboard nav, switcher labelled, destructive delete = `danger` + confirm), empty states, responsive. Apply safe fixes. Commit.

## Verification gates (all must pass before tag)
- `cd src-tauri && cargo test --workspace` (frontend chunk — backend should stay green)
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-B1`

## Spec coverage
- Full-page de-modal canvas (G10) → Task 1. ✓
- Left-nav column + clickable nav tree (G14) → Task 2. ✓
- Delete-project in the switcher (G13) → Task 3. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. `PipelineCanvas` stays the controlled `{draft,onChange}` component (don't fork it). Reuse `workspace_remove_project` (no new backend). This chunk is the host for B2/B4 — keep the canvas mount point + `{draft,onChange}` stable so later chunks layer cleanly.
