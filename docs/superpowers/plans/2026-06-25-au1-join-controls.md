# AU1 — P2/P3 Join Authoring Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Strict TDD: write the failing test first, watch it fail, then the minimum code to pass, then refactor. Small commits per task. Frontend design tokens only; a11y labels on every control. **No backend changes** — the `Join.cancel_on_reject` / `Join.quorum` fields already exist and round-trip.

**Goal:** Give each authored join a `cancel_on_reject` toggle (P2, default off) and a `quorum` N-of-M numeric control (P3, default unset = all-must-approve), wired into the draft so both round-trip through save/load. Honor the runtime's documented DD7 interplay (a set quorum governs success and ignores `cancel_on_reject`) in the UI, and surface an out-of-range quorum (1..=lanes) as a live best-effort issue.

**Architecture:** The join editing surface is the canvas **NodeDrawer** (`src/wizard/canvas/NodeDrawer.tsx`, `JoinEditor` component) — the same drawer that edits teams/gates/forks. The draft model (`src/wizard/draft.ts`) carries `joins: Join[]` where `Join` is the IPC type from `src/ipc/pipeline.ts` (already has `cancel_on_reject?: boolean` and `quorum?: number`). Round-trip is already wired: `draftToPipeline.ts` passes `d.joins` straight through, and the Rust `to_pipeline`/`from_pipeline` round-trips `quorum`/`cancel_on_reject` (proven by `src-tauri/pipeline/src/draft.rs` tests at lines ~824-834). The live best-effort banner is computed by the backend `best_effort_validate` (`src-tauri/pipeline/src/draft.rs`) via the `best_effort_validate_cmd` IPC and rendered by `PipelineCanvas`.

**Tech Stack:** React 18 / TS / Vite (frontend), Tauri 2 (Rust backend, untouched), vitest + @testing-library/react.

---

## Reconciliation — current state (do FIRST, read before writing any task)

This roadmap item is **partially already built**. Confirm the exact state in-tree before starting; do NOT re-create what exists:

- **`JoinEditor` already renders all three sections** (`src/wizard/canvas/NodeDrawer.tsx` ~lines 529-588): a read-only **Lanes** list (`waits_for`), a **Quorum (N of M)** numeric input (`aria-label={`quorum for ${id}`}`, `min=1`, `max=lanes`, blank = all), and an **Early-cancel on reject** checkbox (`aria-label={`early cancel for ${id}`}`). The interplay is already coded: the checkbox is `disabled={j.quorum != null}` and a hint `"Ignored while a quorum is set."` renders when a quorum is present. Edits flow through an inline `setJoin` patch onto `draft.joins` (so they DO round-trip via `draftToPipeline` → `to_pipeline`).
- **Existing tests** (`src/wizard/canvas/NodeDrawer.test.tsx` ~lines 202-227, `describe("NodeDrawer — join editor …")`): lists lanes, sets a quorum (asserts `joins[0].quorum === 2`), toggles early-cancel (asserts `joins[0].cancel_on_reject === true`). The `joinDraft()` helper seeds `{ id: "join-1", waits_for: ["a","b","c"], downstream: "" }`.
- **GAPS this plan closes** (the parts of AU1 that are NOT yet done):
  1. No test pins the **DD7 interplay** (quorum-set ⇒ early-cancel disabled + hint shown), so it is unprotected against regression.
  2. The quorum mutation is an **inline closure**, not a named pure mutator in `draft.ts` (every other field — role, store, workers — has a tested mutator in `draft.ts`). AU1 asks for round-trip-into-the-draft coverage at the model layer.
  3. **Out-of-range quorum is not a live best-effort issue.** `best_effort_validate` (backend) checks join `waits_for`/`downstream` reachability but NOT quorum range. The hard `validate()` (`src-tauri/pipeline/src/validate.rs` ~lines 46-47, 247-250) has `QuorumOutOfRange { join, quorum, lanes }` and only fires on Save. Since AU1 forbids backend changes, mirror the `1..=lanes` rule **client-side** and feed it into the same issues list the banner renders.
