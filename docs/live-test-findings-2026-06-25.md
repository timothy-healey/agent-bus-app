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

---

### LF16 · Directory picker stopped opening Finder (G7 reverted) — `fixed`

> "the directory picker doesn't open finder anymore, it should be native but the styling of the box that displays the path should match our apps styling"

G7 replaced the native folder dialog with an in-app FileTreePicker. Operator wants the native Finder back for folder selection, just with an app-styled path box. Reverted `FolderPickerField` Browse → native `pickFolder()` (plugin-dialog) + tokenized the path input. (Scope reads/writes keep the in-app FileTreePicker — multi-select-within-repo, where native is poor.) Commit on `main`.

---

### LF17 · Focus ring overflows / clipped on full-width inputs — `fixed`

> "the accent highlighting the textbox overflows the visible section of its container, cutting off the horizontal edges. This happens in multiple places"

Global `:focus-visible` used `outline-offset: 1px` (ring OUTSIDE the box) → clipped by overflow containers on full-width controls. Changed to `outline-offset: -2px` (inset) — clip-safe everywhere in one rule. Commit on `main`.

---

### LF18 · NodeDrawer Delete button overlaps the Drawer close X — `fixed`

> "the delete button is overlapping the x to the team panel"

The NodeDrawer header places Delete flush-right (justify-content: space-between), and the Drawer's absolute close X (right:12) sits over it (body horizontal padding is only 20px). Added `paddingRight: var(--sp-7)` to the header so Delete clears the X. Commit on `main`.

---

### LF19 · Live run: generator failed — empty `--print` prompt — `fixed`

> "app: generator loop step failed for `research`: runner invocation failed: ... Error: Input must be provided either through stdin or as a prompt argument when using --print"

First real `claude` invocation. A run is topic-less (the prompts are the work), so `compose_invocation_message(topic="")` yielded an empty user message → `claude --print` rejects an empty prompt. Only bit the LIVE CLI (FakeRunner ignores the message). Fixed: `engine::invoke` falls back to a non-empty directive (`fallback_user_message`) — "Begin…" for the source, or "Process this work item; read your input artifact at <path>…" for a transformer. The real instructions stay in the system prompt (responsibility + output contract). Commit on `main`.

---

### LF20 · Exit/brake doesn't stop in-flight claude subprocesses — `uninvestigated`

> "does exiting the app send a signal to stop ongoing work?"

No graceful shutdown. The per-team worker loops die with the process (no new claims), but an in-flight `claude` invocation is spawned via blocking `std::process::Command::output()` (no kill handle / process group / death-signal), so it ORPHANS and keeps running after quit — potentially still writing the worktree (implementers mutate the target repo). Brake only blocks new claims; it doesn't kill a running subprocess. On restart `release_orphaned_running` recovers the DB row but doesn't reap the process (risking a double-run). Fix would need: a Tauri `RunEvent::ExitRequested` handler that brakes + reaps children; killable spawns (`.spawn()` + tracked child handles, kill-on-exit; Linux PR_SET_PDEATHSIG / kill the process group); and brake-kills-running semantics. Likely roadmap item(s).

---

### LF21 · React-Flow zoom controls look default / unreadable on dark — `fixed`

> "the styling of the zoom controls on the graph look default and aren't readable around our styling. let's apply app styling to the controls"

global.css HAD overrides for `.react-flow__controls*` but they tied the library's `dist/style.css` on specificity (single class) and lost the cascade. Re-prefixed them with the `.react-flow` ancestor (0,2,x > the lib's 0,1,x) so they reliably win, and covered bg/border/disabled + the icon fill (`svg`/`path`/`*`) so the controls read on the dark surface. Commit on `main`.

---

### LF22 · Terminal chat gives no feedback on send (no "registered/thinking") — `fixed`

> "asking the claude prompt at the bottom didn't show claude thinking, it should indicate it's registered the chat"

`useConversation.send` only updated `turns` AFTER the backend turn resolved (claude + agentic loop) and the `streaming` bubble only appears once deltas arrive — so between send and the response, nothing showed. Fixed: optimistically echo the user's line immediately (replaced by the authoritative turns on resolve) + a "thinking…" indicator (`pending` = busy, shown until streaming/turn lands), wired App `busy → Terminal pending`. Commit on `main`.

---

### LF23 · Started a run, no visible activity — `uninvestigated`

> "i started a run but don't see anything happening"

After LF19 the generator runs `claude`, but a generator pass produces NO work-item until it returns the candidate list, so the board shows no card + no "busy" while the (possibly long) research scan runs — looks idle. Likely needs a run/generator activity indicator (a "researching… / generator running" state, or surface the in-flight source pass) + verify the live generator actually returns parseable items (the output-contract live path is still unproven). To investigate (pairs with the live-run shakedown).

---

### LF24 · Live run: generator child insert fails — FK 787 (hardcoded project_id) — `fixed`

