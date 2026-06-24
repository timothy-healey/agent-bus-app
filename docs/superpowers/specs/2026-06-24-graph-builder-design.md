---
id: 2026-06-24-graph-builder-design
title: Interactive node graph builder (Pipeline Authoring)
date: 2026-06-24
status: draft
operator: tim.healey@splose.com
related:
  - docs/vet-graph-builder-language-2026-06-24.md
  - DOMAIN.md
  - docs/context-map.md
  - src-tauri/pipeline/src/model.rs
  - src/lib/pipelineGraph.ts
  - src/wizard/*
---

# Interactive node graph builder

## Summary

Make the pipeline graph the **primary authoring surface**. Today authoring is
form-based (`TeamsStep` / `PromptsStep` / `WiringStep` wrapped in
`ChatDraftPanel`, reused by `PipelineEditor` for edit-mode), and the graph
(`PipelineGraph` + `lib/pipelineGraph.ts`) is a read-only auto-laid-out SVG. This
feature replaces the form steps with an **interactive React-Flow canvas** on which
an operator can build a complete pipeline — including creating new nodes — entirely
by direct manipulation, no prompting required. The same canvas powers both the
new-project wizard and edit-mode.

`DraftPipeline` remains the single source of truth; the canvas is a derived view.
The feature is **frontend-only** — no change to `DraftPipeline`, validation,
kickoff, seeds, create, or save — which is what keeps it clear of the in-flight
domain-model work.

## Goals

- Build a full pipeline (teams, gates, forks, joins, escalations + their routes)
  by direct manipulation on a canvas.
- Edit **every** team field from the canvas: name, prompt body, runner
  (kind/model/effort/api-key env var), scope (reads/writes/tools), scale (min/max).
- Pre-fill the canvas with a **recommended graph** from the Basics-step kickoff
  ("Generate") or a template seed; refine it on the canvas.
- One shared canvas component used in both the new-project wizard and edit-mode.
- Vocabulary that matches the domain model (see Ubiquitous language).
- Forward-compatible with new node kinds (e.g. the in-flight data-store nodes)
  without a rewrite.

## Non-goals (v1 boundaries)

- **No position persistence.** Auto-layout on load; manual drag is ephemeral
  (session-only). Nothing new persisted; zero pipeline-YAML change.
- **No command bar / NL accelerator.** Deferred; the only LLM touchpoint is the
  Basics "Generate." (When it returns it sits behind the existing
  `ChatDraftPanel` / `llm_chat` seam, not on the canvas.)
- **No model changes.** `Team.role` and the `workers.default → min` rename are
  handed to the model/L1 session (see Hand-offs). The builder reads `Team.role`
  when present and infers role best-effort until then.
- **Per-step Design Session chat is removed** and replaced by direct
  manipulation. Kickoff "Generate" (Basics) remains. *(Deliberate tradeoff:
  mid-build LLM refinement returns later as the deferred command bar.)*

## Decisions

| Decision | Choice |
|---|---|
| Surface | Canvas-primary + node drawer (graph is THE builder) |
| Foundation | React Flow (`@xyflow/react`, MIT, bundled locally) |
| Source of truth | `DraftPipeline`; React Flow nodes/edges are a derived view |
| Positions | Auto-layout (dagre or reuse longest-path); drag ephemeral |
| Form steps | Replaced by one canvas step; Basics + Review remain |
| Pre-fill | Kickoff "Generate" + template seeds render as a recommended editable graph |
| Surfaces | New-project wizard **and** edit-mode (one shared component) |
| Command bar | Deferred (optional accelerator, not v1) |

## Architecture

### One-way data flow around `DraftPipeline`

```
DraftPipeline ──draftToFlow()──▶ React Flow (nodes + edges, derived view)
      ▲                                   │ user interaction
      │ onChange(next)                    ▼
new DraftPipeline ◀──mutations.ts── addNode / connect / setRouteKind / remove …
```

The canvas is a **controlled component** with the same `{draft, onChange}`
contract the form steps use today, so it drops into `NewProjectWizard` and
`PipelineEditor` where the step trio lives. Interactions never mutate React Flow
state structurally; they call pure draft mutators → a new `DraftPipeline` →
`onChange` → re-derive.

### Reconciliation (the subtle part)

React Flow wants to own node positions for smooth dragging. So:

- **Structure** (which nodes/edges exist + their data) is derived from `draft`.
- **Positions** live in local component state.
- On each draft change, reconcile: add/remove React Flow nodes to match the
  draft, **preserve existing positions**, auto-layout only newly-added nodes.
- Manual drag updates the local position map and is **never persisted**.

### `draftToFlow` is tolerant of invalid drafts

The existing `buildPipelineGraph` assumes a *valid* `Pipeline`. A `DraftPipeline`
mid-build has missing prompts, partial runners, dangling routes. The adapter
renders these gracefully (placeholder / warning badge, never a crash) and shares
role inference + edge logic with `lib/pipelineGraph.ts` (refactor-before-add:
reshape the shared vocabulary rather than duplicate the off-language one).

### Files (all frontend)

New:
- `src/wizard/PipelineCanvas.tsx` — host: React Flow provider + palette + node drawer.
- `src/wizard/canvas/draftFlow.ts` — pure `draftToFlow` + reconcile.
- `src/wizard/canvas/mutations.ts` — pure draft mutators.
- `src/wizard/canvas/layout.ts` — pure auto-layout.
- `src/wizard/canvas/NodeDrawer.tsx` — per-kind node editor (header: kind · id).
- `src/wizard/canvas/nodes/*.tsx` — custom React Flow node renderers per `NodeKind`.

Reused / unchanged:
- `DraftPipeline` type + all backend (validate, kickoff, seed, create, save) —
  **untouched**.
- `best_effort_validate` → node/edge badges.
- `addForkJoin` / `removeForkJoin` (W4) as the fork+join primitive.
- `teamRole` + edge logic from `lib/pipelineGraph.ts` (shared; consolidated to
  Route vocabulary).
- W2 advanced-panel field components reused inside the node drawer.
- `PipelineEditor` swaps the step-trio → `PipelineCanvas`.

### Dependencies

- `@xyflow/react` (MIT) — bundled by Vite (no network; fine for Tauri).
- Auto-layout: `dagre` (handles forks/joins/revise-loops better than naive
  longest-path) **or** reuse the existing longest-path ranking to avoid a second
  dep. Lean: dagre; final call at implementation time.

## Ubiquitous language (from the vet)

Canonical labels (`docs/vet-graph-builder-language-2026-06-24.md`):

| Surface | Model field | Label |
|---|---|---|
| Palette | `NodeKind` | **Team · Gate · Fork · Join · Escalation** (+ future kinds) |
| Edge — from reviewer/gate | `Routes.on_approve/revise/reject` | **approve · revise · reject** |
| Edge — from producer | `Routes.on_approve` | **hand-off** |
| Node editor | (chrome) | **node drawer** (header: kind · id) |
| Team | `runner.*` | **Runner** (kind · model · effort · API-key env var) |
| Team | `scope.*` | **Scope** (reads · writes · tools) |
| Team | `workers.{min,max}` | **Scale** (min · max) |
| Join | `waits_for/quorum/cancel_on_reject` | **Lanes** / **Quorum (N of M)** / **Early-cancel on reject** |

Retired: "Inspector", "Workers", "branches/parallelism", "forward", "escalate"
(as edge words; escalation remains a node kind).

## Interaction model

- **Create node** — drag from palette (or click) → node of that `NodeKind`
  added to the draft with a generated id, auto-placed, selected, drawer open.
  Palette is driven by the `NodeKind` set (forward-compat, F9).
- **Draw edge** — drag from a source port to a target → a `Route` is created.
  Kind is **role-aware**: a **producer** source yields the single **hand-off**;
  a **reviewer/gate** source offers **approve / revise / reject**. Role is read
  from `Team.role` when present, else inferred via `teamRole`.
- **Delete** — deleting an edge clears that route; deleting a node also clears
  routes pointing at it (UI never introduces a dangling reference).
- **Edit node** — node drawer. Team → full config (name, prompt, runner, scope,
  scale). Gate → label / downstream. **Join → Lanes / Quorum (N of M) /
  Early-cancel on reject** (homes the deferred P2/P3 authoring controls). Fork →
  its lanes. Fields reuse the W2 advanced-panel components.
- **Fork/Join** — `addForkJoin`/`removeForkJoin` (W4) as the create primitive
  (fork + paired join together); lanes assigned by drawing edges.

## Validation

Unchanged from today. `best_effort_validate` runs live and maps issues onto
node/edge badges (producer with no prompt → warning dot; dangling route →
highlighted edge) plus the existing amber banner. **Hard validation only at
Create / Save** (backend untouched). `DraftPipeline ≠ Pipeline` holds.

## Surfaces

- **New-project wizard.** Flow: **Basics** (name/root/target-repo + describe →
  *Generate recommended graph*, or pick a template, or start blank) → **Canvas**
  (pre-filled recommendation or empty; full direct manipulation) → **Review →
  Create**. Teams/Prompts/Wiring steps collapse into the canvas + node drawer.
- **Edit-mode.** `PipelineEditor` loads the existing pipeline → `DraftPipeline`
  (via `from_pipeline`, A1) → renders `PipelineCanvas` → `save_pipeline_edits`.
  Same `{draft, onChange}` contract; no backend change.

## Pre-fill (recommended graph)

The Basics "Generate" (`kickoffGenerate` → LLM → `DraftPipeline`) and template
seeds (A2) already produce a draft. With the canvas, that draft simply renders as
a pre-filled, editable graph instead of pre-filled forms — minimal new work. The
**canvas itself never calls the model** (F6); the only LLM touchpoint is Basics.

## Testing

- Pure adapters/mutators/layout (`draftFlow.ts`, `mutations.ts`, `layout.ts`)
  unit-tested (vitest) — the "all reasoning is pure & tested" discipline that
  `lib/pipelineGraph.ts` already follows.
- Component-level tests for `PipelineCanvas` / `NodeDrawer` orchestration
  (palette add, draw edge → route, delete cleanup, drawer field round-trip),
  colocated per the existing pattern.
- Round-trip: build a graph on the canvas → `to_pipeline` → hard-validate passes;
  edit-mode load (`from_pipeline`) → canvas → save → reload is faithful.
- E2E (S4 harness, Linux/Windows CI) may add a canvas build spec later.

## Hand-offs to the model / L1 session

- **`Team.role`** — contract pinned in the vet (`producer | reviewer`, additive
  enum, default `producer`; reviewer emits the verdict triple, producer emits a
  hand-off). Implemented in the model/L1 session. Builder infers best-effort via
  `teamRole` until the field exists, then reads it.
- **`workers.default → min`** rename — separate `remediate` (aligns code with the
  context-map invariant). UI labels "Scale (min · max)" regardless.
- **New node kinds** (data stores / typed containers) — the canvas is built
  `NodeKind`-driven so they slot in as one palette entry + one drawer form + one
  renderer.

## Risks

- React Flow's controlled/derived pattern + reconciliation is the main
  implementation risk; keep positions local and structure derived to avoid a
  two-source-of-truth bug.
- Auto-layout quality for revise-loops/forks — dagre mitigates; fall back to the
  existing ranking if the dep isn't wanted.
- Removing per-step LLM chat is a UX regression for chat-driven authors until the
  deferred command bar returns; flagged for operator sign-off on spec review.