- **Confirm before coding:** open `NodeDrawer.tsx` `JoinEditor`, `draft.ts` (no `setJoinQuorum`/`setJoinCancelOnReject` exist yet — the mutators in-tree are `addForkJoin`/`removeForkJoin` only), `PipelineCanvas.tsx` (`bestEffortValidate(draft).then((i) => { setIssues(i); onValidityChange(i.length===0, i); })` at ~lines 102-112; banner renders `issues` when `showBanner` at ~lines 246-249), and the `Join` interface comment block in `pipeline.ts` (lines 89-104) which documents the DD7 interplay verbatim.

---

## Tasks (TDD)

- [ ] **Task 1 — Named draft mutators for the join fields.** In `src/wizard/draft.ts` add two pure mutators beside `addForkJoin`/`removeForkJoin`:
  - `setJoinQuorum(d: DraftPipeline, joinId: string, quorum: number | undefined): DraftPipeline` — sets/clears `quorum` on the matching join (omit/`undefined` = all-must-approve). When `quorum` is a number, also normalize: drop `cancel_on_reject` is NOT required at the model layer (the field is merely ignored by the runtime) — keep both fields intact so toggling quorum off restores the prior toggle. No-op if the join id is unknown.
  - `setJoinCancelOnReject(d: DraftPipeline, joinId: string, value: boolean): DraftPipeline` — sets `cancel_on_reject`. No-op on unknown id.
  - **Test first** in `src/wizard/draft.test.ts`: setting a quorum writes the number; passing `undefined` clears it back to `undefined` (all-must-approve); setting cancel-on-reject writes the boolean; round-trip through `draftToPipeline` keeps `joins[0].quorum`/`cancel_on_reject` (assert the adapter passes them straight through). Confirm the real `addForkJoin` signature first (it seeds joins WITHOUT these fields, so they start `undefined`). Commit.

- [ ] **Task 2 — Route JoinEditor through the named mutators.** In `NodeDrawer.tsx` `JoinEditor`, replace the inline `setJoin({ quorum })` / `setJoin({ cancel_on_reject })` calls with `setJoinQuorum` / `setJoinCancelOnReject` (import from `../draft`, matching how `TeamEditor` imports its mutators). Keep the exact same `aria-label`s (`quorum for ${id}`, `early cancel for ${id}`), the `min=1`/`max=lanes`/blank-placeholder behavior, and the `disabled={j.quorum != null}` + hint. The existing "sets a quorum" / "toggles early-cancel" tests must still pass unchanged (they assert on `onChange` payload shape, which the mutators preserve). Run `npx vitest run src/wizard/canvas/NodeDrawer.test.tsx` to confirm green. Commit.

- [ ] **Task 3 — Pin the DD7 interplay in a component test.** Add to the `describe("NodeDrawer — join editor …")` block in `NodeDrawer.test.tsx` (TDD — write these, watch them fail against a draft that has NO quorum, then they pass against the existing disabled-binding):
  - With no quorum set: `screen.getByLabelText("early cancel for join-1")` is **enabled** (`not.toBeDisabled()`) and the "Ignored while a quorum is set." hint is **absent**.
  - With `quorum: 2` seeded on the join: the early-cancel checkbox is **disabled** (`toBeDisabled()`) and the hint text **is present** (`getByText(/ignored while a quorum is set/i)`).
  - A controlled-render test (use the `useState` host pattern already in this test file): set a quorum via the quorum input, then assert the early-cancel checkbox becomes disabled — proving the interplay is live, not just initial-render. Commit.