> "app: generator loop step failed for `research`: database error: (code: 787) FOREIGN KEY constraint failed"

The generator (which has no parent task to inherit from) created its child work-items with a hardcoded `project_id = "proj"` (`engine::generate_once`), so the `tasks.project_id -> projects(id)` FK failed live. Tests passed because the test harness's run was *also* `project_id "proj"` (and the in-mem pools don't enforce that FK). Fixed: the generator now takes `project_id` from the RUN row (`run.project_id`) — the source of truth that downstream stages already inherit via `task.project_id`. Regression test: a distinct run project id, assert the generated child carries it (not "proj"). Commit on `main`.

---

### LF25 · generator_ledger records keys BEFORE the work-item commits → lost candidates — `uninvestigated`

> Surfaced diagnosing "where are the candidates": live DB showed 9 keys in `generator_ledger` but 0 tasks (the FK-787/LF24 runs). The research agent worked (9 sensible candidate keys), but `engine::generate_once` calls `record_keys` for the WHOLE batch BEFORE the reserve+commit loop. So any key whose commit fails (FK error pre-LF24, OR backpressure when the downstream store fills mid-loop) stays in the ledger as "found" but was never stored → a future/resumed pass sees it in `found`, emits 0 new, goes dry, and that candidate is **permanently lost** for the run. (An existing comment treats the backpressure case as intentional — but it's wrong: the un-committed key shouldn't be remembered as found.) Fix: record a key in the ledger only AFTER its work-item commit succeeds (move `record_keys` into the loop post-insert, or un-record on failure) so ledger ⟺ committed items stay consistent. Pre-LF24 runs are poisoned (ledger has keys, no tasks → resume goes dry); start a fresh run. To investigate.

---

### LF26 · Worker artifacts written to the app's cwd, not the project root — `uninvestigated`

> Surfaced sweeping the DB before a wipe. Stored candidate artifacts are at `/Users/tim/projects/agent-bus-app/src-tauri/app/artifacts/research/<key>/candidate.md` — under the running APP's cwd, NOT the project root (`/Users/tim/projects/agent-bus-uplift/artifacts/` is empty). The claude worker process has no `current_dir` set (claude_cli spawn never sets cwd) and/or the `${project}`/artifact-dir isn't passed as an absolute project-rooted path, so the agent's relative `artifacts/research/...` writes resolve against the app's run dir. Consequences: (1) artifacts scatter into the app's own source tree (debris); (2) the CardDrawer artifact tab (`read_artifact`, project-rooted) can't find them → blank. Fix: set the worker's `current_dir` to the project root (or the target-repo/worktree) AND/OR make the output-contract artifact path absolute + project-anchored. Also clean the stray `src-tauri/app/artifacts/` debris. To investigate.

### Diagnostic sweep observations (2026-06-25, pre-wipe)
- `invocation_audit`: 9 `error:other` (the FK-787/LF24 failures) + **4 NULL-outcome rows** (`record_start` with no settle — a hard engine-step error leaves the audit row in-flight forever; settle-on-error is incomplete).
- Leaked store reservations: spec-writers occupancy 1 (run fab) + **5** (run 51aa) with far fewer tasks — confirms the reservation-leak (lifecycle reconcile / LF25) live.
- `generator_ledger` = 12 keys vs `tasks` = 3 — confirms LF25 (keys recorded before commit; most never became tasks).
- 3 tasks stuck `queued` at spec-writers (not claimed) — spec-writers not progressing (running binary may predate recent fixes; worth re-checking on a fresh build).

### LF27 · Unhandled promise rejection from Tauri unlisten on cleanup — `fixed`

> `TypeError: undefined is not an object (evaluating 'listeners[eventId].handlerId')` from `useRuntimeEvents.ts`. Tauri 2.11's unlisten is `async () => _unlisten(...)` and throws synchronously inside the injected `unregisterListener` when the listener's internal bookkeeping is already gone (dev/StrictMode subscribe→teardown race). Cleanup fired it fire-and-forget, so the rejection went unhandled. Fix: `safeUnlisten` wraps each unlisten call (try/catch + `.catch` on the returned promise); a throw means already-unregistered, so nothing leaks. Regression test added. Commit on main.

> **LF26 resolution (decided 2026-06-25):** Keep the project/target directory settable — the fix is to *use* it. (1) Set `current_dir` on the spawned `claude` (`runners/src/claude_cli.rs:59`, no `.current_dir()` today) to the work-item's resolved working dir — the worktree for implementers, the target repo for read-only/artifact stages — so the worker never inherits the app cwd. (2) Absolutize the artifact base before it reaches the agent and route the generator pass through the same PathVars substitution the transformer uses. **Artifacts live in the app data dir** (`~/Library/Application Support/.../projects/<id>/artifacts/`), added to the scope writes + `--add-dir`, removed by the project-delete cascade, never polluting target_repo. `read_artifact` reads from the same absolute base.
