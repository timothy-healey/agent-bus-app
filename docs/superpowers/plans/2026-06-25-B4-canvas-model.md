# B4 — Canvas model representation (G1, G2)

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Design source: `docs/v1.1-backlog.md` items **G1/G2** (cluster C1; "B = stores as first-class nodes"). Canvas representation chunk — touches `draftFlow`/`layout`/`nodes`/edge vocab/`NodeDrawer`. Layers on B1/B2/B3 (canvas contract + optional props stable).

**Goal:** Make the graph faithful to the runtime model: render each team's bounded **store** as a first-class node between producer and consumer, and make a reviewer's **approve / revise / decline** outcomes legible (revise loops back to the writer; decline always terminates at needs-human).

**Architecture:** Stores are **derived/synthetic canvas nodes** projected from `Team.store.capacity` (NO schema/model change — the store already exists on `Team`). `draftToFlow` inserts a store node before each team that has an inbound producer edge and reroutes `producer → store → team`. The store node's drawer edits that team's `store.capacity`. Edge vocabulary/styling gets outcome labels + a curved revise loop + decline→needs-human.

---

## Reconciliation (B1–B3 end-state)
`PipelineCanvas`/`NodeDrawer` gained optional props (`showBanner`, `onValidityChange`, `targetRepo`) — additive, stable. `NodeContextMenu.actions` is the per-node action extension point. The `store` representation here is derived (no `NodeKind` enum change in the model); on the canvas it's a new rendered kind driven by the existing NodeKind-driven renderer map (F9 extensibility) — add a synthetic `"store"` flow-node type for rendering only.

## Tasks

- [ ] **Task 1 — Project store nodes in `draftToFlow` (G1).** For each team with an inbound producer/hand-off/approve edge, synthesize a store node `store:<team-id>` carrying `{ capacity: team.store.capacity, occupancyKey: team-id }` and reroute: the edge that targeted the team now targets the store, and a `store → team` edge is added. The source/generator team (no inbound) has no input store. Pure; unit-test the projection (N producer→team edges become producer→store→team; capacities carried; idempotent). Commit.
- [ ] **Task 2 — Store node renderer + layout (G1).** A `StoreNode` renderer (queue glyph + `▢▢▢ /cap`, compact) added to the NodeKind-driven `nodeTypes` map; `layout.ts` handles the extra rank (store sits between producer and team). Render tests. Commit.
- [ ] **Task 3 — Store node drawer (G1).** Selecting a store node opens a `NodeDrawer` form that edits the owning team's `store.capacity` (reuse `setTeamStoreCapacity`); header "Store · <team-id>". (Occupancy is runtime/board, already in ④e — the authoring drawer edits capacity only.) Tests. Commit.
- [ ] **Task 4 — Legible reviewer outcomes (G2).** Edge vocabulary/rendering: label reviewer edges by outcome (**approve / revise / decline**); render the **revise** edge as an explicit curved loop back to the writer (visually distinct from forward flow); ensure a reviewer's **decline (on_reject)** is drawn to a needs-human/escalation node — and where a reviewer has an `on_reject` but no target, surface it (the engine escalates a reject, so the canvas must show the destination). Touches `draftFlow`/`pipelineGraph` edge vocab + `PipelineCanvas` edge styling. Tests for label + loop + decline rendering. Commit.
- [ ] **Task 5 — Validation: reviewer routes (G2).** `best_effort_validate` (or a canvas-side badge) warns when a `reviewer`-role team has no `revise` route and/or no `decline` route (an under-connected reviewer — the LF2/LF1 root issue). Subtle badge (per B2/G11 timing). Tests. Commit.
- [ ] **Task 6 — Impeccable pass (frontend-facing).** Over store nodes + the new edge vocabulary: tokens, role/edge colors per DESIGN.md, a11y (store node labels, edge-label legibility), the revise-loop curve readable, decline edges clearly terminal. Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-B4`

## Spec coverage
- Stores as first-class (derived) nodes + drawer + layout (G1) → Tasks 1–3. ✓
- Reviewer approve/revise/decline legible + decline→needs-human (G2) → Task 4. ✓
- Reviewer-route validation warning (G2) → Task 5. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. **Derived stores — NO model/schema change** (`Team.store` already exists; project it on the canvas only). Keep the controlled `{draft,onChange}` contract (a store node's edits map back to its team's `store.capacity`; deleting/adding store nodes is not a draft mutation — they're projected). `to_pipeline`/`from_pipeline` unaffected (stores aren't draft teams).
