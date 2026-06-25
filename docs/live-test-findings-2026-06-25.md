# Live-test findings — 2026-06-25

Raw findings from live testing, captured verbatim. **Not yet investigated.** Each is a holding note, not a vetted roadmap item — promote into `docs/v1.1-backlog.md` only after investigation confirms scope.

Status legend: `uninvestigated` (default) · `investigating` · `promoted` (moved to backlog) · `dismissed`.

---

### LF1 · Represent reviewer revisions in the graph + decline behavior — `promoted → G2`

> "need a way to represent reviewers revisions in graph / what happens when decline"

Likely area: graph-builder canvas (how a reviewer's revise edge/loop is shown) + runtime review semantics (what happens on a reviewer decline/reject). To investigate later.

---

### LF2 · Auto-graph handles review nodes poorly (mostly only implementation nodes) — `promoted → G3`

> "the auto graph didn't do so well with each review node. mostly just the implementation nodes"

Likely area: graph-builder auto-layout / recommended-graph generation — review nodes aren't laid out / produced well; mainly the implementation nodes came through. To investigate later.

---

### LF3 · "team has no prompt" validation fires too early — `promoted → G11`

> "team has no prompt message comes up to early, should probably just block going to the next page when the button is pressed, not persists from the start"

Likely area: wizard/graph-builder `best_effort_validate` badge/banner timing — the "no prompt" issue shows from the start instead of gating only on Next/Create press. To investigate later.

---

### LF4 · Native folder picker styling is jarring / off-brand — `promoted → G7`

> "native folder picker styling is jarring and doesn't match"

Likely area: A3 `FolderPickerField` / `pickFolder` (the OS-native dialog). To investigate later (note: the OS dialog can't be themed — may push toward an in-app picker).

---

### LF5 · Node delete affordance + larger non-modal graph view — `promoted → G9 (delete) + G10 (full-page)`

> "can i delete a node? maybe right click actions in graph view. And make the view larger, not inside a modal"

Likely area: graph-builder canvas — node deletion is wired in `mutations.ts` but lacks a UI affordance (right-click/context-menu actions); and the canvas should be a larger full-page surface rather than constrained inside a modal. To investigate later.

---

### LF6 · Prompts section not recognising skills + no prefilled prompt from initial generation — `promoted → G4 (skills) + G5 (prompts)`

> "i don't think the prompts section is actually recognising my skills. and there's no prefilled prompt based off the initial generation"

Likely area: (a) A4 `SkillAutocomplete` / `list_skills` discovery not surfacing the user's skills in the node-drawer prompt field; (b) the kickoff "Generate" recommended graph isn't prefilling team prompt bodies. To investigate later.

---

### LF7 · Scope fields unclear — folder/file pickers + info tooltips — `promoted → G7 (pickers) + G8 (tooltips)`

> "scope fields are unclear, can they be folder pickers / file pickers? should we have an informational question mark next to fields"

Likely area: node-drawer Scope editor (reads/writes/tools) — make reads/writes folder/file pickers (reuse `pickFolder`/a file variant) and add informational `?` tooltips on fields. To investigate later.

---

### LF8 · Model field should be a selector of available models — `promoted → G6`

> "models should probably be a selector based on available models to our claude subscription"

Likely area: node-drawer Runner editor — replace the free-text model input with a selector populated from the models available to the user's Claude subscription (needs a discovery source for available models). To investigate later.

---

### LF9 · Stores/queues not represented in the graph — `promoted → G1`

> "how is the store of outcomes represented by the graph, i just see edges between working nodes but no representation of the queues"

Likely area: graph-builder canvas vs the bounded-buffer model — the per-stage bounded **stores** (queues with capacity/occupancy) aren't visualised; edges only connect working nodes. Question of whether stores become first-class nodes/edge-decorations on the canvas (authoring) and/or how board lane occupancy ties in. To investigate later. (Relates to the runtime redesign Store model + ④e board occupancy indicators.)

---

### LF10 · Delete project option — `promoted → G13`

> "delete project option"

Likely area: a `workspace_remove_project` command + Settings → Projects remove already exist (S1) — finding may be discoverability/placement, or a wish for delete elsewhere (e.g. the project list/topbar). To investigate later.

---

### LF11 · Bundled template for the DDD pipeline use-case — `promoted → G15`

> "i think we should have a template for my usecase research -> spec -> spec-review -> plan -> plan-review -> implement -> code-review -> hand off to human"

Likely area: A2 seed templates (`pipeline/src/seed_template.rs`) — add a bundled template for this exact pipeline: research → spec → spec-review → plan → plan-review → implement → code-review → hand off to human. To investigate later (pairs with the role/store/gate model so the seed is creatable end-to-end).

---

### LF12 · Basics↔Canvas back-nav loses the graph (forces re-Generate) — `promoted → G12`

> "can't move back from graph view to basics and then from basics back to graph without re clicking generate"

Likely area: `NewProjectWizard` step navigation — going Canvas → Basics → Canvas drops the built/generated draft, forcing a re-Generate. The draft should persist across back-nav. To investigate later.

---

### LF13 · Left-nav column + clickable nav tree (from C4 brainstorm) — `promoted → G14`

> "with the changes we make to remove the modal and use screens we could probably have some nicer navigations. We could set up a column on the left with a clickable nav tree on top of the continue+back buttons. Surface the delete action in the switcher."

Design idea raised during the C4 brainstorm (not a raw test finding): once the canvas is full-page (G10), add a left navigation column with a clickable nav tree (Basics / Canvas / Review + project switcher) above the continue/back buttons, and surface delete-project in the switcher (overlaps G13).

---

### LF14 · Project delete fails (FK 787) + leaves remnants — `fixed (tag fix-project-delete-cascade)`

> Live error: "database error: FOREIGN KEY constraint failed (code 787)" on delete-project (ProjectSwitcher). Operator: "delete should cascade so there's no longer any remnants of the project pieces."

Root cause: `ProjectStore::remove` did a bare `DELETE FROM projects` while child rows (conversations/tasks/comments/audit/runs/stores/ledger/workers) reference it; FK enforcement blocked it. Fixed: transactional cascade of ALL DB children + worker rows, + on-disk cleanup (scaffolded subdirs + git-worktree teardown) guarded to NEVER touch `target_repo` (skips if target_repo == root or nested). Commits a158619 / 621f95c / 4c05870 / 9bab60a.

---

### LF15 · Draggable bottom-chat (terminal) height — `fixed`

> "can we make the size of the bottom chat draggable"

Added a drag-to-resize handle on the docked terminal's top edge (pointer drag + ↑/↓ keyboard, clamped [120px, 80vh], role="separator"); the panel uses a resizable height instead of a fixed 280px max. Session-only. Frontend-only.
