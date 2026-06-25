# B2 — Canvas interactions (G9, G11, G12)

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Design source: `docs/v1.1-backlog.md` items **G9/G11/G12**. Layers on the B1 full-page `AuthoringLayout` + left-nav tree (tag `plan-B1`).

**Goal:** Node delete affordances on the canvas; validation that only nags on Next/Create; and Basics↔Canvas navigation that never loses the draft.

**Architecture:** All frontend, on the B1 shell. `removeNode` (mutations.ts) already exists — wire UI to it. `best_effort_validate` already runs live — change *when* it's prominent + gates navigation. The nav tree (B1/G14) gains validity gating here.

---

## Reconciliation (B1 end-state)
B1 made the canvas full-page in `AuthoringLayout`, the nav tree free-navigation, and renamed the canvas footer button to "Continue". This chunk refines that: G11 gates Continue/Create + nav-tree steps on validity; G12 makes Basics→Canvas draft-preserving.

## Tasks

- [ ] **Task 1 — Node delete affordance (G9).** `removeNode` is wired in `src/wizard/canvas/mutations.ts`. Add UI: (a) **right-click context menu** on a node (with Delete; extensible for future actions), (b) **Delete/Backspace** on the selected node (React Flow `onNodesDelete` or a key handler — guard against deleting while editing a text field), (c) a **delete button in the `NodeDrawer`** header. All route through `removeNode` (which already clears dangling routes). Component tests for each path. Commit.
- [ ] **Task 2 — Validation timing (G11).** Today `best_effort_validate` surfaces the amber banner + "no prompt" badges from the start. Keep **subtle live inline node badges** (a small dot/marker, not shouty), but only show the **prominent banner AND block** Continue/Create **on press**. Wire the nav-tree (B1/G14) + Continue + Create to: on attempt, if invalid → show the banner + the offending badges prominently + block; otherwise proceed. Tests: badges subtle pre-press; banner+block on invalid Continue/Create; valid proceeds. Commit.
- [ ] **Task 3 — Draft-preserving Basics↔Canvas nav (G12).** Basics currently reaches the canvas only via "Generate" (which overwrites the draft). Add: when a draft already exists, Basics offers a draft-preserving **"Continue →"** (just navigates to the canvas) AND **"Generate" becomes "Regenerate"** with a confirm ("Replace the current draft?") so it never silently clobbers an authored/edited draft. The nav-tree (clicking Canvas from Basics) is also draft-preserving. Tests: Basics→Canvas→Basics→Canvas keeps the draft (no regenerate); Regenerate confirms before replacing. Commit.
- [ ] **Task 4 — Impeccable pass (frontend-facing).** Over the context menu, delete affordances, validation prominence, and the Continue/Regenerate controls: tokens, `ui/Button`, focus-visible, a11y (context menu roles/keyboard + Escape, destructive delete styling, banner as `role=alert`/`--warn`), confirm dialogs. Apply safe fixes. Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-B2`

## Spec coverage
- Node delete: context menu + key + drawer button (G9) → Task 1. ✓
- Subtle live badges, block+banner on press (G11) → Task 2. ✓
- Draft-preserving Continue + Regenerate-with-confirm (G12) → Task 3. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. Frontend only — backend stays green. Keep `PipelineCanvas`'s `{draft,onChange}` contract intact (B4 layers on it next). Don't fork `removeNode`/`best_effort_validate` — wire UI to the existing logic.