- [ ] **Task 4 — Client-side quorum-range best-effort validation.** Mirror the backend `QuorumOutOfRange` rule (`1..=waits_for.length`) on the frontend so it surfaces in the live issues banner WITHOUT a backend change. Add a pure helper — recommended location `src/ipc/pipeline.ts` (next to `bestEffortValidate`) — e.g. `quorumIssues(draft: DraftPipeline): string[]` returning one message per offending join in the **same wording family** as the backend (`join '<id>' quorum <q> out of range (must be 1..=<lanes>)`). Rule: only flag when `quorum != null` AND (`quorum < 1` OR `quorum > waits_for.length`). **Test first** in `src/ipc/pipeline.test.ts`: in-range (1 and == lanes) ⇒ no issue; 0 ⇒ issue; above lane-count ⇒ issue; `undefined` quorum ⇒ no issue. Commit.

- [ ] **Task 5 — Fold quorum issues into the live banner.** In `PipelineCanvas.tsx`, combine the backend `bestEffortValidate(draft)` result with `quorumIssues(draft)` before `setIssues` / `onValidityChange` (e.g. `const i = [...await bestEffortValidate(draft), ...quorumIssues(draft)]`). This keeps the backend as the hard authority on Save while giving the author live in-canvas feedback. **Test** (`src/wizard/PipelineCanvas.test.tsx`): mock `bestEffortValidate` to resolve `[]`; render with a draft whose join has `quorum` above its lane count and `showBanner` raised; assert the `aria-label="validation issues"` banner contains the out-of-range message; assert `onValidityChange(false, …)` was called. Verify the existing PipelineCanvas tests still pass (the mock for `bestEffortValidate` already exists there — confirm its real name/signature before writing). Commit.

- [ ] **Task 6 — Impeccable + a11y pass (frontend-facing).** Run the impeccable skill over the `JoinEditor` surface: confirm only design tokens (no px/hex literals — the `inp`/`group`/`legend` styles already use tokens), focus-visible on the quorum input and checkbox, the disabled checkbox has a visible disabled affordance, the hint is associated/readable (consider `aria-describedby` linking the checkbox to the hint, and `aria-disabled` semantics), and the quorum input communicates its range. Add an `InfoTip`/`TipLegend` help string for the Quorum and Early-cancel fieldsets in the `HELP` map (matching the existing G8 pattern) explaining the DD7 interplay in one line. Apply only safe, token-only changes; keep all existing `aria-label`s stable so tests stay green. Commit.

## Verification gates (the final task runs all of these; all must pass)

- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `cd /Users/tim/projects/agent-bus-app && npx tsc --noEmit`
- `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo build` (sanity — no backend changes expected)
- Tag: `git tag plan-au1`

## Out of scope / non-goals

- **No backend changes.** `Join.cancel_on_reject` / `Join.quorum`, `to_pipeline`/`from_pipeline` round-trip, the hard `validate()` `QuorumOutOfRange`, and `best_effort_validate` are all left as-is. The frontend mirrors the range rule for live UX only.
- Not adding a quorum control to the read-only ReviewStep/Pipeline viewer (authoring is the NodeDrawer).
- Not changing how lanes (`waits_for`) are authored (edges/fork-join pairing, owned by W4/`addForkJoin`).

## File map

- `src/wizard/draft.ts` — new `setJoinQuorum` / `setJoinCancelOnReject` mutators (Task 1).
- `src/wizard/draft.test.ts` — mutator + round-trip tests (Task 1).
- `src/wizard/canvas/NodeDrawer.tsx` — `JoinEditor` routes through the mutators + help copy (Tasks 2, 6).
- `src/wizard/canvas/NodeDrawer.test.tsx` — DD7 interplay tests (Task 3).
- `src/ipc/pipeline.ts` — `quorumIssues` helper (Task 4).
- `src/ipc/pipeline.test.ts` — `quorumIssues` tests (Task 4).
- `src/wizard/PipelineCanvas.tsx` + `.test.tsx` — fold quorum issues into the live banner (Task 5).
