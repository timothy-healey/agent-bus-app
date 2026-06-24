# Graph Builder Implementation Plan (chunk ②)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. The detailed design is `docs/superpowers/specs/2026-06-24-graph-builder-design.md` + the language vet `docs/vet-graph-builder-language-2026-06-24.md` — read BOTH first; they are the source of truth for behavior and vocabulary. This plan is the task decomposition + interfaces + chunk-① reconciliation + verification gates.

**Goal:** Make an interactive React-Flow canvas the primary Pipeline Authoring surface (new-project wizard + edit-mode), replacing the Teams/Prompts/Wiring form steps, with `DraftPipeline` as the single source of truth. Frontend-only; no backend change.

**Architecture:** A controlled `PipelineCanvas` component with the same `{draft, onChange}` contract the form steps use today. Structure (nodes/edges) is derived from `draft` via a pure `draftToFlow`; positions live in local state (auto-layout new nodes, preserve dragged ones; never persisted). All reasoning (adapter, mutators, layout) is pure and unit-tested. Per-node editing via a `NodeDrawer`.

**Tech Stack:** React 18, `@xyflow/react` (MIT, bundled by Vite), optional `dagre` for layout, TypeScript, vitest.

---

## Chunk-① reconciliation (do FIRST — the model changed under us)

The model+schema chunk landed `Team.role: "producer" | "reviewer"` and `Team.store: { capacity }` (TS `src/ipc/pipeline.ts`), renamed `Workers.default → min`, and `SCHEMA_VERSION = 3`. Consequences for this chunk:
- The node drawer's Team editor MUST expose **Role** (producer/reviewer), **Scale (min · max)** (the renamed `workers.min`/`max`), and **Store capacity** (`store.capacity`), in addition to Runner/Scope/prompt/name.
- Edge role-awareness reads `Team.role` **when present** and falls back to the `teamRole` regex (`src/lib/pipelineGraph.ts`) only when absent (drafts mid-build may not have set it). Producer source → single **hand-off** edge; reviewer/gate source → **approve · revise · reject**.
- `src/wizard/WiringStep.tsx:draftToPipeline` currently hardcodes `role:"producer"`/`store:{capacity:8}` — once the canvas authors these, thread the real draft values through.
- New `draft.ts` mutators are needed: `setTeamRole`, `setTeamStoreCapacity`, `setTeamWorkers` (min/max). Mirror the existing `setTeamModel`/`setTeamEffort` pattern.

## File structure (from the spec §Files)

New (all under `src/wizard/`):
- `canvas/draftFlow.ts` — pure `draftToFlow(draft) → {nodes, edges}` + `reconcile(prev, next, positions)`. Tolerant of invalid drafts (placeholders, never crash).
- `canvas/mutations.ts` — pure draft mutators: `addNode(draft, kind) → {draft, newId}`, `connect(draft, source, target, routeKind?) → draft` (role-aware), `setRouteKind`, `removeNode`, `removeEdge` (clears dangling refs).
- `canvas/layout.ts` — pure auto-layout (dagre or reuse the longest-path ranking already in `pipelineGraph.ts`).
- `canvas/NodeDrawer.tsx` — per-`NodeKind` editor; header "kind · id". Team→full config; Gate→label/downstream; Join→Lanes/Quorum (N of M)/Early-cancel on reject; Fork→lanes.
- `canvas/nodes/*.tsx` — one custom React-Flow node renderer per `NodeKind` (Team, Gate, Fork, Join, Escalation), driven by the `NodeKind` set (forward-compat F9).
- `PipelineCanvas.tsx` — host: `<ReactFlowProvider>` + palette + canvas + NodeDrawer; controlled `{draft, onChange}`.

Modified:
- `src/lib/pipelineGraph.ts` — consolidate the edge/role vocabulary (Route vocab: approve/revise/reject + hand-off; retire "forward"/"escalate" as edge words); export shared role inference that reads `Team.role` first. Refactor-before-add: reshape, don't duplicate.
- `src/wizard/draft.ts` — add the new mutators (above).
- `src/wizard/NewProjectWizard.tsx` — replace the Teams/Prompts/Wiring steps (and their `ChatDraftPanel` per-step chat) with one **Canvas** step. Keep **Basics** (incl. "Generate") and **Review/Create**. (Per-step chat removal is operator-approved.)
- `src/wizard/PipelineEditor.tsx` — swap the `TeamsStep/PromptsStep/WiringStep` step-trio for `PipelineCanvas`; same `{draft,onChange}` + save path.

Reused unchanged: `DraftPipeline` + all backend (validate/kickoff/seed/create/save), `best_effort_validate`, `addForkJoin`/`removeForkJoin` (W4), W2 advanced-panel field setters.

## Tasks (TDD; pure modules first, then components, then wiring)

- [ ] **Task 1 — Dependency.** `bun add @xyflow/react` (and `dagre` + `@types/dagre` if using dagre). Confirm Vite builds it offline (no CDN). Commit `chore: add @xyflow/react`.
- [ ] **Task 2 — `draft.ts` mutators.** Add `setTeamRole`/`setTeamStoreCapacity`/`setTeamWorkers`; unit tests (pure, immutable update) mirroring existing draft tests. Commit.
- [ ] **Task 3 — `pipelineGraph.ts` consolidation.** Read `Team.role` first (fallback regex); reshape edge vocab to Route terms (approve/revise/reject + hand-off); update its tests. Keep `buildPipelineGraph` (read-only viewer) working. Commit.
- [ ] **Task 4 — `canvas/mutations.ts`** (pure) + tests: add/connect(role-aware)/setRouteKind/removeNode/removeEdge; deleting a node clears routes pointing at it (no dangling refs); fork+join via `addForkJoin`. Commit.
- [ ] **Task 5 — `canvas/draftFlow.ts`** (pure) + tests: `draftToFlow` (nodes+edges from draft, tolerant of partial drafts → placeholder/warning data, never throws) and `reconcile` (preserve existing positions, layout only new nodes). Commit.
- [ ] **Task 6 — `canvas/layout.ts`** (pure) + tests: deterministic positions for a sample graph incl. fork/join/revise-loop. Commit.
- [ ] **Task 7 — node renderers `canvas/nodes/*.tsx`** driven by `NodeKind`; per-kind chrome + validation badge slot. Lightweight render tests. Commit.
- [ ] **Task 8 — `canvas/NodeDrawer.tsx`** + tests: Team editor (name, prompt, Runner [kind/model/effort/api-key env name only — F7], Scope [reads/writes/tools], **Role**, **Scale (min·max)**, **Store capacity**) reusing W2 setters + the new ones; Gate (label/downstream); Join (**Lanes / Quorum (N of M) / Early-cancel on reject** — homes deferred P2/P3 controls); Fork (lanes). Field round-trip tests. Commit.
- [ ] **Task 9 — `PipelineCanvas.tsx`** + component tests: palette add → node in draft (selected, drawer open); draw edge → role-aware Route; delete cleanup; `best_effort_validate` issues → node/edge badges + amber banner; controlled `{draft,onChange}`; positions local/ephemeral. Commit.
- [ ] **Task 10 — Wizard integration.** `NewProjectWizard`: Basics → **Canvas** → Review/Create; remove per-step `ChatDraftPanel` chat (keep Basics "Generate" → renders recommended graph). Update wizard tests. Commit.
- [ ] **Task 11 — Edit-mode integration.** `PipelineEditor`: step-trio → `PipelineCanvas`; same save path. Round-trip test: `from_pipeline` → canvas → `save_pipeline_edits` faithful; build-on-canvas → `to_pipeline` hard-validate passes. Commit.
- [ ] **Task 12 — Impeccable pass (frontend-facing).** Run the impeccable skill over the new canvas/drawer surface: focus rings, tokens (no px/hex literals), `ui/Button` for buttons, a11y (drawer focus-trap/Escape via `useModalA11y`, aria labels), keyboard reachability, empty/loading states. Apply safe fixes. Commit.

## Validation (unchanged)

`best_effort_validate` runs live → node/edge badges + the existing amber banner. Hard validation only at Create/Save (backend untouched). `DraftPipeline ≠ Pipeline` holds.

## Verification gates (must all pass before tagging)

- `cd src-tauri && cargo test --workspace` (should be untouched/green — frontend-only chunk).
- `cd /Users/tim/projects/agent-bus-app && npx vitest run` — all green incl. new pure-module + component tests.
- `npx tsc --noEmit` — clean.
- `bun run build` — green (confirms @xyflow bundles).
- Tag: `git tag plan-graph-builder`.

## Self-review / spec coverage

- Canvas-primary + node drawer (spec Decisions) → Tasks 7–9. ✓
- Every team field editable incl. role/scale/store (chunk-① reconcile) → Task 8. ✓
- Role-aware edges (vet F3/F8) → Tasks 3,4. ✓
- Pre-fill from Generate/seeds renders as graph → Tasks 5,10 (drafts already produced; canvas just renders). ✓
- One shared component (wizard + edit-mode) → Tasks 10,11. ✓
- NodeKind-driven palette/renderers/drawer (F9) → Tasks 7,8,9. ✓
- Join Quorum/Early-cancel authoring (F4, deferred P2/P3) → Task 8. ✓
- Pure-and-tested discipline → Tasks 2–6 are pure + unit-tested. ✓
- No backend/model change in this chunk (model already changed in chunk ①). ✓

## Constraints

Local commits only, NEVER push. Commit per task. Follow the design spec + vet vocabulary exactly (node drawer, hand-off vs verdict edges, Scale, Lanes/Quorum/Early-cancel; retire Inspector/Workers/branches/forward/escalate-as-edge).
