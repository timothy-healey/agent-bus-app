# Agent Bus App — Plan: Brainstorming new-project wizard (sub-project 3 of 3 — the headline)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the silent fixed-template auto-instantiation with a guided, AI-assisted 5-step new-project wizard. The user describes what they're building; a per-step interactive **Design Session** (over the `llm_chat` ACL) produces a tailored **`DraftPipeline`** — teams, per-team responsibility prompts, and fork/join wiring — which is hard-validated, serialized to YAML + prompt files (written by Workspace), and activated.

**Architecture:** Three contexts change. **Pipeline Authoring** (`pipeline` crate, gaining an `llm_chat` dep) gets a new `DraftPipeline` type (an in-progress, not-yet-valid pipeline, distinct from the validated `Pipeline` aggregate), pure slice-merge helpers (apply a teams / a prompt / a wiring JSON slice), best-effort validation for live editing, and a `design_session` module that runs the one-shot kickoff + per-step chat turns over `llm_chat` (parse a fenced ```json slice from the model's prose, apply it, best-effort validate; the backend is the trust boundary). **Workspace** (`workspace` crate) gains a `write_project_pipeline` OHS command that creates the project sub-dirs and writes `pipelines/<id>.yaml` + `prompts/<team>.md` under the (already-`~`-expanded) project root, path-scoped with the `resolve_under_root` escape guard. The **composition root** (`app` crate) wires a `DesignSessionState` holding an `Arc<dyn ChatRunner>`, exposes the three Design Session Tauri commands (`kickoff_generate`, `design_session_turn`, `create_project_from_draft`), and `create_project_from_draft` orchestrates hard-validate → Workspace create + write → activate (nothing written if invalid). The **frontend** replaces `ProjectWizard` with a 5-step `NewProjectWizard` driving an ephemeral `DesignSession` React state and a shared chat-left + live-editable-draft-right component. **Removed:** the App.tsx `listPipelines`→`instantiateTemplate` path, the `instantiate_template` / `pipeline_list_templates` commands + their IPC wrappers, and the bundled `templates/`.

**Tech Stack:** Rust (serde, serde_json, serde_yaml, async-trait, tokio, thiserror, uuid), the `agent_bus_core` kernel, the `llm_chat` ACL (sub-project 1), the fork/join `Pipeline` v2 schema (sub-project 2), Tauri 2 composition root, React 18 + TypeScript + Vitest.

**Source spec:** `docs/superpowers/specs/2026-06-23-brainstorming-wizard-design.md` (brainstormed + DDD-vetted 2026-06-23). Honour every decision and the DDD resolutions (F1/F2 below).

**DDD anchors (DOMAIN.md + the council vet 2026-06-23):**
- **Design Session is a Pipeline Authoring concept, NOT the `Conversation` aggregate.** It is an ephemeral, AI-assisted authoring dialogue conducted over the `llm_chat` ACL (kernel-only). It has no persistence and no `Conversation` type. One `dialogue_id` per wizard step (`<wizard-session>:<step>`).
- **F2 — `DraftPipeline` is distinct from the validated `Pipeline` aggregate.** A half-built draft cannot satisfy `validate.rs`, so the wizard manipulates a `DraftPipeline`. **Best-effort** validation runs during editing (surfaces errors live, never blocks); **hard** validation (the existing `validate()`) runs at create and must pass before any file is written. Only a valid `DraftPipeline` becomes a `Pipeline`.
- **F1 — Workspace writes files; Pipeline Authoring serializes.** Workspace owns the project filesystem layout (`prompts/`, `pipelines/`) and gains a write command. Pipeline Authoring validates the `DraftPipeline` and serializes it to content (YAML + per-team prompt files). `create_project_from_draft` (composition root) orchestrates validate → `workspace_create_project` + `write_project_pipeline` → activate. Pipeline Authoring never touches the filesystem directly.
- **`pipeline` gains an `llm_chat` edge.** `llm_chat` depends only on `agent_bus_core` (sub-project 1, F2), so `pipeline → llm_chat → agent_bus_core` is acyclic. `pipeline` already depends on `workspace` (for `paths`); no new cycle is introduced. Verified by `cargo tree -p pipeline` in Task 16.

---

## Orientation — the real code this plan builds on

Read these before starting; every task references them.

- **Pipeline Authoring (`pipeline` crate).**
  - `src-tauri/pipeline/src/model.rs` — the `Pipeline` aggregate (post sub-project 2): `Pipeline { id, name, description, schema_version, teams, gates, escalations, forks, joins }`; `Team { id, name, prompt, runner: RunnerConfig, scope: Scope, outputs: Routes, workers: Workers }`; `RunnerConfig { kind: RunnerKind, model, effort: EffortMode, api_key_env: Option<String> }`; `Scope { reads, writes, tools }`; `Routes { on_approve, on_revise, on_reject }` (each `Option<String>`); `Fork { id, lanes: Vec<String> }`; `Join { id, waits_for: Vec<String>, downstream: String }`; `enum NodeKind { Team, Gate, Escalation, Fork, Join }`; `pub const SCHEMA_VERSION: u32 = 2`. `Workers::default() = {default:1, max:1}`.
  - `src-tauri/pipeline/src/validate.rs` — `validate(&Pipeline) -> Result<(), PipelineValidationError>`. This is the **hard** validation reused at create. The error enum is `#[derive(Debug, Error, PartialEq, Eq)]`.
  - `src-tauri/pipeline/src/parse.rs` — `parse_pipeline(yaml) -> Result<Pipeline, PipelineParseError>` (serde_yaml only).
  - `src-tauri/pipeline/src/store.rs` — `PipelineStore` reads/lists/saves YAML under `<root>/pipelines/`; `save()` validates then `serde_yaml::to_string`. **The wizard does NOT write via this store** (Workspace owns the write surface now); the serialization helper produced in this plan reuses `serde_yaml::to_string`.
  - `src-tauri/pipeline/src/template.rs` + `src-tauri/pipeline/templates/ddd-spec-plan-impl.yaml` — the bundled template. **Removed in this plan (Task 14).**
  - `src-tauri/pipeline/src/api.rs` — the OHS. Has `pipeline_list_templates`, `pipeline_list`, `pipeline_load`, `pipeline_instantiate_template`, `TemplateInfo`, `tools()`. **`pipeline_list_templates` + `pipeline_instantiate_template` + `TemplateInfo` removed (Task 14).** `tools()` and the two read commands (`pipeline_list`, `pipeline_load`) stay.
  - `src-tauri/pipeline/src/lib.rs` — `pub mod model/parse/validate/template/store/api;` + `pub use` re-exports; `#[cfg(test)] mod contract_tests;`. This plan adds `pub mod draft;` and `pub mod design_session;` and removes `pub mod template; pub use template::*;`.
  - `src-tauri/pipeline/src/contract_tests.rs` — serde key-set regression tests locking the TS interfaces. Imports `crate::api::TemplateInfo` (its `template_info_key_set_matches_ts` test is removed in Task 14).
  - `src-tauri/pipeline/Cargo.toml` — deps `agent_bus_core`, `workspace`, serde/serde_json/serde_yaml/thiserror/tauri; dev-dep `uuid`. **This plan adds `llm_chat` (Task 1) and `uuid` to non-dev deps (Task 4, for draft ids).**
- **`llm_chat` ACL (sub-project 1).**
  - `src-tauri/llm_chat/src/chat.rs` — `ChatUsage { model, input_tokens, output_tokens, cache_creation, cache_read }`; `ChatRequest { dialogue_id, system_prompt, user_message, model, thinking_budget }`; `ChatReply { text, usage }` (no session_id); `enum ChatError { RateLimited, Spawn, NoResult, Other }` with `is_rate_limited()`; `#[async_trait] trait ChatRunner: Send + Sync { async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError>; }`.
  - `src-tauri/llm_chat/src/fake.rs` — `FakeChatRunner::new(Vec<ChatReply>)` (seeded replies, clamps to last) + `::failing(ChatError)`; records `received: Mutex<Vec<ChatRequest>>`. The Design Session tests drive all chat with this — **no live `claude`**.
  - `src-tauri/llm_chat/src/claude_cli.rs` — `ClaudeChatRunner::new()` is the production runner wired at the root.
- **Workspace (`workspace` crate).**
  - `src-tauri/workspace/src/api.rs` — `WorkspaceState { store: Arc<ProjectStore> }`; `workspace_create_project(state, name, root_path)` (expands `~` once via `expand_tilde` + `home_dir()`, then `Project::new`, `store.insert`); `workspace_set_active_pipeline(state, id, pipeline_id: Option<String>)`; `read_artifact(state, project_id, path)` which `store.get` → `resolve_under_root(&root, &path)` → `std::fs::read_to_string`; the pure `resolve_under_root(root, rel) -> Result<PathBuf, String>` escape guard; `expand_tilde`; `tools()` OHS list. **`write_project_pipeline` is added here (Tasks 8–9). The root is already `~`-expanded at create — do NOT re-expand per write.**
  - `src-tauri/workspace/src/paths.rs` — `project_subdirs() -> &'static [&'static str]` (`["pipelines","prompts","artifacts","worktrees",".agent-bus"]`); `pipelines_dir(root) -> root/pipelines`. **This plan adds `prompts_dir(root) -> root/prompts` (Task 8).**
  - `src-tauri/workspace/src/project.rs` — `Project { id: ProjectId, name, root_path: PathBuf, active_pipeline_id, created_at, updated_at }`.
  - `src-tauri/workspace/src/store.rs` — `ProjectStore` over `SqlitePool`; `get(&ProjectId)`, `set_active_pipeline(&ProjectId, Option<&PipelineId>, now)`.
- **Composition root — `src-tauri/app/src/lib.rs`.** Constructs the pool, every context's state, the catalog union, the chat runner (`let chat_runner: Arc<dyn llm_chat::chat::ChatRunner> = Arc::new(ClaudeChatRunner::new());`), the terminal `LlmEngine`, the worker loops, and the `generate_handler![...]` registration (currently includes `pipeline::api::pipeline_list_templates` and `pipeline::api::pipeline_instantiate_template`). `load_active(...)` resolves the active project + its first pipeline at boot. **This plan: adds a `DesignSessionState` + three Design Session commands; removes the two template command registrations; the worker-loop seeding references `pipe.teams` not templates, so it is untouched.**
  - `src-tauri/app/Cargo.toml` already depends on `pipeline`, `workspace`, `llm_chat`. No new crate deps needed at the root.
- **Frontend (`web-app`).**
  - `src/components/ProjectWizard.tsx` — the current single-screen modal (name + root → `createProject`). **Replaced by `NewProjectWizard` (Tasks 17–24).**
  - `src/components/ProjectWizard.test.tsx` — its tests. **Replaced.**
  - `src/App.tsx` — holds `wizardOpen`, renders `<ProjectWizard>`, and the `useEffect` that calls `listPipelines` then `instantiateTemplate`. **The auto-instantiate effect is ripped out (Task 22); the wizard becomes the only new-project path.**
  - `src/App.test.tsx` — mocks `instantiateTemplate` / `listPipelines`. **Updated (Task 22).**
  - `src/ipc/pipeline.ts` — TS mirrors + `listTemplates` / `instantiateTemplate` wrappers. **Those two wrappers + `TemplateInfo` removed (Task 15); a `DraftPipeline` type + the three Design Session wrappers added (Task 16).**
  - `src/ipc/pipeline.test.ts` — has a `listTemplates` test. **Removed in Task 15; Design Session contract tests added in Task 16.**
  - `src/components/PipelineView.tsx` — read-only graph viewer; renders Teams / Gates / Escalations / Forks / Joins sections. The Wiring step (Task 21) renders a `DraftPipeline` through a draft→`Pipeline`-shaped adapter into this viewer.
  - `src/ipc/workspace.ts` — `createProject`, `getProject`, `readArtifact`, `Project`. Unchanged (the wizard calls Design Session commands, not `createProject` directly).

---

## Spec anchors (source of truth — do not invent)

- **5 steps:** (1) Basics & goal (+ one-shot description) → (2) Teams → (3) Responsibilities→prompts → (4) Wiring → (5) Review & create.
- **One-shot kickoff:** Step 1 "Generate" → `kickoff_generate(description)` → a full `DraftPipeline`; steps 2–4 open pre-filled and are refined by chat.
- **Per-step multi-turn chat** over `llm_chat` drives a **live, editable draft panel** (layout A: chat left, draft right). **Two-way bound:** a chat turn emits a structured slice that updates the draft and is narrated; manual edits to the draft are fed into the next chat turn so the agent interprets them.
- **Team technical config:** smart defaults at kickoff + an optional per-team **advanced panel** (model / effort / tools / scope) — not a required step.
- **Wizard-only:** the fixed-template auto-instantiation is removed; every new project is designed here and always gets prompt files.
- **Ephemeral:** the Design Session (step, draft, basics, per-step transcripts) lives in frontend state; closing mid-wizard discards it. No new persistence.
- **Backend OHS:** `kickoff_generate(description) -> DraftPipeline`; `design_session_turn(step, draft, user_message) -> { reply_text, updated_draft }`; `create_project_from_draft(name, root, draft) -> Project`.
- **Workspace:** `write_project_pipeline(root, pipeline_yaml, prompts: Vec<(path, content)>)` — writes `pipelines/<id>.yaml` + `prompts/<team>.md`, path-scoped with `resolve_under_root`.
- **Structured-emit contract:** the agent emits a fenced ```json block holding only the current step's slice, against a documented mini-schema per step (kept in the system prompt). The backend parses + validates; prose never mutates state directly. Invalid JSON → draft unchanged, prose surfaces the issue.
- **Testing:** pure draft→YAML serialization; slice-merge; best-effort vs hard validation; `kickoff_generate` from a canned reply; `design_session_turn` applies a slice + returns prose; invalid-JSON turn leaves the draft unchanged; `create_project_from_draft` rejects an invalid draft (writes nothing) and on a valid draft calls Workspace write with the expected YAML + prompt set; `write_project_pipeline` writes under the root + rejects path escapes; wizard step nav; two-way sync; review render; `DraftPipeline` IPC contract. All green with **no live `claude`**.
- **DOMAIN.md / config updates (Task 25):** Pipeline Authoring ubiquitous language gains the full **Design Session** def + **DraftPipeline**; Workspace language notes the **`write_project_pipeline`** write surface; **Template** is removed from Workspace's language (marked v1.1-future).

---

## File structure (created / modified)

```
src-tauri/
├── pipeline/
│   ├── Cargo.toml                 M  add llm_chat dep; promote uuid to [dependencies]
│   └── src/
│       ├── lib.rs                 M  + pub mod draft; pub mod design_session;  − pub mod template; pub use template::*;
│       ├── draft.rs               A  DraftPipeline type, slice types, slice-merge, best-effort validate, to_yaml + prompt extraction, draft→Pipeline
│       ├── design_session.rs      A  kickoff + per-step turn logic over llm_chat (system prompts, fenced-json extraction, apply+best-effort)
│       ├── template.rs            D  bundled template (removed)
│       ├── api.rs                 M  remove pipeline_list_templates + pipeline_instantiate_template + TemplateInfo
│       └── contract_tests.rs      M  remove template_info_key_set_matches_ts + TemplateInfo import; add DraftPipeline key-set test
│   └── templates/
│       └── ddd-spec-plan-impl.yaml  D  bundled template YAML (removed)
├── workspace/src/
│   ├── paths.rs                   M  + prompts_dir(root)
│   └── api.rs                     M  + write_project_pipeline command + tools() entry
├── app/src/
│   └── lib.rs                     M  + DesignSessionState; + kickoff_generate/design_session_turn/create_project_from_draft commands; − two template registrations
src/
├── ipc/
│   ├── pipeline.ts                M  − listTemplates/instantiateTemplate/TemplateInfo; + DraftPipeline + kickoffGenerate/designSessionTurn/createProjectFromDraft
│   └── pipeline.test.ts           M  − listTemplates test; + Design Session contract tests
├── wizard/                        A  the wizard feature folder
│   ├── draft.ts                   A  DraftPipeline TS helpers (empty draft, apply manual edits, step enum)
│   ├── draft.test.ts              A  helper unit tests
│   ├── NewProjectWizard.tsx       A  the 5-step wizard shell (ephemeral DesignSession state)
│   ├── NewProjectWizard.test.tsx  A  step-nav + create tests
│   ├── ChatDraftPanel.tsx         A  shared chat-left + editable-draft-right component
│   ├── ChatDraftPanel.test.tsx    A  two-way-sync tests
│   ├── TeamsStep.tsx              A  team cards (add/remove/rename) + advanced panel
│   ├── TeamsStep.test.tsx         A
│   ├── PromptsStep.tsx            A  per-team prompt editor
│   ├── PromptsStep.test.tsx       A
│   ├── WiringStep.tsx             A  fork/join via PipelineView (draft→Pipeline adapter)
│   ├── WiringStep.test.tsx        A
│   ├── ReviewStep.tsx            A  assembled pipeline + all prompt files; Create
│   └── ReviewStep.test.tsx        A
├── components/
│   ├── ProjectWizard.tsx          D  replaced by wizard/NewProjectWizard.tsx
│   ├── ProjectWizard.test.tsx     D
│   └── App.tsx (src/App.tsx)      M  use NewProjectWizard; rip out listPipelines→instantiateTemplate effect
├── App.test.tsx                   M  drop instantiate/listPipelines mocks; wizard is the new-project path
DOMAIN.md                          M  Design Session (full) + DraftPipeline (Authoring); write_project_pipeline (Workspace); remove Template
```

---

## Decisions (resolved ambiguities — autonomous)

- **D1 — `DraftPipeline` is its own type in `pipeline::draft`, never a `Pipeline`.** It mirrors `Pipeline` field-for-field but every collection is allowed to be partial/empty and ids may be blank. Shape: `DraftPipeline { id, name, description, schema_version, teams: Vec<DraftTeam>, forks: Vec<Fork>, joins: Vec<Join>, escalations: Vec<Escalation> }`. `DraftTeam` carries the same fields as `Team` plus the prompt **body** held inline as `prompt_body: String` (the wizard edits prompt *text*, not a path) — `to_pipeline()` converts `prompt_body` into a `prompts/<id>.md` path on the `Team` and surfaces the body separately for file writing. No gates in v1 drafts (the wizard produces team/fork/join graphs; gates remain a v1.1 authoring affordance) — but `to_pipeline()` always emits an empty `gates: vec![]`. `DraftPipeline` derives `Serialize, Deserialize` so it crosses the IPC boundary verbatim.
- **D2 — The structured-emit slice is the FIRST fenced ```json block in the model's prose.** `design_session.rs` extracts it with a small scanner (find ```` ```json ```` … ```` ``` ````; fall back to the first ```` ``` ```` fenced block). The block is parsed into a step-tagged `Slice` enum (`TeamsSlice`, `PromptSlice`, `WiringSlice`) matching a documented mini-schema. **No JSON block, or unparseable JSON → the draft is returned unchanged and the full prose (which explains the problem) is surfaced as `reply_text`.** The prose is never parsed for state — only the fenced block mutates the draft (the backend is the trust boundary).
- **D3 — Best-effort vs hard validation are two functions.** `draft::best_effort_validate(&DraftPipeline) -> Vec<String>` returns a list of human-readable issues (missing prompt, lane referencing an unknown team, etc.) and **never blocks** — it is for live display during editing. **Hard** validation is the existing `validate::validate(&Pipeline)`, run only inside `create_project_from_draft` after `to_pipeline()`, and it must return `Ok(())` before any file is written. Best-effort intentionally tolerates an empty draft (kickoff hasn't run) by returning a single "no teams yet" issue rather than erroring.
- **D4 — `kickoff_generate` and `design_session_turn` are PURE over an injected `&dyn ChatRunner`.** They live in `pipeline::design_session` as free async functions taking `runner: &dyn ChatRunner` (so unit tests pass a `FakeChatRunner` and there is no Tauri/state coupling in the crate). The Tauri command wrappers (which fetch the runner from managed state) live at the composition root (Task 12). This keeps the `pipeline` crate testable headless and keeps `tauri::State` out of the domain logic.
- **D5 — `create_project_from_draft` orchestration lives at the composition root, not in `pipeline`.** It needs three things from different contexts: hard-validate + serialize (`pipeline`), create the project row + write files (`workspace`, whose `ProjectStore` is async sqlx state only reachable via managed state), and set-active (`workspace`). Per the vet, Pipeline Authoring must not touch the filesystem. **Resolution:** `pipeline::draft` exposes the pure pieces — `to_pipeline()` (→ `Pipeline`), `validate()` (reused), `to_yaml(&Pipeline)` (serialize), and `prompt_files(&DraftPipeline) -> Vec<(String, String)>` (`("prompts/<id>.md", body)`). The root command sequences: build `Pipeline` from draft → `validate()` (return Err, write nothing, if invalid) → `workspace_create_project` → `write_project_pipeline` → `workspace_set_active_pipeline`. Workspace owns the bytes-to-disk; Pipeline Authoring owns the bytes.
- **D6 — `write_project_pipeline` takes `project_id` + the already-expanded root from the stored `Project`.** Like `read_artifact`, the command resolves the root from the `Project` row (which holds the `~`-expanded root from create, per the recent tilde fix) and never re-expands. Signature: `write_project_pipeline(state, project_id, pipeline_yaml, prompts: Vec<(String, String)>)`. It `create_dir_all`s every `project_subdirs()` entry under the root, writes `pipelines/<id>.yaml` (the id is derived from the yaml? no — see D7), and each `prompts/<team>.md`, each path passed through `resolve_under_root` (escape guard). Returns `Result<(), String>`.
- **D7 — The pipeline file id/basename is the `DraftPipeline.id`, defaulted at kickoff.** `kickoff_generate` stamps `id` from a slugified project description / a fresh `pipeline-<short-uuid>` if blank. The root computes the yaml path as `pipelines/<pipeline.id>.yaml` and passes the **already-rendered relative path** plus content list to `write_project_pipeline`; the command treats the yaml path as `pipelines/<id>.yaml` derived from the parsed yaml's `id` field (so the command is self-contained and the id is authoritative from the serialized content). To avoid re-parsing, the root passes the yaml relative path explicitly: signature is `write_project_pipeline(state, project_id, yaml_rel_path, pipeline_yaml, prompts)`. Both `yaml_rel_path` and every prompt path are escape-guarded.
- **D8 — The frontend Design Session is a single `useReducer` state object, no backend persistence.** `DesignSession { step: Step, basics: { name, root, description }, draft: DraftPipeline, perStepChat: Record<Step, Turn[]> }`. Closing the wizard discards it. The draft is the single source of truth: a chat turn sends `(step, draft, user_message)` and applies the returned `updated_draft`; a manual edit mutates `draft` locally; the next turn sends the mutated draft (so narration reflects it). `dialogue_id` is `<wizard-session-uuid>:<step>` and is owned inside the backend turn call (the frontend passes `step`; the backend composes the dialogue id) — actually, since `kickoff_generate`/`design_session_turn` are stateless one-shots over the chat runner whose session map keys on `dialogue_id`, the frontend passes a stable `session_id` string it mints once per wizard open, and the backend composes `dialogue_id = "<session_id>:<step>"`. This gives each step its own resumable chat without persistence.
- **D9 — The wizard chat is non-blocking-validating, surfacing best-effort issues inline.** After every kickoff / turn / manual edit, the frontend recomputes nothing server-side for live display in v1 — the backend already returns the `updated_draft` after applying best-effort validation (invalid slices are simply not applied). The Review step is where the user sees the final assembled pipeline; the hard validation error (if any) comes back from `create_project_from_draft` and is shown on the Review step. (Live best-effort issue display from `best_effort_validate` is a backend capability used in tests; the frontend renders the issues list it gets back on Review — Task 23.)
- **D10 — Removal sequencing keeps every boundary green.** Template removal (Task 14) happens **after** the new write path exists and **after** the frontend has stopped importing `instantiateTemplate` (Task 15) and `App.tsx` has dropped the auto-instantiate effect (Task 22). Within Task 14 the order is: remove the api commands + `TemplateInfo`, remove `template.rs` + its module line + `template/` dir, fix `store.rs` (drop `instantiate_template` + the template-using tests), fix `contract_tests.rs`, drop the two root registrations — all in one commit so nothing dangles. `app/src/lib.rs`'s `load_active` does not reference templates, so boot is unaffected (a project with no pipeline file shows the empty placeholder until the wizard writes one).
- **D11 — Per-team advanced panel edits the `DraftTeam.runner`/`scope` in place.** The panel exposes `runner.model` (text), `runner.effort` (preset select mapping to `EffortMode` objects + a custom budget), `scope.tools` (comma list), `scope.reads`/`scope.writes` (comma lists). These are plain controlled inputs over the draft; no backend round-trip. Defaults come from kickoff.

---

## Task 1: `pipeline` gains the `llm_chat` dependency (no cycle)

**Files:**
- Modify: `src-tauri/pipeline/Cargo.toml`

- [ ] **Step 1: Add the dep + promote uuid**

In `src-tauri/pipeline/Cargo.toml`, under `[dependencies]` add `llm_chat` and move `uuid` up from `[dev-dependencies]` (the draft ids need it at runtime):

```toml
[dependencies]
agent_bus_core = { path = "../agent_bus_core" }
workspace = { path = "../workspace" }
llm_chat = { path = "../llm_chat" }
serde.workspace = true
serde_json.workspace = true
serde_yaml.workspace = true
thiserror.workspace = true
tauri = { workspace = true }
uuid = { workspace = true }
async-trait = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

(`async-trait` is needed because `design_session.rs` calls the `#[async_trait] ChatRunner`; `tokio` dev-dep is for `#[tokio::test]`.)

- [ ] **Step 2: Verify it compiles and the dependency graph is acyclic**

Run: `cd src-tauri && cargo build -p pipeline`
Expected: builds clean (no new code yet — just the dep).

Run: `cd src-tauri && cargo tree -p pipeline -i llm_chat`
Expected: shows `llm_chat` is a dependency of `pipeline`, and the inverse tree does NOT list `pipeline` under `llm_chat` (no cycle). If `cargo` reports a cyclic dependency error, STOP — the plan's premise is wrong.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/pipeline/Cargo.toml
git commit -m "build(pipeline): depend on llm_chat ACL (kernel-only, acyclic)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `DraftPipeline` + `DraftTeam` types (`draft.rs`)

**Files:**
- Create: `src-tauri/pipeline/src/draft.rs`
- Modify: `src-tauri/pipeline/src/lib.rs` (register the module)

- [ ] **Step 1: Register the module**

In `src-tauri/pipeline/src/lib.rs`, after `pub mod store;` add:

```rust
pub mod draft;
```

(Leave `pub use` additions for later tasks; the module compiles standalone.)

- [ ] **Step 2: Write the failing test**

Create `src-tauri/pipeline/src/draft.rs` with ONLY the doc + test module first:

```rust
//! DraftPipeline — an in-progress, NOT-yet-valid pipeline the wizard edits
//! (DOMAIN.md → Pipeline Authoring). Distinct from the validated `Pipeline`
//! aggregate (vet F2): a half-built draft cannot satisfy validate.rs, so the
//! wizard manipulates this looser structure. Best-effort validation surfaces
//! issues live; only a draft that passes HARD validation (validate::validate on
//! its to_pipeline()) becomes a `Pipeline`. Prompt text is held inline as
//! `prompt_body`; to_pipeline() converts it to a `prompts/<id>.md` path.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_draft_has_no_teams_and_default_schema_version() {
        let d = DraftPipeline::empty();
        assert!(d.teams.is_empty());
        assert_eq!(d.schema_version, crate::model::SCHEMA_VERSION);
        assert!(d.forks.is_empty());
        assert!(d.joins.is_empty());
    }

    #[test]
    fn draft_round_trips_through_serde_json() {
        let mut d = DraftPipeline::empty();
        d.id = "pl-1".into();
        d.name = "P".into();
        d.teams.push(DraftTeam::new("research", "Research"));
        let s = serde_json::to_string(&d).unwrap();
        let back: DraftPipeline = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn draft_team_new_has_sane_defaults() {
        let t = DraftTeam::new("research", "Research");
        assert_eq!(t.id, "research");
        assert_eq!(t.name, "Research");
        assert_eq!(t.prompt_body, "");
        assert_eq!(t.runner.kind, agent_bus_core::RunnerKind::ClaudeCli);
        assert_eq!(t.runner.effort, agent_bus_core::EffortMode::Standard);
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: FAIL — `cannot find type DraftPipeline` / `DraftTeam`.

- [ ] **Step 4: Implement the types**

Prepend to `src-tauri/pipeline/src/draft.rs` (above the test module):

```rust
use crate::model::{Escalation, Fork, Join, Routes, RunnerConfig, Scope, Workers, SCHEMA_VERSION};
use agent_bus_core::{EffortMode, RunnerKind};
use serde::{Deserialize, Serialize};

/// A team as the wizard edits it. Same fields as `model::Team` except the prompt
/// is held inline as text (`prompt_body`), not a file path — the wizard edits
/// prompt *content*. `to_pipeline()` (Task 5) writes it to `prompts/<id>.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftTeam {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub prompt_body: String,
    pub runner: RunnerConfig,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub outputs: Routes,
    #[serde(default)]
    pub workers: Workers,
}

impl DraftTeam {
    /// A new team with smart defaults (claude-cli, standard effort, 1/1 workers,
    /// empty prompt + scope + routes). `kickoff_generate` overrides these.
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            prompt_body: String::new(),
            runner: RunnerConfig {
                kind: RunnerKind::ClaudeCli,
                model: "claude-opus-4-8".into(),
                effort: EffortMode::Standard,
                api_key_env: None,
            },
            scope: Scope::default(),
            outputs: Routes::default(),
            workers: Workers::default(),
        }
    }
}

/// The in-progress pipeline. Looser than `Pipeline`: ids may be blank, teams may
/// be empty, routes may dangle — none of that is an error here (that is what
/// best-effort validation reports). Only `to_pipeline()` + hard validate gates
/// the transition to a real `Pipeline`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftPipeline {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub teams: Vec<DraftTeam>,
    #[serde(default)]
    pub forks: Vec<Fork>,
    #[serde(default)]
    pub joins: Vec<Join>,
    #[serde(default)]
    pub escalations: Vec<Escalation>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl DraftPipeline {
    /// A blank draft (before kickoff). schema_version defaults to the current
    /// version so a draft that grows fork/join lanes is already v2.
    pub fn empty() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            schema_version: SCHEMA_VERSION,
            teams: vec![],
            forks: vec![],
            joins: vec![],
            escalations: vec![],
        }
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/lib.rs src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): DraftPipeline + DraftTeam (the wizard's in-progress, not-yet-valid pipeline)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Slice types + slice-merge (apply a teams / prompt / wiring slice)

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/draft.rs` `mod tests`:

```rust
    #[test]
    fn teams_slice_replaces_the_team_set_preserving_known_bodies() {
        let mut d = DraftPipeline::empty();
        // an existing team with a written prompt body
        let mut existing = DraftTeam::new("research", "Research");
        existing.prompt_body = "Investigate the codebase.".into();
        d.teams.push(existing);

        // the slice declares research (kept) + a new "writers" team
        let slice = Slice::Teams(TeamsSlice {
            teams: vec![
                SliceTeam { id: "research".into(), name: "Research".into() },
                SliceTeam { id: "writers".into(), name: "Writers".into() },
            ],
        });
        apply_slice(&mut d, slice);

        assert_eq!(d.teams.len(), 2);
        // the existing research team's prompt body is preserved across the merge
        let research = d.teams.iter().find(|t| t.id == "research").unwrap();
        assert_eq!(research.prompt_body, "Investigate the codebase.");
        // the new writers team exists with defaults
        assert!(d.teams.iter().any(|t| t.id == "writers"));
    }

    #[test]
    fn prompt_slice_sets_one_team_body() {
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("research", "Research"));
        apply_slice(&mut d, Slice::Prompt(PromptSlice {
            team_id: "research".into(),
            prompt_body: "You investigate the target repo and write findings.".into(),
        }));
        assert_eq!(d.teams[0].prompt_body, "You investigate the target repo and write findings.");
    }

    #[test]
    fn prompt_slice_for_unknown_team_is_a_no_op() {
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("research", "Research"));
        apply_slice(&mut d, Slice::Prompt(PromptSlice {
            team_id: "ghost".into(),
            prompt_body: "x".into(),
        }));
        assert_eq!(d.teams[0].prompt_body, "");
    }

    #[test]
    fn wiring_slice_replaces_routes_forks_and_joins() {
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("entry", "Entry"));
        d.teams.push(DraftTeam::new("a", "A"));
        d.teams.push(DraftTeam::new("b", "B"));
        apply_slice(&mut d, Slice::Wiring(WiringSlice {
            routes: vec![
                RouteEdge { team_id: "entry".into(), on_approve: Some("fork-1".into()), on_revise: None, on_reject: None },
                RouteEdge { team_id: "a".into(), on_approve: Some("join-1".into()), on_revise: None, on_reject: None },
                RouteEdge { team_id: "b".into(), on_approve: Some("join-1".into()), on_revise: None, on_reject: None },
            ],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["a".into(), "b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "needs-human".into() }],
        }));
        assert_eq!(d.teams.iter().find(|t| t.id == "entry").unwrap().outputs.on_approve.as_deref(), Some("fork-1"));
        assert_eq!(d.forks.len(), 1);
        assert_eq!(d.joins[0].downstream, "needs-human");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: FAIL — `cannot find type Slice` / `TeamsSlice` / `apply_slice` etc.

- [ ] **Step 3: Implement the slice types + `apply_slice`**

Add to `src-tauri/pipeline/src/draft.rs` (above the test module, after the type defs):

```rust
/// One team in a teams slice (id + display name only; technical config keeps its
/// existing/default values across a merge).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceTeam {
    pub id: String,
    pub name: String,
}

/// Step 2 slice: the full team set (add/remove/rename). Known teams keep their
/// prompt body + runner/scope; new teams get defaults; dropped teams are removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamsSlice {
    pub teams: Vec<SliceTeam>,
}

/// Step 3 slice: one team's responsibility prompt text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSlice {
    pub team_id: String,
    pub prompt_body: String,
}

/// One team's routing edges in a wiring slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEdge {
    pub team_id: String,
    #[serde(default)]
    pub on_approve: Option<String>,
    #[serde(default)]
    pub on_revise: Option<String>,
    #[serde(default)]
    pub on_reject: Option<String>,
}

/// Step 4 slice: the fork/join wiring + per-team routes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WiringSlice {
    #[serde(default)]
    pub routes: Vec<RouteEdge>,
    #[serde(default)]
    pub forks: Vec<Fork>,
    #[serde(default)]
    pub joins: Vec<Join>,
}

/// A structured slice the model emits (one per wizard step). Internally tagged on
/// `kind` so a single fenced ```json block round-trips. The backend is the trust
/// boundary: only this typed slice mutates the draft, never the model's prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Slice {
    Teams(TeamsSlice),
    Prompt(PromptSlice),
    Wiring(WiringSlice),
}

/// Apply a slice to the draft in place. Pure; tolerant (unknown team in a prompt
/// slice is a no-op, dropped teams are removed). This is the merge half of the
/// two-way binding.
pub fn apply_slice(draft: &mut DraftPipeline, slice: Slice) {
    match slice {
        Slice::Teams(s) => {
            let mut next: Vec<DraftTeam> = Vec::with_capacity(s.teams.len());
            for st in s.teams {
                // Preserve an existing team's full config across the rename/merge.
                if let Some(existing) = draft.teams.iter().find(|t| t.id == st.id) {
                    let mut kept = existing.clone();
                    kept.name = st.name;
                    next.push(kept);
                } else {
                    next.push(DraftTeam::new(&st.id, &st.name));
                }
            }
            draft.teams = next;
        }
        Slice::Prompt(s) => {
            if let Some(team) = draft.teams.iter_mut().find(|t| t.id == s.team_id) {
                team.prompt_body = s.prompt_body;
            }
        }
        Slice::Wiring(s) => {
            for edge in &s.routes {
                if let Some(team) = draft.teams.iter_mut().find(|t| t.id == edge.team_id) {
                    team.outputs = Routes {
                        on_approve: edge.on_approve.clone(),
                        on_revise: edge.on_revise.clone(),
                        on_reject: edge.on_reject.clone(),
                    };
                }
            }
            draft.forks = s.forks;
            draft.joins = s.joins;
        }
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: PASS — all `draft::` tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): slice types + apply_slice (teams/prompt/wiring two-way merge)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Best-effort validation (live editing surface)

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/draft.rs` `mod tests`:

```rust
    #[test]
    fn best_effort_on_empty_draft_reports_no_teams_only() {
        let issues = best_effort_validate(&DraftPipeline::empty());
        assert_eq!(issues, vec!["draft has no teams yet".to_string()]);
    }

    #[test]
    fn best_effort_flags_a_team_with_no_prompt() {
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("research", "Research"));
        let issues = best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.contains("research") && i.contains("prompt")));
    }

    #[test]
    fn best_effort_flags_a_route_to_an_unknown_node() {
        let mut d = DraftPipeline::empty();
        let mut t = DraftTeam::new("research", "Research");
        t.prompt_body = "x".into();
        t.outputs.on_approve = Some("ghost".into());
        d.teams.push(t);
        let issues = best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.contains("ghost")));
    }

    #[test]
    fn best_effort_is_silent_on_a_complete_linear_draft() {
        let mut d = DraftPipeline::empty();
        let mut a = DraftTeam::new("research", "Research");
        a.prompt_body = "investigate".into();
        a.outputs.on_approve = Some("writers".into());
        let mut b = DraftTeam::new("writers", "Writers");
        b.prompt_body = "write".into();
        d.teams.push(a);
        d.teams.push(b);
        assert_eq!(best_effort_validate(&d), Vec::<String>::new());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::best_effort`
Expected: FAIL — `cannot find function best_effort_validate`.

- [ ] **Step 3: Implement `best_effort_validate`**

Add to `src-tauri/pipeline/src/draft.rs` (after `apply_slice`):

```rust
/// Non-blocking validation for LIVE editing (Decision D3). Returns a list of
/// human-readable issues; NEVER errors and never blocks. The wizard shows these
/// inline. Hard validation (validate::validate on to_pipeline()) is what gates
/// the create. An empty draft is not an error — it reports the single "no teams
/// yet" hint.
pub fn best_effort_validate(draft: &DraftPipeline) -> Vec<String> {
    let mut issues = Vec::new();
    if draft.teams.is_empty() {
        issues.push("draft has no teams yet".to_string());
        return issues;
    }

    // Known node ids: teams + forks + joins + escalations.
    let mut known: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for t in &draft.teams { known.insert(t.id.as_str()); }
    for f in &draft.forks { known.insert(f.id.as_str()); }
    for j in &draft.joins { known.insert(j.id.as_str()); }
    for e in &draft.escalations { known.insert(e.id.as_str()); }

    for t in &draft.teams {
        if t.prompt_body.trim().is_empty() {
            issues.push(format!("team '{}' has no prompt yet", t.id));
        }
        for (label, target) in [
            ("on_approve", t.outputs.on_approve.as_deref()),
            ("on_revise", t.outputs.on_revise.as_deref()),
            ("on_reject", t.outputs.on_reject.as_deref()),
        ] {
            if let Some(target) = target {
                if !known.contains(target) {
                    issues.push(format!("team '{}' {} points at unknown node '{}'", t.id, label, target));
                }
            }
        }
    }
    for f in &draft.forks {
        for lane in &f.lanes {
            if !known.contains(lane.as_str()) {
                issues.push(format!("fork '{}' lane '{}' is not a known team", f.id, lane));
            }
        }
    }
    for j in &draft.joins {
        for w in &j.waits_for {
            if !known.contains(w.as_str()) {
                issues.push(format!("join '{}' waits_for '{}' is not a known team", j.id, w));
            }
        }
        if !known.contains(j.downstream.as_str()) {
            issues.push(format!("join '{}' downstream '{}' is unknown", j.id, j.downstream));
        }
    }
    issues
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::best_effort`
Expected: PASS — 4 best-effort tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): best-effort draft validation (non-blocking live-editing issues)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Draft → `Pipeline` + YAML serialization + prompt file extraction

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/draft.rs` `mod tests`:

```rust
    fn complete_draft() -> DraftPipeline {
        let mut d = DraftPipeline::empty();
        d.id = "demo".into();
        d.name = "Demo".into();
        d.description = "A two-team demo".into();
        let mut a = DraftTeam::new("research", "Research");
        a.prompt_body = "You investigate the repo.".into();
        a.outputs.on_approve = Some("writers".into());
        let mut b = DraftTeam::new("writers", "Writers");
        b.prompt_body = "You write the spec.".into();
        d.teams.push(a);
        d.teams.push(b);
        d
    }

    #[test]
    fn to_pipeline_maps_prompt_body_to_a_prompts_path() {
        let p = complete_draft().to_pipeline();
        assert_eq!(p.id, "demo");
        assert_eq!(p.teams[0].prompt, "prompts/research.md");
        assert_eq!(p.teams[1].prompt, "prompts/writers.md");
        // gates is always empty for a draft-built pipeline (D1)
        assert!(p.gates.is_empty());
    }

    #[test]
    fn to_pipeline_then_hard_validate_passes_for_a_complete_draft() {
        let p = complete_draft().to_pipeline();
        assert_eq!(crate::validate::validate(&p), Ok(()));
    }

    #[test]
    fn prompt_files_are_team_id_addressed_markdown() {
        let files = prompt_files(&complete_draft());
        assert_eq!(files.len(), 2);
        assert!(files.contains(&("prompts/research.md".to_string(), "You investigate the repo.".to_string())));
        assert!(files.contains(&("prompts/writers.md".to_string(), "You write the spec.".to_string())));
    }

    #[test]
    fn to_yaml_round_trips_through_parse() {
        let p = complete_draft().to_pipeline();
        let yaml = to_yaml(&p).unwrap();
        let back = crate::parse::parse_pipeline(&yaml).unwrap();
        assert_eq!(back.id, "demo");
        assert_eq!(back.teams.len(), 2);
        assert_eq!(back.teams[0].prompt, "prompts/research.md");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: FAIL — `no method to_pipeline` / `cannot find function prompt_files` / `to_yaml`.

- [ ] **Step 3: Implement the conversions**

Add to `src-tauri/pipeline/src/draft.rs` (after `best_effort_validate`). Add the `Team`/`Pipeline` imports to the existing `use crate::model::{...}` line at the top — change it to include `Pipeline` and `Team`:

```rust
use crate::model::{Escalation, Fork, Join, Pipeline, Routes, RunnerConfig, Scope, Team, Workers, SCHEMA_VERSION};
```

Then add:

```rust
/// The relative prompt path for a team (`prompts/<id>.md`). Single source of the
/// path convention so to_pipeline() and prompt_files() agree.
fn prompt_path(team_id: &str) -> String {
    format!("prompts/{team_id}.md")
}

impl DraftPipeline {
    /// Convert to a real `Pipeline` (Decision D1/D5). Each team's inline
    /// `prompt_body` becomes a `prompts/<id>.md` path; gates are always empty for
    /// a wizard-built pipeline. The result is NOT yet validated — the caller runs
    /// hard validation (validate::validate) before writing anything.
    pub fn to_pipeline(&self) -> Pipeline {
        Pipeline {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            schema_version: self.schema_version,
            teams: self
                .teams
                .iter()
                .map(|t| Team {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    prompt: prompt_path(&t.id),
                    runner: t.runner.clone(),
                    scope: t.scope.clone(),
                    outputs: t.outputs.clone(),
                    workers: t.workers.clone(),
                })
                .collect(),
            gates: vec![],
            escalations: self.escalations.clone(),
            forks: self.forks.clone(),
            joins: self.joins.clone(),
        }
    }
}

/// The per-team prompt files to write: `("prompts/<id>.md", body)` for every
/// team. Workspace writes these (vet F1); Pipeline Authoring only produces them.
pub fn prompt_files(draft: &DraftPipeline) -> Vec<(String, String)> {
    draft
        .teams
        .iter()
        .map(|t| (prompt_path(&t.id), t.prompt_body.clone()))
        .collect()
}

/// Serialize a validated Pipeline to YAML (reuses serde_yaml, the same shape the
/// PipelineStore writes). Pipeline Authoring serializes; Workspace writes (F1).
pub fn to_yaml(pipeline: &Pipeline) -> Result<String, serde_yaml::Error> {
    serde_yaml::to_string(pipeline)
}

// Keep the unused-import linter quiet for symbols used only by tests / later
// tasks (RunnerConfig/Scope/Workers/Routes/Escalation/Fork/Join/SCHEMA_VERSION
// are all referenced above and in the type defs).
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: PASS — all `draft::` tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): draft to_pipeline + to_yaml + prompt_files (serialize; Workspace writes)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Fenced-JSON slice extraction (`design_session.rs` — pure parser)

**Files:**
- Create: `src-tauri/pipeline/src/design_session.rs`
- Modify: `src-tauri/pipeline/src/lib.rs` (register the module)

- [ ] **Step 1: Register the module**

In `src-tauri/pipeline/src/lib.rs`, after `pub mod draft;` add:

```rust
pub mod design_session;
```

- [ ] **Step 2: Write the failing test**

Create `src-tauri/pipeline/src/design_session.rs` with the doc + test module first:

```rust
//! Design Session — the ephemeral, AI-assisted authoring dialogue that produces a
//! pipeline (DOMAIN.md → Pipeline Authoring). Distinct from the terminal's
//! `Conversation` aggregate; conducted over the llm_chat ACL. This module holds
//! the kickoff one-shot + the per-step turn logic and the structured-emit
//! contract: the model returns PROSE plus a fenced ```json slice; the backend
//! extracts + parses + best-effort-applies the slice (the trust boundary). The
//! prose is never parsed for state.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_fenced_json_block() {
        let prose = "Here are the teams I suggest:\n\n```json\n{\"kind\":\"teams\",\"teams\":[]}\n```\n\nLet me know.";
        let block = extract_json_block(prose).unwrap();
        assert!(block.contains("\"kind\":\"teams\""));
    }

    #[test]
    fn extracts_a_bare_fenced_block_when_no_json_tag() {
        let prose = "ok\n```\n{\"kind\":\"prompt\",\"team_id\":\"a\",\"prompt_body\":\"x\"}\n```";
        let block = extract_json_block(prose).unwrap();
        assert!(block.contains("\"kind\":\"prompt\""));
    }

    #[test]
    fn returns_none_when_no_fence() {
        assert!(extract_json_block("just prose, no block").is_none());
    }

    #[test]
    fn parses_an_extracted_teams_slice() {
        let prose = "```json\n{\"kind\":\"teams\",\"teams\":[{\"id\":\"research\",\"name\":\"Research\"}]}\n```";
        let block = extract_json_block(prose).unwrap();
        let slice = parse_slice(&block).unwrap();
        match slice {
            crate::draft::Slice::Teams(s) => assert_eq!(s.teams[0].id, "research"),
            _ => panic!("expected a teams slice"),
        }
    }

    #[test]
    fn parse_slice_errors_on_garbage() {
        assert!(parse_slice("{not json").is_err());
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib design_session::`
Expected: FAIL — `cannot find function extract_json_block` / `parse_slice`.

- [ ] **Step 4: Implement the extractor + parser**

Prepend to `src-tauri/pipeline/src/design_session.rs` (above the test module):

```rust
use crate::draft::Slice;

/// Extract the first fenced code block from the model's prose. Prefers a
/// ```` ```json ```` fence; falls back to the first bare ```` ``` ```` fence.
/// Returns the block's inner text (no fences), or None when there is no fence.
/// This is the only place the model's free text is scanned for structure
/// (Decision D2).
pub fn extract_json_block(prose: &str) -> Option<String> {
    // Prefer a ```json fence.
    if let Some(start) = prose.find("```json") {
        let after = &prose[start + "```json".len()..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    // Fall back to the first bare ``` fence.
    if let Some(start) = prose.find("```") {
        let after = &prose[start + 3..];
        // Skip an optional language token on the same line.
        let after = match after.find('\n') {
            Some(nl) if !after[..nl].contains("```") => &after[nl + 1..],
            _ => after,
        };
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    None
}

/// Parse an extracted block into a typed Slice (internally tagged on `kind`).
/// Errors on anything that isn't a valid slice — the caller treats an error as
/// "leave the draft unchanged, surface the prose" (Decision D2).
pub fn parse_slice(block: &str) -> Result<Slice, serde_json::Error> {
    serde_json::from_str::<Slice>(block)
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib design_session::`
Expected: PASS — 5 tests green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/lib.rs src-tauri/pipeline/src/design_session.rs
git commit -m "feat(pipeline): fenced-json slice extraction + typed parse (the trust boundary)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: `design_session_turn` + `kickoff_generate` over an injected `ChatRunner`

**Files:**
- Modify: `src-tauri/pipeline/src/design_session.rs`

This is the heart of the backend: one chat turn applies a slice; kickoff is a one-shot that produces a full draft. Both take `runner: &dyn ChatRunner` (D4) so tests use `FakeChatRunner` with no live `claude`.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/design_session.rs` `mod tests`:

```rust
    use crate::draft::{DraftPipeline, DraftTeam};
    use llm_chat::chat::{ChatReply, ChatUsage};
    use llm_chat::fake::FakeChatRunner;

    fn reply(text: &str) -> ChatReply {
        ChatReply { text: text.into(), usage: ChatUsage::default() }
    }

    #[tokio::test]
    async fn kickoff_generate_builds_a_draft_from_a_teams_slice() {
        let canned = "I propose a two-team flow.\n\n```json\n{\"kind\":\"teams\",\"teams\":[\
            {\"id\":\"research\",\"name\":\"Research\"},{\"id\":\"writers\",\"name\":\"Writers\"}]}\n```";
        let runner = FakeChatRunner::new(vec![reply(canned)]);
        let draft = kickoff_generate(&runner, "sess-1", "Build a research+writing pipeline").await;
        assert_eq!(draft.teams.len(), 2);
        assert!(draft.teams.iter().any(|t| t.id == "research"));
        // the description is carried onto the draft
        assert_eq!(draft.description, "Build a research+writing pipeline");
        // a non-empty id is stamped so the yaml has a basename (D7)
        assert!(!draft.id.is_empty());
    }

    #[tokio::test]
    async fn design_session_turn_applies_a_prompt_slice_and_returns_prose() {
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        let canned = "Set the research prompt.\n\n```json\n{\"kind\":\"prompt\",\"team_id\":\"research\",\
            \"prompt_body\":\"You investigate the repo and write findings.\"}\n```";
        let runner = FakeChatRunner::new(vec![reply(canned)]);
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "make research investigate").await;
        assert_eq!(out.updated_draft.teams[0].prompt_body, "You investigate the repo and write findings.");
        // the prose (minus the JSON internals) is surfaced
        assert!(out.reply_text.contains("Set the research prompt."));
    }

    #[tokio::test]
    async fn design_session_turn_with_invalid_json_leaves_the_draft_unchanged() {
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        let before = draft.clone();
        // prose with NO fenced block at all
        let runner = FakeChatRunner::new(vec![reply("I can't do that — please clarify the team.")]);
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "garble").await;
        assert_eq!(out.updated_draft, before); // unchanged
        assert!(out.reply_text.contains("clarify"));
    }

    #[tokio::test]
    async fn design_session_turn_passes_the_step_scoped_dialogue_id() {
        let draft = DraftPipeline::empty();
        let runner = FakeChatRunner::new(vec![reply("ok, no changes.")]);
        let _ = design_session_turn(&runner, "sess-9", Step::Teams, draft, "hi").await;
        let received = runner.received.lock().unwrap();
        assert_eq!(received[0].dialogue_id, "sess-9:teams");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib design_session::`
Expected: FAIL — `cannot find type Step` / `function kickoff_generate` / `design_session_turn` / `TurnResult`.

- [ ] **Step 3: Implement the step enum, system prompts, turn + kickoff**

Add to `src-tauri/pipeline/src/design_session.rs` (above the test module). Extend the top `use` line and add the logic:

```rust
use crate::draft::{apply_slice, best_effort_validate, DraftPipeline};
use llm_chat::chat::{ChatRequest, ChatRunner};
use serde::{Deserialize, Serialize};
```

(keep the existing `use crate::draft::Slice;` — or merge it into the line above.)

```rust
/// The wizard steps that carry a chat (2–4). Step 1 is basics (no chat) and
/// step 5 is review (no chat); the kickoff one-shot is its own call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    Teams,
    Prompts,
    Wiring,
}

impl Step {
    /// The dialogue-id suffix for this step (Decision D8: one dialogue per step).
    fn slug(self) -> &'static str {
        match self {
            Step::Teams => "teams",
            Step::Prompts => "prompts",
            Step::Wiring => "wiring",
        }
    }

    /// The step-specific system prompt. Each documents the fenced-json mini-schema
    /// the model must emit (the structured-emit contract). The schemas mirror the
    /// `Slice` variants in draft.rs — keep them in sync.
    fn system_prompt(self) -> &'static str {
        match self {
            Step::Teams => TEAMS_SYSTEM_PROMPT,
            Step::Prompts => PROMPTS_SYSTEM_PROMPT,
            Step::Wiring => WIRING_SYSTEM_PROMPT,
        }
    }
}

const KICKOFF_SYSTEM_PROMPT: &str = "\
You are designing a multi-team Claude Code agent pipeline from a one-line \
description. Reply with a short paragraph of prose, THEN a fenced ```json block \
containing ONLY the team set, exactly this schema:\n\
```json\n{\"kind\":\"teams\",\"teams\":[{\"id\":\"<slug>\",\"name\":\"<Display Name>\"}]}\n```\n\
Use 2–5 teams. ids are lowercase slugs. Do not include anything but the json in \
the fenced block.";

const TEAMS_SYSTEM_PROMPT: &str = "\
You are refining the TEAM SET of a pipeline being designed. Reply with prose, \
THEN a fenced ```json block with the FULL updated team set (this replaces the \
previous set), schema:\n\
```json\n{\"kind\":\"teams\",\"teams\":[{\"id\":\"<slug>\",\"name\":\"<Display Name>\"}]}\n```\n\
Preserve existing team ids the user wants to keep. Only the json mutates state.";

const PROMPTS_SYSTEM_PROMPT: &str = "\
You are writing ONE team's responsibility prompt. Reply with prose, THEN a fenced \
```json block, schema:\n\
```json\n{\"kind\":\"prompt\",\"team_id\":\"<existing team id>\",\"prompt_body\":\"<the operating prompt>\"}\n```\n\
team_id must be one of the existing teams. Only the json mutates state.";

const WIRING_SYSTEM_PROMPT: &str = "\
You are wiring the pipeline's flow (routes + optional fork/join lanes). Reply with \
prose, THEN a fenced ```json block, schema:\n\
```json\n{\"kind\":\"wiring\",\
\"routes\":[{\"team_id\":\"<id>\",\"on_approve\":\"<id|null>\",\"on_revise\":null,\"on_reject\":null}],\
\"forks\":[{\"id\":\"fork-1\",\"lanes\":[\"<team id>\",\"<team id>\"]}],\
\"joins\":[{\"id\":\"join-1\",\"waits_for\":[\"<team id>\",\"<team id>\"],\"downstream\":\"<id>\"}]}\n```\n\
A fork must have >=2 lanes; routes stay single-target. Only the json mutates state.";

/// Build the user message for a turn: the user's words plus the current draft as
/// JSON, so manual edits the user made (the other half of the two-way binding,
/// Decision D8) are visible to the model.
fn turn_user_message(user_message: &str, draft: &DraftPipeline) -> String {
    let draft_json = serde_json::to_string(draft).unwrap_or_default();
    format!("{user_message}\n\nCurrent draft (JSON):\n{draft_json}")
}

/// The result of one Design Session turn: the assistant's prose + the draft after
/// applying any extracted slice (unchanged if none/invalid).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnResult {
    pub reply_text: String,
    pub updated_draft: DraftPipeline,
}

/// One-shot kickoff (Decision D7): generate a full team set from the description.
/// Stamps a non-empty id + carries the description. Best-effort applies the teams
/// slice. Never errors — a bad reply just yields a draft with no teams (the
/// wizard can retry). dialogue_id is `<session_id>:kickoff`.
pub async fn kickoff_generate(
    runner: &dyn ChatRunner,
    session_id: &str,
    description: &str,
) -> DraftPipeline {
    let mut draft = DraftPipeline::empty();
    draft.description = description.to_string();
    draft.id = slug_id(description);

    let req = ChatRequest {
        dialogue_id: format!("{session_id}:kickoff"),
        system_prompt: KICKOFF_SYSTEM_PROMPT.to_string(),
        user_message: description.to_string(),
        model: "claude-opus-4-8".to_string(),
        thinking_budget: 8192,
    };
    if let Ok(reply) = runner.chat(&req).await {
        if let Some(block) = extract_json_block(&reply.text) {
            if let Ok(slice) = parse_slice(&block) {
                apply_slice(&mut draft, slice);
            }
        }
    }
    draft
}

/// One Design Session turn (Decision D2/D4). Sends the user's words + the current
/// draft; extracts + applies the fenced slice (unchanged if none/invalid);
/// returns the prose + updated draft. best_effort_validate is run for its side of
/// surfacing issues to the caller via logging in v1 (the returned draft already
/// reflects only valid mutations).
pub async fn design_session_turn(
    runner: &dyn ChatRunner,
    session_id: &str,
    step: Step,
    mut draft: DraftPipeline,
    user_message: &str,
) -> TurnResult {
    let req = ChatRequest {
        dialogue_id: format!("{session_id}:{}", step.slug()),
        system_prompt: step.system_prompt().to_string(),
        user_message: turn_user_message(user_message, &draft),
        model: "claude-opus-4-8".to_string(),
        thinking_budget: 8192,
    };
    let reply_text = match runner.chat(&req).await {
        Ok(reply) => {
            if let Some(block) = extract_json_block(&reply.text) {
                if let Ok(slice) = parse_slice(&block) {
                    apply_slice(&mut draft, slice);
                }
            }
            reply.text
        }
        Err(e) => format!("[design session error] {e}"),
    };
    // best-effort issues are computed for completeness (live-display capability,
    // D3/D9); they do not block and are not returned in v1's TurnResult.
    let _issues = best_effort_validate(&draft);
    TurnResult { reply_text, updated_draft: draft }
}

/// Slugify a description into a pipeline id; fall back to a fresh id when blank.
fn slug_id(description: &str) -> String {
    let slug: String = description
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        format!("pipeline-{}", &uuid::Uuid::new_v4().to_string()[..8])
    } else {
        slug
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib design_session::`
Expected: PASS — all `design_session::` tests green.

- [ ] **Step 5: Run the whole pipeline crate**

Run: `cd src-tauri && cargo test -p pipeline`
Expected: PASS — model / parse / validate / template (still present) / store / draft / design_session / contract all green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/design_session.rs
git commit -m "feat(pipeline): kickoff_generate + design_session_turn over llm_chat (FakeChatRunner-tested)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Workspace `prompts_dir` path helper

**Files:**
- Modify: `src-tauri/workspace/src/paths.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/workspace/src/paths.rs` `mod tests`:

```rust
    #[test]
    fn prompts_dir_is_project_root_join_prompts() {
        assert_eq!(prompts_dir(Path::new("/p")), PathBuf::from("/p/prompts"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p workspace --lib paths::prompts_dir`
Expected: FAIL — `cannot find function prompts_dir`.

- [ ] **Step 3: Implement it**

In `src-tauri/workspace/src/paths.rs`, after `pipelines_dir`, add:

```rust
/// The directory per-team prompt markdown files live in, relative to a project
/// root. Workspace owns this layout vocabulary (vet F1).
pub fn prompts_dir(project_root: &Path) -> PathBuf {
    project_root.join("prompts")
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p workspace --lib paths::prompts_dir`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/workspace/src/paths.rs
git commit -m "feat(workspace): prompts_dir path helper

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Workspace `write_project_pipeline` OHS command

**Files:**
- Modify: `src-tauri/workspace/src/api.rs`

The write surface (vet F1). Resolves the root from the stored `Project` (already `~`-expanded — D6), creates the project sub-dirs, writes the YAML + each prompt file, each path escape-guarded with `resolve_under_root`.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/workspace/src/api.rs` `mod tests`. The command needs a `WorkspaceState` with a project row; build an in-memory pool + store like the existing tests in the crate. Add this test (and the imports it needs):

```rust
    use crate::project::Project;
    use crate::store::ProjectStore;
    use std::sync::Arc as StdArc;

    async fn state_with_project(root: &std::path::Path) -> (WorkspaceState, String) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        let store = StdArc::new(ProjectStore::new(pool));
        let project = Project::new("Demo".into(), root.to_path_buf(), 0);
        store.insert(&project).await.unwrap();
        (WorkspaceState { store }, project.id.0)
    }

    #[tokio::test]
    async fn write_project_pipeline_writes_yaml_and_prompts_under_root() {
        let root = std::env::temp_dir().join(format!("abp-wpp-{}", uuid::Uuid::new_v4()));
        let (state, project_id) = state_with_project(&root).await;
        let tstate = tauri::State::from(&state);
        write_project_pipeline_inner(
            &state,
            project_id,
            "pipelines/demo.yaml".into(),
            "id: demo\nname: Demo\n".into(),
            vec![("prompts/research.md".into(), "investigate".into())],
        )
        .await
        .unwrap();
        let _ = tstate; // (the inner fn is what we test; the command just wraps it)
        assert_eq!(std::fs::read_to_string(root.join("pipelines/demo.yaml")).unwrap(), "id: demo\nname: Demo\n");
        assert_eq!(std::fs::read_to_string(root.join("prompts/research.md")).unwrap(), "investigate");
        // the canonical subdirs were created
        assert!(root.join("artifacts").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn write_project_pipeline_rejects_a_path_escape() {
        let root = std::env::temp_dir().join(format!("abp-wpp-{}", uuid::Uuid::new_v4()));
        let (state, project_id) = state_with_project(&root).await;
        let err = write_project_pipeline_inner(
            &state,
            project_id,
            "../escape.yaml".into(),
            "x".into(),
            vec![],
        )
        .await
        .unwrap_err();
        assert!(err.contains("escape") || err.contains("relative"));
        let _ = std::fs::remove_dir_all(&root);
    }
```

(Note: the test drives a pure-ish `write_project_pipeline_inner(&WorkspaceState, ...)` so it doesn't need a real Tauri `State` wrapper; the `#[tauri::command]` is a thin wrapper over it — same split the crate already uses conceptually.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p workspace --lib api::write_project_pipeline`
Expected: FAIL — `cannot find function write_project_pipeline_inner`.

- [ ] **Step 3: Implement the inner fn + the command + the imports**

In `src-tauri/workspace/src/api.rs`, add `use crate::paths::project_subdirs;` near the top imports, then add (after `read_artifact`):

```rust
/// Inner write logic (testable without a Tauri State wrapper). Resolves the
/// project root from the stored Project (already ~-expanded at create — D6; do
/// NOT re-expand), creates the canonical sub-dirs, and writes the pipeline YAML +
/// each prompt file. Every relative path is escape-guarded with
/// resolve_under_root (the same guard read_artifact uses).
pub async fn write_project_pipeline_inner(
    state: &WorkspaceState,
    project_id: String,
    yaml_rel_path: String,
    pipeline_yaml: String,
    prompts: Vec<(String, String)>,
) -> Result<(), String> {
    let project = state
        .store
        .get(&ProjectId(project_id))
        .await
        .map_err(|e| e.to_string())?;
    let root = project.root_path.to_string_lossy().into_owned();

    // Create the canonical project sub-dirs (Workspace owns the layout).
    for sub in project_subdirs() {
        std::fs::create_dir_all(Path::new(&root).join(sub)).map_err(|e| e.to_string())?;
    }

    // Write the pipeline YAML (path-scoped).
    let yaml_path = resolve_under_root(&root, &yaml_rel_path)?;
    if let Some(parent) = yaml_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&yaml_path, pipeline_yaml).map_err(|e| e.to_string())?;

    // Write each prompt file (path-scoped).
    for (rel, body) in prompts {
        let path = resolve_under_root(&root, &rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, body).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// OHS command: write a project's pipeline YAML + per-team prompt files
/// (vet F1 — Workspace owns the bytes-to-disk; Pipeline Authoring serializes).
#[tauri::command(rename_all = "snake_case")]
pub async fn write_project_pipeline(
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    yaml_rel_path: String,
    pipeline_yaml: String,
    prompts: Vec<(String, String)>,
) -> Result<(), String> {
    write_project_pipeline_inner(&state, project_id, yaml_rel_path, pipeline_yaml, prompts).await
}
```

Then add the tool to `tools()` (after the `read_artifact` ToolSpec):

```rust
        ToolSpec {
            name: "write_project_pipeline".into(),
            description: "Write a project's pipeline YAML + per-team prompt files under the project root.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "yaml_rel_path": { "type": "string" },
                    "pipeline_yaml": { "type": "string" },
                    "prompts": { "type": "array" }
                },
                "required": ["project_id", "yaml_rel_path", "pipeline_yaml", "prompts"]
            }),
            supplier_context: "workspace".into(),
        },
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p workspace --lib api::`
Expected: PASS — the two new write tests + existing `tools_*` / `resolve_under_root_*` / `expand_tilde_*` green. (A `tools_publishes_workspace_named_tools`-style assertion may now want a `write_project_pipeline` check — add `assert!(t.iter().any(|s| s.name == "write_project_pipeline"));` to that test if you wish; not required.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/workspace/src/api.rs
git commit -m "feat(workspace): write_project_pipeline OHS command (YAML + prompts, escape-guarded)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: Workspace IPC wrapper for `write_project_pipeline`

**Files:**
- Modify: `src/ipc/workspace.ts`

(The frontend doesn't call this directly — `create_project_from_draft` orchestrates it server-side — but the wrapper + a contract test lock the IPC shape and document the surface.)

- [ ] **Step 1: Write the failing test**

Create `src/ipc/workspace.write.test.ts`:

```ts
import { describe, expect, it, vi, beforeEach } from "vitest";
import { writeProjectPipeline } from "./workspace";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("writeProjectPipeline ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("passes project_id, yaml path, yaml + prompts", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await writeProjectPipeline("proj-1", "pipelines/demo.yaml", "id: demo\n", [["prompts/a.md", "body"]]);
    expect(invokeMock).toHaveBeenCalledWith("write_project_pipeline", {
      project_id: "proj-1",
      yaml_rel_path: "pipelines/demo.yaml",
      pipeline_yaml: "id: demo\n",
      prompts: [["prompts/a.md", "body"]],
    });
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/ipc/workspace.write.test.ts`
Expected: FAIL — `writeProjectPipeline` is not exported.

- [ ] **Step 3: Implement the wrapper**

Append to `src/ipc/workspace.ts`:

```ts
export async function writeProjectPipeline(
  projectId: string,
  yamlRelPath: string,
  pipelineYaml: string,
  prompts: [string, string][],
): Promise<void> {
  await invoke<void>("write_project_pipeline", {
    project_id: projectId,
    yaml_rel_path: yamlRelPath,
    pipeline_yaml: pipelineYaml,
    prompts,
  });
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/ipc/workspace.write.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/workspace.ts src/ipc/workspace.write.test.ts
git commit -m "feat(web): writeProjectPipeline IPC wrapper + contract test

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: Composition root — `DesignSessionState` + `kickoff_generate` / `design_session_turn` commands

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

The two chat commands. They fetch the runner from a new managed `DesignSessionState` and delegate to the pure `pipeline::design_session` functions (D4).

- [ ] **Step 1: Add the state struct + the two commands**

In `src-tauri/app/src/lib.rs`, near the other state structs (after `LlmEngine`'s impls, before `ToolDispatcher for RootDispatcher`), add:

```rust
use pipeline::draft::DraftPipeline;
use pipeline::design_session::{design_session_turn, kickoff_generate, Step, TurnResult};

/// Holds the chat runner for the wizard's Design Session (ephemeral; no
/// persistence). The same Arc<dyn ChatRunner> the terminal uses can be shared.
pub struct DesignSessionState {
    pub runner: Arc<dyn ChatRunner>,
}

/// OHS: one-shot kickoff — generate a full DraftPipeline from the description.
#[tauri::command(rename_all = "snake_case")]
async fn kickoff_generate_cmd(
    state: tauri::State<'_, DesignSessionState>,
    session_id: String,
    description: String,
) -> Result<DraftPipeline, String> {
    Ok(kickoff_generate(state.runner.as_ref(), &session_id, &description).await)
}

/// OHS: one Design Session turn — apply a slice + return prose + updated draft.
#[tauri::command(rename_all = "snake_case")]
async fn design_session_turn_cmd(
    state: tauri::State<'_, DesignSessionState>,
    session_id: String,
    step: Step,
    draft: DraftPipeline,
    user_message: String,
) -> Result<TurnResult, String> {
    Ok(design_session_turn(state.runner.as_ref(), &session_id, step, draft, &user_message).await)
}
```

- [ ] **Step 2: Manage the state at startup**

In the `.setup(|app| { ... })` block, after the `chat_runner` is constructed (the line `let chat_runner: Arc<dyn llm_chat::chat::ChatRunner> = Arc::new(...)`), add:

```rust
                handle.manage(DesignSessionState { runner: chat_runner.clone() });
```

- [ ] **Step 3: Register the two commands**

In the `invoke_handler![ ... ]` list, after `pipeline::api::pipeline_load,` add:

```rust
            kickoff_generate_cmd,
            design_session_turn_cmd,
```

(They are free functions in the `app` crate, so they are referenced bare.)

- [ ] **Step 4: Write the failing test**

Add a test module at the bottom of `src-tauri/app/src/lib.rs` (mirrors the existing `llm_engine_tests` pattern; uses `FakeChatRunner`, no live `claude`):

```rust
#[cfg(test)]
mod design_session_tests {
    use super::*;
    use llm_chat::chat::{ChatReply, ChatUsage};
    use llm_chat::fake::FakeChatRunner;
    use pipeline::design_session::{design_session_turn, kickoff_generate, Step};

    #[tokio::test]
    async fn root_kickoff_produces_a_draft_from_a_canned_reply() {
        let canned = "two teams.\n```json\n{\"kind\":\"teams\",\"teams\":[{\"id\":\"research\",\"name\":\"Research\"}]}\n```";
        let runner = FakeChatRunner::new(vec![ChatReply { text: canned.into(), usage: ChatUsage::default() }]);
        let draft = kickoff_generate(&runner, "s1", "design a flow").await;
        assert_eq!(draft.teams.len(), 1);
        assert_eq!(draft.teams[0].id, "research");
    }

    #[tokio::test]
    async fn root_turn_applies_a_teams_slice() {
        let runner = FakeChatRunner::new(vec![ChatReply {
            text: "ok\n```json\n{\"kind\":\"teams\",\"teams\":[{\"id\":\"a\",\"name\":\"A\"},{\"id\":\"b\",\"name\":\"B\"}]}\n```".into(),
            usage: ChatUsage::default(),
        }]);
        let draft = pipeline::draft::DraftPipeline::empty();
        let out = design_session_turn(&runner, "s1", Step::Teams, draft, "add two teams").await;
        assert_eq!(out.updated_draft.teams.len(), 2);
    }
}
```

- [ ] **Step 5: Run the tests + build**

Run: `cd src-tauri && cargo test -p app design_session_tests`
Expected: PASS — both tests green.

Run: `cd src-tauri && cargo build -p app`
Expected: builds (commands registered, state managed).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): DesignSessionState + kickoff_generate/design_session_turn commands

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Composition root — `create_project_from_draft` orchestration command

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

The orchestration (D5): hard-validate → create project → write files → activate. Nothing is written if invalid.

- [ ] **Step 1: Write the failing test**

Add to the `design_session_tests` module in `src-tauri/app/src/lib.rs`:

```rust
    use pipeline::draft::{DraftPipeline, DraftTeam};
    use workspace::api::WorkspaceState;
    use workspace::store::ProjectStore;
    use std::sync::Arc as StdArc;

    async fn workspace_state(pool: sqlx::SqlitePool) -> WorkspaceState {
        sqlx::query(include_str!("../migrations/001_initial.sql")).execute(&pool).await.unwrap();
        WorkspaceState { store: StdArc::new(ProjectStore::new(pool)) }
    }

    fn complete_draft(root_friendly_id: &str) -> DraftPipeline {
        let mut d = DraftPipeline::empty();
        d.id = root_friendly_id.into();
        d.name = "Demo".into();
        let mut a = DraftTeam::new("research", "Research");
        a.prompt_body = "investigate".into();
        a.outputs.on_approve = Some("writers".into());
        let mut b = DraftTeam::new("writers", "Writers");
        b.prompt_body = "write".into();
        d.teams.push(a);
        d.teams.push(b);
        d
    }

    #[tokio::test]
    async fn create_from_an_invalid_draft_writes_nothing_and_errors() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let root = std::env::temp_dir().join(format!("abp-cpfd-bad-{}", uuid::Uuid::new_v4()));
        // an invalid draft: a team routes to a non-existent node -> hard validate fails
        let mut d = complete_draft("bad");
        d.teams[0].outputs.on_approve = Some("ghost".into());
        let err = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), d)
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        // nothing written
        assert!(!root.join("pipelines").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn create_from_a_valid_draft_writes_files_and_activates() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let root = std::env::temp_dir().join(format!("abp-cpfd-ok-{}", uuid::Uuid::new_v4()));
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"))
            .await
            .unwrap();
        // files written
        assert!(root.join("pipelines/demo.yaml").exists());
        assert_eq!(std::fs::read_to_string(root.join("prompts/research.md")).unwrap(), "investigate");
        // active pipeline set to the draft id
        let reloaded = ws.store.get(&agent_bus_core::ProjectId(project.id.0.clone())).await.unwrap();
        assert_eq!(reloaded.active_pipeline_id.map(|p| p.0), Some("demo".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p app design_session_tests::create_from`
Expected: FAIL — `cannot find function create_project_from_draft_inner`.

- [ ] **Step 3: Implement the inner orchestration + the command**

In `src-tauri/app/src/lib.rs`, add (near the other Design Session commands). Note it reuses `workspace::api::write_project_pipeline_inner` and the existing `workspace_create_project` / `workspace_set_active_pipeline` logic via the store directly:

```rust
use workspace::project::Project;

/// Orchestrate create-from-draft (Decision D5; vet F1). HARD validate the draft's
/// Pipeline; only on Ok create the project + write files (Workspace) + activate.
/// Nothing is written when invalid. Inner fn so it is unit-testable without a
/// Tauri State wrapper.
pub async fn create_project_from_draft_inner(
    ws: &workspace::api::WorkspaceState,
    name: String,
    root: String,
    draft: DraftPipeline,
) -> Result<Project, String> {
    // 1. Serialize + HARD validate (gate before any write).
    let pipeline = draft.to_pipeline();
    pipeline::validate::validate(&pipeline).map_err(|e| e.to_string())?;
    let yaml = pipeline::draft::to_yaml(&pipeline).map_err(|e| e.to_string())?;
    let prompts = pipeline::draft::prompt_files(&draft);
    let yaml_rel = format!("pipelines/{}.yaml", pipeline.id);

    // 2. Create the project row (Workspace; ~ already expanded inside).
    let expanded = workspace::api::expand_tilde(&root, &std::env::var("HOME").unwrap_or_default());
    let project = Project::new(name, std::path::PathBuf::from(expanded), now_unix());
    ws.store.insert(&project).await.map_err(|e| e.to_string())?;

    // 3. Write the YAML + prompt files (Workspace owns the bytes-to-disk).
    workspace::api::write_project_pipeline_inner(
        ws, project.id.0.clone(), yaml_rel, yaml, prompts,
    )
    .await?;

    // 4. Activate.
    ws.store
        .set_active_pipeline(&project.id, Some(&agent_bus_core::PipelineId(pipeline.id.clone())), now_unix())
        .await
        .map_err(|e| e.to_string())?;

    // Return the project with the active pipeline reflected.
    ws.store.get(&project.id).await.map_err(|e| e.to_string())
}

/// OHS command: validate → create + write → activate. The only new-project path.
#[tauri::command(rename_all = "snake_case")]
async fn create_project_from_draft(
    state: tauri::State<'_, WorkspaceState>,
    name: String,
    root: String,
    draft: DraftPipeline,
) -> Result<Project, String> {
    create_project_from_draft_inner(&state, name, root, draft).await
}
```

Add `use workspace::api::WorkspaceState;` if not already imported at the top (it is — `use workspace::{api::WorkspaceState, ...}` is line 4).

- [ ] **Step 4: Register the command**

In the `invoke_handler![ ... ]` list, after `design_session_turn_cmd,` add:

```rust
            create_project_from_draft,
```

- [ ] **Step 5: Run the tests + build**

Run: `cd src-tauri && cargo test -p app design_session_tests`
Expected: PASS — all four design-session tests green (including the two create tests).

Run: `cd src-tauri && cargo build -p app`
Expected: builds.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): create_project_from_draft — hard-validate, create+write, activate (writes nothing if invalid)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 13: Pipeline contract test for `DraftPipeline` IPC shape

**Files:**
- Modify: `src-tauri/pipeline/src/contract_tests.rs`

Lock the `DraftPipeline` / `DraftTeam` / `Slice` JSON shapes the frontend IPC mirrors. (The `TemplateInfo` test is removed in Task 14; this task only ADDS — keep it green before the removal.)

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/contract_tests.rs` (after the existing tests). Add the import at the top:

```rust
use crate::draft::{DraftPipeline, DraftTeam, Slice, SliceTeam, TeamsSlice};
```

Then:

```rust
/// Locks the DraftPipeline key set the wizard IPC mirrors (src/ipc/pipeline.ts).
#[test]
fn draft_pipeline_key_set_matches_ts() {
    let mut d = DraftPipeline::empty();
    d.id = "p".into();
    d.name = "P".into();
    d.teams.push(DraftTeam::new("research", "Research"));
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(
        keys(&v),
        set(&["id", "name", "description", "schema_version", "teams", "forks", "joins", "escalations"]),
    );
}

/// Locks DraftTeam (carries prompt_body inline, not a path).
#[test]
fn draft_team_key_set_matches_ts() {
    let v = serde_json::to_value(DraftTeam::new("research", "Research")).unwrap();
    assert_eq!(keys(&v), set(&["id", "name", "prompt_body", "runner", "scope", "outputs", "workers"]));
}

/// Locks the internally-tagged Slice wire shape (kind discriminator).
#[test]
fn teams_slice_serialises_with_kind_tag() {
    let s = Slice::Teams(TeamsSlice { teams: vec![SliceTeam { id: "a".into(), name: "A".into() }] });
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["kind"], serde_json::Value::String("teams".into()));
    assert!(v["teams"].is_array());
}
```

- [ ] **Step 2: Run the tests**

Run: `cd src-tauri && cargo test -p pipeline contract_tests::draft`
Expected: PASS (the types already exist). If a key set differs, fix the assertion to match the real serialization (the test documents reality).

Run: `cd src-tauri && cargo test -p pipeline contract_tests::teams_slice`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/pipeline/src/contract_tests.rs
git commit -m "test(pipeline): lock DraftPipeline/DraftTeam/Slice IPC key sets

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 14: Remove the bundled template + the template commands (backend)

**Files:**
- Modify: `src-tauri/pipeline/src/api.rs`
- Modify: `src-tauri/pipeline/src/lib.rs`
- Modify: `src-tauri/pipeline/src/store.rs`
- Modify: `src-tauri/pipeline/src/contract_tests.rs`
- Modify: `src-tauri/app/src/lib.rs`
- Delete: `src-tauri/pipeline/src/template.rs`
- Delete: `src-tauri/pipeline/templates/ddd-spec-plan-impl.yaml`

Do this as ONE commit so nothing dangles (D10). The frontend stops importing the removed IPC in Task 15 — but Tauri command registration is backend-only, so removing the registration here + the commands is self-consistent. (The frontend `instantiateTemplate` wrapper still *exists* until Task 15, but it only `invoke`s a now-missing command at runtime; no compile break. Sequence Task 15 right after to remove the dead wrapper.)

- [ ] **Step 1: Remove the two template commands + `TemplateInfo` from `api.rs`**

In `src-tauri/pipeline/src/api.rs`:
- Delete the `use crate::template::bundled_templates;` import.
- Delete the `TemplateInfo` struct, `pipeline_list_templates`, and `pipeline_instantiate_template` functions.
- Delete the `list_templates_returns_the_ddd_template` test in its `mod tests`.

The file should retain: `pipeline_list`, `pipeline_load`, `tools()`, and the `tools_publishes_pipeline_authoring_named_tools` test. The top of the file becomes:

```rust
use crate::model::Pipeline;
use crate::store::PipelineStore;
use agent_bus_core::ToolSpec;
use serde_json::json;
```

(Drop the now-unused `use serde::Serialize;`.)

- [ ] **Step 2: Remove the template module + re-export from `lib.rs`**

In `src-tauri/pipeline/src/lib.rs`, delete the lines `pub mod template;` and `pub use template::*;`.

- [ ] **Step 3: Drop `instantiate_template` + template tests from `store.rs`**

In `src-tauri/pipeline/src/store.rs`:
- Delete `use crate::template::Template;`.
- Delete the `instantiate_template` method.
- In `mod tests`, delete `use crate::template::bundled_templates;`, the `instantiate_then_load_and_list_round_trip` test, and the `save_round_trips_through_yaml` test (it builds from `bundled_templates`). Replace `save_round_trips_through_yaml` with a template-free equivalent so save coverage stays:

```rust
    #[test]
    fn save_round_trips_through_yaml() {
        use crate::model::{Escalation, Pipeline, Routes, RunnerConfig, Scope, Team, Workers};
        use agent_bus_core::{EffortMode, RunnerKind};
        let store = PipelineStore::new(temp_root());
        let p = Pipeline {
            id: "demo".into(), name: "Demo".into(), description: String::new(), schema_version: 1,
            teams: vec![Team {
                id: "research".into(), name: "Research".into(), prompt: "prompts/research.md".into(),
                runner: RunnerConfig { kind: RunnerKind::ClaudeCli, model: "m".into(), effort: EffortMode::Standard, api_key_env: None },
                scope: Scope::default(),
                outputs: Routes { on_approve: Some("needs-human".into()), on_revise: None, on_reject: None },
                workers: Workers::default(),
            }],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![], joins: vec![],
        };
        store.save(&p).unwrap();
        let reloaded = store.load("demo").unwrap();
        assert_eq!(reloaded.name, "Demo");
    }
```

- [ ] **Step 4: Remove the `TemplateInfo` contract test**

In `src-tauri/pipeline/src/contract_tests.rs`:
- Delete `use crate::api::TemplateInfo;`.
- Delete the `template_info_key_set_matches_ts` test.

- [ ] **Step 5: Remove the two command registrations from the root**

In `src-tauri/app/src/lib.rs`, in the `invoke_handler![ ... ]` list, delete the lines:

```rust
            pipeline::api::pipeline_list_templates,
            pipeline::api::pipeline_instantiate_template,
```

- [ ] **Step 6: Delete the template source files**

```bash
git rm src-tauri/pipeline/src/template.rs src-tauri/pipeline/templates/ddd-spec-plan-impl.yaml
```

(If `templates/` becomes empty, also remove the dir.)

- [ ] **Step 7: Build + test the affected crates**

Run: `cd src-tauri && cargo test -p pipeline`
Expected: PASS — no `template::` module, no dangling references; all remaining tests green.

Run: `cd src-tauri && cargo build -p app && cargo test -p app`
Expected: builds + all app tests green (the two removed registrations cause no break; nothing else references them).

- [ ] **Step 8: Commit**

```bash
git add -A src-tauri/pipeline src-tauri/app/src/lib.rs
git commit -m "refactor: drop the bundled template + instantiate_template/pipeline_list_templates (wizard is the only path)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 15: Frontend — remove `listTemplates` / `instantiateTemplate` / `TemplateInfo`; add Design Session IPC

**Files:**
- Modify: `src/ipc/pipeline.ts`
- Modify: `src/ipc/pipeline.test.ts`

- [ ] **Step 1: Remove the template wrappers + add the Design Session types/wrappers**

In `src/ipc/pipeline.ts`:
- Delete the `TemplateInfo` interface, `listTemplates`, and `instantiateTemplate`.
- Add the `DraftPipeline` / `DraftTeam` / `Slice` mirrors + the three Design Session wrappers + the `Step` + `TurnResult` types:

```ts
export interface DraftTeam {
  id: string;
  name: string;
  prompt_body: string;
  runner: RunnerConfig;
  scope: Scope;
  outputs: Routes;
  workers: Workers;
}

export interface DraftPipeline {
  id: string;
  name: string;
  description: string;
  schema_version: number;
  teams: DraftTeam[];
  forks: Fork[];
  joins: Join[];
  escalations: Escalation[];
}

export type Step = "teams" | "prompts" | "wiring";

export interface TurnResult {
  reply_text: string;
  updated_draft: DraftPipeline;
}

export async function kickoffGenerate(sessionId: string, description: string): Promise<DraftPipeline> {
  return await invoke<DraftPipeline>("kickoff_generate_cmd", {
    session_id: sessionId,
    description,
  });
}

export async function designSessionTurn(
  sessionId: string,
  step: Step,
  draft: DraftPipeline,
  userMessage: string,
): Promise<TurnResult> {
  return await invoke<TurnResult>("design_session_turn_cmd", {
    session_id: sessionId,
    step,
    draft,
    user_message: userMessage,
  });
}

export async function createProjectFromDraft(
  name: string,
  root: string,
  draft: DraftPipeline,
): Promise<{ id: string; name: string; root_path: string; active_pipeline_id: string | null; created_at: number; updated_at: number }> {
  return await invoke("create_project_from_draft", { name, root, draft });
}
```

- [ ] **Step 2: Update `pipeline.test.ts`**

In `src/ipc/pipeline.test.ts`:
- Change the import line to drop `listTemplates`: `import { listPipelines, loadPipeline, kickoffGenerate, designSessionTurn } from "./pipeline";`
- Delete the `listTemplates calls pipeline_list_templates` test.
- Add Design Session contract tests:

```ts
  it("kickoffGenerate passes session_id + description", async () => {
    const draft = { id: "p", name: "P", description: "d", schema_version: 2, teams: [], forks: [], joins: [], escalations: [] };
    invokeMock.mockResolvedValueOnce(draft);
    const result = await kickoffGenerate("s1", "build a flow");
    expect(invokeMock).toHaveBeenCalledWith("kickoff_generate_cmd", { session_id: "s1", description: "build a flow" });
    expect(result.id).toBe("p");
  });

  it("designSessionTurn passes step + draft + user_message and returns prose", async () => {
    const draft = { id: "p", name: "P", description: "", schema_version: 2, teams: [], forks: [], joins: [], escalations: [] };
    invokeMock.mockResolvedValueOnce({ reply_text: "ok", updated_draft: draft });
    const out = await designSessionTurn("s1", "teams", draft, "add a team");
    expect(invokeMock).toHaveBeenCalledWith("design_session_turn_cmd", {
      session_id: "s1", step: "teams", draft, user_message: "add a team",
    });
    expect(out.reply_text).toBe("ok");
  });
```

- [ ] **Step 3: Run the tests**

Run: `npm test -- src/ipc/pipeline.test.ts`
Expected: PASS — `listPipelines` / `loadPipeline` + the two new Design Session tests green; no `listTemplates` reference.

- [ ] **Step 4: Commit**

```bash
git add src/ipc/pipeline.ts src/ipc/pipeline.test.ts
git commit -m "feat(web): Design Session IPC (kickoff/turn/create); drop template IPC wrappers

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 16: Frontend — `wizard/draft.ts` ephemeral state helpers

**Files:**
- Create: `src/wizard/draft.ts`
- Create: `src/wizard/draft.test.ts`

Pure helpers for the ephemeral `DesignSession` state (D8): an empty draft, the step list, and manual-edit appliers (rename a team, set a prompt body, edit advanced config) — all client-side mutations that feed the next turn.

- [ ] **Step 1: Write the failing test**

Create `src/wizard/draft.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { emptyDraft, WIZARD_STEPS, renameTeam, setPromptBody, setTeamModel, addTeam, removeTeam } from "./draft";

describe("wizard draft helpers", () => {
  it("emptyDraft has no teams + current schema version", () => {
    const d = emptyDraft();
    expect(d.teams).toEqual([]);
    expect(d.schema_version).toBeGreaterThanOrEqual(2);
  });

  it("WIZARD_STEPS lists the five steps in order", () => {
    expect(WIZARD_STEPS).toEqual(["basics", "teams", "prompts", "wiring", "review"]);
  });

  it("addTeam appends a defaulted team", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    expect(d.teams).toHaveLength(1);
    expect(d.teams[0].id).toBe("research");
    expect(d.teams[0].runner.kind).toBe("claude-cli");
  });

  it("renameTeam changes the display name only", () => {
    const d = renameTeam(addTeam(emptyDraft(), "research", "Research"), "research", "Investigators");
    expect(d.teams[0].name).toBe("Investigators");
    expect(d.teams[0].id).toBe("research");
  });

  it("setPromptBody sets one team's body", () => {
    const d = setPromptBody(addTeam(emptyDraft(), "research", "Research"), "research", "investigate the repo");
    expect(d.teams[0].prompt_body).toBe("investigate the repo");
  });

  it("setTeamModel edits the advanced config", () => {
    const d = setTeamModel(addTeam(emptyDraft(), "research", "Research"), "research", "claude-haiku-4");
    expect(d.teams[0].runner.model).toBe("claude-haiku-4");
  });

  it("removeTeam drops the team", () => {
    const d = removeTeam(addTeam(emptyDraft(), "research", "Research"), "research");
    expect(d.teams).toHaveLength(0);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/draft.test.ts`
Expected: FAIL — module `./draft` not found / functions undefined.

- [ ] **Step 3: Implement the helpers**

Create `src/wizard/draft.ts`:

```ts
import type { DraftPipeline, DraftTeam } from "../ipc/pipeline";

export const WIZARD_STEPS = ["basics", "teams", "prompts", "wiring", "review"] as const;
export type WizardStep = (typeof WIZARD_STEPS)[number];

const SCHEMA_VERSION = 2;

export function emptyDraft(): DraftPipeline {
  return {
    id: "",
    name: "",
    description: "",
    schema_version: SCHEMA_VERSION,
    teams: [],
    forks: [],
    joins: [],
    escalations: [],
  };
}

function defaultTeam(id: string, name: string): DraftTeam {
  return {
    id,
    name,
    prompt_body: "",
    runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" }, api_key_env: null },
    scope: { reads: [], writes: [], tools: [] },
    outputs: {},
    workers: { default: 1, max: 1 },
  };
}

function mapTeams(d: DraftPipeline, f: (t: DraftTeam) => DraftTeam): DraftPipeline {
  return { ...d, teams: d.teams.map(f) };
}

export function addTeam(d: DraftPipeline, id: string, name: string): DraftPipeline {
  if (d.teams.some((t) => t.id === id)) return d;
  return { ...d, teams: [...d.teams, defaultTeam(id, name)] };
}

export function removeTeam(d: DraftPipeline, id: string): DraftPipeline {
  return { ...d, teams: d.teams.filter((t) => t.id !== id) };
}

export function renameTeam(d: DraftPipeline, id: string, name: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, name } : t));
}

export function setPromptBody(d: DraftPipeline, id: string, body: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, prompt_body: body } : t));
}

export function setTeamModel(d: DraftPipeline, id: string, model: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, runner: { ...t.runner, model } } : t));
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/draft.test.ts`
Expected: PASS — 7 tests green.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/draft.ts src/wizard/draft.test.ts
git commit -m "feat(web): wizard draft helpers (empty draft, steps, manual edits)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 17: Frontend — `ChatDraftPanel` (shared chat-left + editable-draft-right, two-way bound)

**Files:**
- Create: `src/wizard/ChatDraftPanel.tsx`
- Create: `src/wizard/ChatDraftPanel.test.tsx`

The reusable component (layout A). Left: a transcript + input that calls `designSessionTurn(sessionId, step, draft, msg)` and applies `updated_draft`. Right: a render-prop slot the parent fills with the step's editable draft view (which mutates `draft` and feeds the next turn).

- [ ] **Step 1: Write the failing test**

Create `src/wizard/ChatDraftPanel.test.tsx`:

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ChatDraftPanel } from "./ChatDraftPanel";
import { emptyDraft } from "./draft";

const turnMock = vi.fn();
vi.mock("../ipc/pipeline", () => ({
  designSessionTurn: (...a: unknown[]) => turnMock(...a),
}));

describe("ChatDraftPanel", () => {
  beforeEach(() => turnMock.mockReset());

  it("sends a turn with the current draft and applies updated_draft", async () => {
    const draft = emptyDraft();
    const updated = { ...draft, teams: [{ id: "research", name: "Research", prompt_body: "", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { default: 1, max: 1 } }] };
    turnMock.mockResolvedValueOnce({ reply_text: "added research", updated_draft: updated });
    const onDraftChange = vi.fn();

    render(
      <ChatDraftPanel
        sessionId="s1"
        step="teams"
        draft={draft}
        onDraftChange={onDraftChange}
        renderDraft={(d) => <div data-testid="draft-view">{d.teams.length} teams</div>}
      />,
    );
    fireEvent.change(screen.getByPlaceholderText(/refine/i), { target: { value: "add research" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(turnMock).toHaveBeenCalledWith("s1", "teams", draft, "add research"));
    await waitFor(() => expect(onDraftChange).toHaveBeenCalledWith(updated));
    expect(await screen.findByText("added research")).toBeInTheDocument();
  });

  it("renders the draft slot via renderDraft", () => {
    render(
      <ChatDraftPanel
        sessionId="s1"
        step="teams"
        draft={emptyDraft()}
        onDraftChange={() => {}}
        renderDraft={(d) => <div data-testid="draft-view">{d.teams.length} teams</div>}
      />,
    );
    expect(screen.getByTestId("draft-view")).toHaveTextContent("0 teams");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/ChatDraftPanel.test.tsx`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the component**

Create `src/wizard/ChatDraftPanel.tsx`:

```tsx
import { type ReactNode, useState } from "react";
import { designSessionTurn, type DraftPipeline, type Step } from "../ipc/pipeline";

interface Msg {
  role: "you" | "claude";
  text: string;
}

interface ChatDraftPanelProps {
  sessionId: string;
  step: Step;
  draft: DraftPipeline;
  onDraftChange: (d: DraftPipeline) => void;
  renderDraft: (d: DraftPipeline, onChange: (d: DraftPipeline) => void) => ReactNode;
}

/// The shared two-way-bound panel (layout A): chat left, live-editable draft
/// right. A chat turn emits a slice that updates the draft + is narrated; manual
/// edits (via renderDraft's onChange) mutate the draft so the next turn sends it.
export function ChatDraftPanel({ sessionId, step, draft, onDraftChange, renderDraft }: ChatDraftPanelProps) {
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  async function send() {
    const v = value.trim();
    if (!v || busy) return;
    setBusy(true);
    setMsgs((m) => [...m, { role: "you", text: v }]);
    setValue("");
    try {
      const out = await designSessionTurn(sessionId, step, draft, v);
      setMsgs((m) => [...m, { role: "claude", text: out.reply_text }]);
      onDraftChange(out.updated_draft);
    } catch (e) {
      setMsgs((m) => [...m, { role: "claude", text: `[error] ${e instanceof Error ? e.message : String(e)}` }]);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "flex", gap: "var(--sp-5)", height: "100%" }}>
      <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 280 }}>
        <div style={{ flex: 1, overflowY: "auto", border: "1px solid var(--border)", borderRadius: "var(--r-sm)", padding: "var(--sp-3)" }}>
          {msgs.map((m, i) => (
            <div key={i} style={{ marginBottom: 8 }}>
              <div style={{ color: "var(--text-3)", fontSize: 11 }}>{m.role}</div>
              <div style={{ color: "var(--text)" }}>{m.text}</div>
            </div>
          ))}
        </div>
        <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
          <input
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") send(); }}
            placeholder="refine this step…"
            style={{ flex: 1, background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" }}
          />
          <button onClick={send} disabled={busy} aria-label="send">Send</button>
        </div>
      </div>
      <div style={{ flex: 1, overflowY: "auto", minWidth: 280 }}>{renderDraft(draft, onDraftChange)}</div>
    </div>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/ChatDraftPanel.test.tsx`
Expected: PASS — 2 tests green.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/ChatDraftPanel.tsx src/wizard/ChatDraftPanel.test.tsx
git commit -m "feat(web): ChatDraftPanel — shared chat-left + editable-draft-right (two-way bound)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 18: Frontend — `TeamsStep` (team cards + advanced panel)

**Files:**
- Create: `src/wizard/TeamsStep.tsx`
- Create: `src/wizard/TeamsStep.test.tsx`

The Step-2 draft view: team cards (add/remove/rename) + a per-card advanced panel (model/effort/tools/scope). It is the `renderDraft` slot for `ChatDraftPanel` on the Teams step.

- [ ] **Step 1: Write the failing test**

Create `src/wizard/TeamsStep.test.tsx`:

```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TeamsStep } from "./TeamsStep";
import { addTeam, emptyDraft } from "./draft";

describe("TeamsStep", () => {
  it("renders a card per team", () => {
    const d = addTeam(addTeam(emptyDraft(), "research", "Research"), "writers", "Writers");
    render(<TeamsStep draft={d} onChange={() => {}} />);
    expect(screen.getByDisplayValue("Research")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Writers")).toBeInTheDocument();
  });

  it("renaming a team calls onChange with the new name", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.change(screen.getByDisplayValue("Research"), { target: { value: "Investigators" } });
    expect(onChange).toHaveBeenCalled();
    const next = onChange.mock.calls[0][0];
    expect(next.teams[0].name).toBe("Investigators");
  });

  it("removing a team calls onChange without it", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /remove research/i }));
    expect(onChange.mock.calls[0][0].teams).toHaveLength(0);
  });

  it("the advanced panel edits the model", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /advanced research/i }));
    fireEvent.change(screen.getByLabelText(/model for research/i), { target: { value: "claude-haiku-4" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.model).toBe("claude-haiku-4");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/TeamsStep.test.tsx`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the component**

Create `src/wizard/TeamsStep.tsx`:

```tsx
import { useState } from "react";
import type { DraftPipeline } from "../ipc/pipeline";
import { removeTeam, renameTeam, setTeamModel } from "./draft";

interface TeamsStepProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
}

/// Step 2 draft view: team cards with rename/remove + a per-team advanced panel
/// (model in v1; effort/tools/scope follow the same controlled-input pattern).
export function TeamsStep({ draft, onChange }: TeamsStepProps) {
  const [openAdvanced, setOpenAdvanced] = useState<string | null>(null);

  return (
    <div>
      {draft.teams.map((t) => (
        <div key={t.id} style={{ border: "1px solid var(--border)", borderRadius: "var(--r-sm)", padding: "var(--sp-3)", marginBottom: "var(--sp-2)" }}>
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input
              aria-label={`name for ${t.id}`}
              value={t.name}
              onChange={(e) => onChange(renameTeam(draft, t.id, e.target.value))}
              style={{ flex: 1, background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" }}
            />
            <span style={{ color: "var(--text-3)", fontSize: 11 }}>{t.id}</span>
            <button aria-label={`advanced ${t.id}`} onClick={() => setOpenAdvanced((o) => (o === t.id ? null : t.id))}>⚙</button>
            <button aria-label={`remove ${t.id}`} onClick={() => onChange(removeTeam(draft, t.id))}>✕</button>
          </div>
          {openAdvanced === t.id && (
            <div style={{ marginTop: 6 }}>
              <label style={{ fontSize: 11, color: "var(--text-3)" }}>
                model
                <input
                  aria-label={`model for ${t.id}`}
                  value={t.runner.model}
                  onChange={(e) => onChange(setTeamModel(draft, t.id, e.target.value))}
                  style={{ width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" }}
                />
              </label>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/TeamsStep.test.tsx`
Expected: PASS — 4 tests green.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/TeamsStep.tsx src/wizard/TeamsStep.test.tsx
git commit -m "feat(web): TeamsStep — team cards (rename/remove) + advanced model panel

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 19: Frontend — `PromptsStep` (per-team prompt editor)

**Files:**
- Create: `src/wizard/PromptsStep.tsx`
- Create: `src/wizard/PromptsStep.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/wizard/PromptsStep.test.tsx`:

```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { PromptsStep } from "./PromptsStep";
import { addTeam, emptyDraft, setPromptBody } from "./draft";

describe("PromptsStep", () => {
  it("renders an editor per team showing its body", () => {
    let d = addTeam(emptyDraft(), "research", "Research");
    d = setPromptBody(d, "research", "investigate the repo");
    render(<PromptsStep draft={d} onChange={() => {}} />);
    expect(screen.getByDisplayValue("investigate the repo")).toBeInTheDocument();
  });

  it("editing a body calls onChange with the new text", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<PromptsStep draft={d} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/prompt for research/i), { target: { value: "new responsibility" } });
    expect(onChange.mock.calls[0][0].teams[0].prompt_body).toBe("new responsibility");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/PromptsStep.test.tsx`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the component**

Create `src/wizard/PromptsStep.tsx`:

```tsx
import type { DraftPipeline } from "../ipc/pipeline";
import { setPromptBody } from "./draft";

interface PromptsStepProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
}

/// Step 3 draft view: each team's responsibility prompt text, editable.
export function PromptsStep({ draft, onChange }: PromptsStepProps) {
  return (
    <div>
      {draft.teams.map((t) => (
        <div key={t.id} style={{ marginBottom: "var(--sp-3)" }}>
          <div style={{ color: "var(--text-2)", fontSize: 12, marginBottom: 4 }}>{t.name} · {t.id}</div>
          <textarea
            aria-label={`prompt for ${t.id}`}
            value={t.prompt_body}
            onChange={(e) => onChange(setPromptBody(draft, t.id, e.target.value))}
            rows={5}
            style={{ width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit", fontSize: 12 }}
          />
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/PromptsStep.test.tsx`
Expected: PASS — 2 tests green.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/PromptsStep.tsx src/wizard/PromptsStep.test.tsx
git commit -m "feat(web): PromptsStep — per-team responsibility prompt editor

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 20: Frontend — `WiringStep` (fork/join via `PipelineView`)

**Files:**
- Create: `src/wizard/WiringStep.tsx`
- Create: `src/wizard/WiringStep.test.tsx`

Renders the draft's flow through the existing `PipelineView` viewer (sub-project 2's fork/join render). The draft is adapted to a `Pipeline`-shaped object (prompt body → a placeholder path; gates empty) so the viewer renders Teams / Forks / Joins. v1 wiring edits come from chat; the viewer is read-only here.

- [ ] **Step 1: Write the failing test**

Create `src/wizard/WiringStep.test.tsx`:

```tsx
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { WiringStep } from "./WiringStep";
import { addTeam, emptyDraft } from "./draft";

describe("WiringStep", () => {
  it("renders teams + fork/join sections via PipelineView", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = { ...d, forks: [{ id: "fork-1", lanes: ["a", "b"] }], joins: [{ id: "join-1", waits_for: ["a", "b"], downstream: "needs-human" }] };
    render(<WiringStep draft={d} />);
    expect(screen.getByText(/Forks \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/Joins \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/fork-1/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/WiringStep.test.tsx`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the component + the draft→Pipeline adapter**

Create `src/wizard/WiringStep.tsx`:

```tsx
import type { DraftPipeline, Pipeline } from "../ipc/pipeline";
import { PipelineView } from "../components/PipelineView";

/// Adapt a DraftPipeline to the Pipeline shape the read-only viewer expects:
/// inline prompt bodies become placeholder paths; gates are empty (D1).
export function draftToPipeline(d: DraftPipeline): Pipeline {
  return {
    id: d.id || "(draft)",
    name: d.name || "(unnamed)",
    description: d.description,
    schema_version: d.schema_version,
    teams: d.teams.map((t) => ({ ...t, prompt: `prompts/${t.id}.md` })),
    gates: [],
    escalations: d.escalations,
    forks: d.forks,
    joins: d.joins,
  };
}

interface WiringStepProps {
  draft: DraftPipeline;
}

/// Step 4 draft view: the fork/join flow rendered through PipelineView.
export function WiringStep({ draft }: WiringStepProps) {
  return <PipelineView pipeline={draftToPipeline(draft)} />;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/WiringStep.test.tsx`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/WiringStep.tsx src/wizard/WiringStep.test.tsx
git commit -m "feat(web): WiringStep — fork/join flow via PipelineView (draft adapter)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 21: Frontend — `ReviewStep` (assembled pipeline + prompt files + Create)

**Files:**
- Create: `src/wizard/ReviewStep.tsx`
- Create: `src/wizard/ReviewStep.test.tsx`

Renders the assembled pipeline (via `PipelineView`/the adapter) + every prompt file, and a Create button that calls `createProjectFromDraft(name, root, draft)`. On error (hard-validation failure from the backend) it shows the message; on success it calls `onCreated`.

- [ ] **Step 1: Write the failing test**

Create `src/wizard/ReviewStep.test.tsx`:

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ReviewStep } from "./ReviewStep";
import { addTeam, emptyDraft, setPromptBody } from "./draft";

const createMock = vi.fn();
vi.mock("../ipc/pipeline", () => ({
  createProjectFromDraft: (...a: unknown[]) => createMock(...a),
}));

describe("ReviewStep", () => {
  beforeEach(() => createMock.mockReset());

  function draft() {
    let d = addTeam(emptyDraft(), "research", "Research");
    d = setPromptBody(d, "research", "investigate");
    return { ...d, name: "Demo" };
  }

  it("renders the assembled pipeline + the prompt files", () => {
    render(<ReviewStep basics={{ name: "Demo", root: "/p", description: "" }} draft={draft()} onCreated={() => {}} />);
    expect(screen.getByText("investigate")).toBeInTheDocument();
    expect(screen.getByText(/prompts\/research\.md/)).toBeInTheDocument();
  });

  it("Create calls createProjectFromDraft and onCreated on success", async () => {
    const created = { id: "proj-x", name: "Demo", root_path: "/p", active_pipeline_id: "demo", created_at: 0, updated_at: 0 };
    createMock.mockResolvedValueOnce(created);
    const onCreated = vi.fn();
    render(<ReviewStep basics={{ name: "Demo", root: "/p", description: "" }} draft={draft()} onCreated={onCreated} />);
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    await waitFor(() => expect(createMock).toHaveBeenCalledWith("Demo", "/p", expect.objectContaining({ name: "Demo" })));
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(created));
  });

  it("shows the backend error when create fails (writes nothing)", async () => {
    createMock.mockRejectedValueOnce(new Error("team 'research' is unreachable"));
    render(<ReviewStep basics={{ name: "Demo", root: "/p", description: "" }} draft={draft()} onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    expect(await screen.findByText(/unreachable/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/ReviewStep.test.tsx`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the component**

Create `src/wizard/ReviewStep.tsx`:

```tsx
import { useState } from "react";
import { createProjectFromDraft, type DraftPipeline } from "../ipc/pipeline";
import type { Project } from "../ipc/workspace";
import { PipelineView } from "../components/PipelineView";
import { draftToPipeline } from "./WiringStep";

interface ReviewStepProps {
  basics: { name: string; root: string; description: string };
  draft: DraftPipeline;
  onCreated: (p: Project) => void;
}

/// Step 5: review the assembled pipeline + every prompt file, then Create.
/// Create calls create_project_from_draft (hard-validate → write → activate);
/// a hard-validation failure comes back as an error and nothing is written.
export function ReviewStep({ basics, draft, onCreated }: ReviewStepProps) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const named = { ...draft, name: basics.name, description: basics.description };

  async function create() {
    setBusy(true);
    setError(null);
    try {
      const project = await createProjectFromDraft(basics.name, basics.root, named);
      onCreated(project as Project);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <PipelineView pipeline={draftToPipeline(named)} />
      <h3 style={{ fontSize: 12, color: "var(--text-2)" }}>Prompt files</h3>
      {named.teams.map((t) => (
        <div key={t.id} style={{ marginBottom: "var(--sp-2)" }}>
          <div style={{ color: "var(--text-3)", fontSize: 11 }}>prompts/{t.id}.md</div>
          <pre style={{ whiteSpace: "pre-wrap", background: "var(--bg-2)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", color: "var(--text)" }}>{t.prompt_body}</pre>
        </div>
      ))}
      {error && <div style={{ color: "var(--danger)", fontSize: 12 }}>{error}</div>}
      <button onClick={create} disabled={busy}>Create project</button>
    </div>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/ReviewStep.test.tsx`
Expected: PASS — 3 tests green.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/ReviewStep.tsx src/wizard/ReviewStep.test.tsx
git commit -m "feat(web): ReviewStep — assembled pipeline + prompt files + create_project_from_draft

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 22: Frontend — `NewProjectWizard` shell (5-step navigation + ephemeral state)

**Files:**
- Create: `src/wizard/NewProjectWizard.tsx`
- Create: `src/wizard/NewProjectWizard.test.tsx`

The shell. Holds the ephemeral `DesignSession` state (D8): step, basics, draft, a per-open `sessionId`. Step 1 has name/root/description + a Generate button (`kickoffGenerate`). Steps 2–4 render `ChatDraftPanel` with the step's slot. Step 5 is `ReviewStep`. Next/Back navigate.

- [ ] **Step 1: Write the failing test**

Create `src/wizard/NewProjectWizard.test.tsx`:

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { NewProjectWizard } from "./NewProjectWizard";
import { emptyDraft } from "./draft";

const kickoffMock = vi.fn();
vi.mock("../ipc/pipeline", () => ({
  kickoffGenerate: (...a: unknown[]) => kickoffMock(...a),
  // ChatDraftPanel / ReviewStep import these too; stub them so the shell test is isolated.
  designSessionTurn: vi.fn(),
  createProjectFromDraft: vi.fn(),
}));

describe("NewProjectWizard", () => {
  beforeEach(() => kickoffMock.mockReset());

  it("does not render when open=false", () => {
    render(<NewProjectWizard open={false} onClose={() => {}} onCreated={() => {}} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("starts on Basics and generates a draft, advancing to Teams", async () => {
    const draft = { ...emptyDraft(), teams: [{ id: "research", name: "Research", prompt_body: "", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { default: 1, max: 1 } }] };
    kickoffMock.mockResolvedValueOnce(draft);
    render(<NewProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
    fireEvent.change(screen.getByLabelText(/root path/i), { target: { value: "/p" } });
    fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "a research flow" } });
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));
    await waitFor(() => expect(kickoffMock).toHaveBeenCalled());
    // advanced to the Teams step (step heading visible)
    expect(await screen.findByText(/teams/i)).toBeInTheDocument();
  });

  it("Cancel calls onClose", () => {
    const onClose = vi.fn();
    render(<NewProjectWizard open={true} onClose={onClose} onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onClose).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/NewProjectWizard.test.tsx`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the shell**

Create `src/wizard/NewProjectWizard.tsx`:

```tsx
import { useMemo, useState } from "react";
import { kickoffGenerate, type DraftPipeline, type Step } from "../ipc/pipeline";
import type { Project } from "../ipc/workspace";
import { emptyDraft, WIZARD_STEPS, type WizardStep } from "./draft";
import { ChatDraftPanel } from "./ChatDraftPanel";
import { TeamsStep } from "./TeamsStep";
import { PromptsStep } from "./PromptsStep";
import { WiringStep } from "./WiringStep";
import { ReviewStep } from "./ReviewStep";

interface NewProjectWizardProps {
  open: boolean;
  onClose: () => void;
  onCreated: (p: Project) => void;
}

const STEP_TO_API: Record<"teams" | "prompts" | "wiring", Step> = {
  teams: "teams",
  prompts: "prompts",
  wiring: "wiring",
};

/// The 5-step new-project wizard (the only new-project path). Ephemeral
/// DesignSession state (D8): step, basics, draft, a per-open sessionId. Closing
/// discards everything.
export function NewProjectWizard({ open, onClose, onCreated }: NewProjectWizardProps) {
  const [step, setStep] = useState<WizardStep>("basics");
  const [name, setName] = useState("");
  const [root, setRoot] = useState("");
  const [description, setDescription] = useState("");
  const [draft, setDraft] = useState<DraftPipeline>(emptyDraft());
  const [busy, setBusy] = useState(false);
  // a stable per-open dialogue session id (D8).
  const sessionId = useMemo(() => `wiz-${Math.random().toString(36).slice(2)}`, [open]);

  if (!open) return null;

  const idx = WIZARD_STEPS.indexOf(step);
  const go = (next: WizardStep) => setStep(next);

  async function generate() {
    setBusy(true);
    try {
      const d = await kickoffGenerate(sessionId, description);
      setDraft({ ...d, name, description });
      go("teams");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div role="dialog" aria-modal="true" style={overlay} onClick={onClose}>
      <div onClick={(e) => e.stopPropagation()} style={panel}>
        <div style={{ display: "flex", gap: 8, marginBottom: "var(--sp-4)" }}>
          {WIZARD_STEPS.map((s) => (
            <span key={s} style={{ color: s === step ? "var(--text)" : "var(--text-3)", fontSize: 12, textTransform: "capitalize" }}>{s}</span>
          ))}
        </div>

        {step === "basics" && (
          <div>
            <label style={lbl}>Project name<input aria-label="Project name" value={name} onChange={(e) => setName(e.target.value)} style={inp} /></label>
            <label style={lbl}>Root path<input aria-label="Root path" value={root} onChange={(e) => setRoot(e.target.value)} placeholder="~/projects/example" style={inp} /></label>
            <label style={lbl}>Describe what you're building<textarea aria-label="Describe what you're building" value={description} onChange={(e) => setDescription(e.target.value)} rows={4} style={inp} /></label>
            <button onClick={generate} disabled={busy || !name.trim() || !root.trim() || !description.trim()}>Generate</button>
          </div>
        )}

        {(step === "teams" || step === "prompts" || step === "wiring") && (
          <div style={{ height: 420 }}>
            <h3 style={{ fontSize: 13, textTransform: "capitalize" }}>{step}</h3>
            {step === "wiring" ? (
              // Wiring's draft view is the read-only viewer; chat still drives edits.
              <ChatDraftPanel
                sessionId={sessionId}
                step={STEP_TO_API[step]}
                draft={draft}
                onDraftChange={setDraft}
                renderDraft={(d) => <WiringStep draft={d} />}
              />
            ) : (
              <ChatDraftPanel
                sessionId={sessionId}
                step={STEP_TO_API[step]}
                draft={draft}
                onDraftChange={setDraft}
                renderDraft={(d, onChange) => (step === "teams" ? <TeamsStep draft={d} onChange={onChange} /> : <PromptsStep draft={d} onChange={onChange} />)}
              />
            )}
          </div>
        )}

        {step === "review" && (
          <ReviewStep basics={{ name, root, description }} draft={draft} onCreated={onCreated} />
        )}

        <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--sp-5)" }}>
          <button onClick={onClose}>Cancel</button>
          <div style={{ display: "flex", gap: 8 }}>
            {idx > 0 && step !== "review" && <button onClick={() => go(WIZARD_STEPS[idx - 1])}>Back</button>}
            {step !== "basics" && step !== "review" && <button onClick={() => go(WIZARD_STEPS[idx + 1])}>Next</button>}
          </div>
        </div>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = { position: "fixed", inset: 0, background: "rgba(0,0,0,0.5)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 100 };
const panel: React.CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", borderRadius: "var(--r-md)", padding: "var(--sp-7)", minWidth: 720, maxWidth: 900 };
const lbl: React.CSSProperties = { display: "block", fontSize: 11, color: "var(--text-3)", marginBottom: "var(--sp-3)" };
const inp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit", fontSize: 12 };
```

Add `import type React from "react";` at the top if the style typings require it (the project uses `React.CSSProperties` elsewhere via `import type React`).

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/NewProjectWizard.test.tsx`
Expected: PASS — 3 tests green.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/NewProjectWizard.tsx src/wizard/NewProjectWizard.test.tsx
git commit -m "feat(web): NewProjectWizard 5-step shell (ephemeral Design Session state)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 23: Frontend — wire `NewProjectWizard` into `App.tsx`; rip out the auto-instantiate effect

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/App.test.tsx`

- [ ] **Step 1: Swap the wizard import + element**

In `src/App.tsx`:
- Change `import { ProjectWizard } from "./components/ProjectWizard";` to `import { NewProjectWizard } from "./wizard/NewProjectWizard";`.
- Change the import on line 17 to drop the template wrappers: `import { loadPipeline, type Pipeline } from "./ipc/pipeline";`.
- Replace the `<ProjectWizard ... />` element at the bottom with:

```tsx
      <NewProjectWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreated={onCreated}
      />
```

- [ ] **Step 2: Rip out the auto-instantiate effect**

In `src/App.tsx`, replace the `useEffect` that calls `listPipelines` then `instantiateTemplate` (the block starting `// Load (or first-time instantiate) the active project's pipeline`) with a load-only effect (no template fallback — the wizard always writes a pipeline; a project with none simply shows the empty viewer):

```tsx
  // Load the active project's pipeline whenever the active project changes. The
  // wizard is the only new-project path now (it always writes a pipeline), so a
  // project with no pipeline file just shows the empty viewer.
  useEffect(() => {
    let cancelled = false;
    if (!activeProject) {
      setPipeline(null);
      return;
    }
    const root = activeProject.root_path;
    (async () => {
      try {
        const ids = await listPipelines(root);
        const target = ids[0] ?? null;
        const loaded = target ? await loadPipeline(root, target) : null;
        if (!cancelled) setPipeline(loaded);
      } catch {
        if (!cancelled) setPipeline(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [activeProject]);
```

Keep `listPipelines` in the import (it is still used): `import { listPipelines, loadPipeline, type Pipeline } from "./ipc/pipeline";`.

- [ ] **Step 3: Update `App.test.tsx`**

In `src/App.test.tsx`:
- Change the `./ipc/pipeline` mock to drop `instantiateTemplate` / `listTemplates` and keep `loadPipeline` + `listPipelines`:

```ts
const loadPipelineMock = vi.fn();
vi.mock("./ipc/pipeline", () => ({
  loadPipeline: (...a: unknown[]) => loadPipelineMock(...a),
  listPipelines: vi.fn().mockResolvedValue(["pl"]),
}));
```

- In `beforeEach`, drop the `instantiateMock` lines and seed `listPipelines` to return `["pl"]` so `loadPipeline` is hit (the pipeline literal already has `forks: [], joins: []` — leave it; the existing `pl` literal lacks `forks`/`joins`, so add them to keep the viewer adapter happy):

```ts
    const pl = {
      id: "pl", name: "PL", description: "", schema_version: 2,
      teams: [{ id: "plan-writers", name: "Plan Writers" }],
      gates: [{ id: "gate-2-plan", label: "Gate 2", downstream: "implementers" }],
      escalations: [], forks: [], joins: [],
    };
    loadPipelineMock.mockReset().mockResolvedValue(pl);
```

- The wizard is not opened in these tests, so no `NewProjectWizard` mock is required (the board/terminal tests don't touch it). If `NewProjectWizard`'s imports (which import the Design Session IPC) break the mock, also stub the extra exports the wizard tree imports by adding them to the `./ipc/pipeline` mock: `kickoffGenerate: vi.fn(), designSessionTurn: vi.fn(), createProjectFromDraft: vi.fn(),`.

- [ ] **Step 4: Run the App tests**

Run: `npm test -- src/App.test.tsx`
Expected: PASS — the board renders the gated card + the terminal docks; no `instantiateTemplate` reference.

- [ ] **Step 5: Commit**

```bash
git add src/App.tsx src/App.test.tsx
git commit -m "feat(web): NewProjectWizard is the only new-project path; drop auto-instantiate

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 24: Remove the old `ProjectWizard` component + its test

**Files:**
- Delete: `src/components/ProjectWizard.tsx`
- Delete: `src/components/ProjectWizard.test.tsx`

- [ ] **Step 1: Confirm nothing imports it**

Run: `grep -rn "ProjectWizard" src --include=*.tsx --include=*.ts | grep -v "src/wizard/" | grep -v "NewProjectWizard"`
Expected: no matches (App.tsx now imports `NewProjectWizard`). If any match remains, fix that import first.

- [ ] **Step 2: Delete the files**

```bash
git rm src/components/ProjectWizard.tsx src/components/ProjectWizard.test.tsx
```

- [ ] **Step 3: Run the full frontend suite**

Run: `npm test`
Expected: PASS — every frontend test green; no dangling `ProjectWizard` import.

- [ ] **Step 4: Commit**

```bash
git add -A src/components
git commit -m "refactor(web): remove the single-screen ProjectWizard (replaced by NewProjectWizard)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 25: DOMAIN.md updates + full-suite green gate

**Files:**
- Modify: `DOMAIN.md`

- [ ] **Step 1: Update Pipeline Authoring ubiquitous language**

In `DOMAIN.md`, under `### Pipeline Authoring`, replace the existing **Design Session** stub line (currently ends "*(Full definition lands with sub-project 3 — the wizard.)*") with the full definition, and add **DraftPipeline**:

```markdown
- **Design Session** — an ephemeral, AI-assisted authoring dialogue that produces a pipeline, conducted over the LLM Chat ACL (one `dialogue_id` per wizard step: `<session>:<step>`). Distinct from the terminal's `Conversation` aggregate — it has no persistence and lives in frontend state + an in-memory chat session for the duration of the wizard. Drives the 5-step new-project wizard (basics → teams → responsibilities → wiring → review).
- **DraftPipeline** — an in-progress, not-yet-valid pipeline the wizard edits; distinct from the validated `Pipeline` aggregate. Best-effort validation surfaces issues live during editing; only a `DraftPipeline` that passes **hard** validation (`validate.rs`) at create becomes a `Pipeline`. Prompt text is held inline; Pipeline Authoring serializes it (YAML + per-team prompt files) and Workspace writes it.
```

- [ ] **Step 2: Update Workspace ubiquitous language**

Under `### Workspace`, **remove** the `**Template**` line and add a note on the write surface:

```markdown
- **Project write surface** — `write_project_pipeline` writes a project's pipeline YAML + per-team prompt files (`prompts/<team>.md`) under the (already `~`-expanded) project root, path-scoped with the `resolve_under_root` escape guard — the counterpart to `read_artifact`. (The bundled-**Template** instantiation path was dropped in sub-project 3; the wizard writes YAML directly. Templates may return as wizard seeds in v1.1.)
```

- [ ] **Step 3: Run the FULL backend suite**

Run: `cd src-tauri && cargo test`
Expected: PASS — every crate green (pipeline incl. draft/design_session, workspace incl. write, app incl. design_session_tests, and all others untouched).

- [ ] **Step 4: Run the FULL frontend suite**

Run: `npm test`
Expected: PASS — every test green (wizard/* , ipc/* , App, components).

- [ ] **Step 5: Verify the dependency graph is still acyclic**

Run: `cd src-tauri && cargo tree -p pipeline -i llm_chat`
Expected: `pipeline` depends on `llm_chat`; `llm_chat` does not depend on `pipeline` (no cycle).

- [ ] **Step 6: Commit**

```bash
git add DOMAIN.md
git commit -m "docs(domain): Design Session (full) + DraftPipeline; Workspace write surface; remove Template

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Roadmap note

After this lands, the brainstorming new-project wizard is **feature-complete** across the three sub-projects (`llm_chat` foundation, parallel flow / fork+join, this wizard). Deferred to v1.1 (noted in `## Decisions`): live best-effort issue display inside steps 2–4 (the backend already computes `best_effort_validate`); a per-team advanced panel beyond `model` (effort preset select, tools/scope comma editors — same controlled-input pattern as Task 18); gate authoring in the draft (the wizard produces team/fork/join graphs in v1; `to_pipeline()` already emits an empty `gates`); templates returning as wizard *seeds* (the bundled-template path was dropped, not the concept); and incremental token streaming for the Design Session chat (v1 awaits the full `--print` reply, inherited from `llm_chat`).

---

## Self-review (run against the spec)

- **Spec coverage:** DraftPipeline type (T2), slice-merge (T3), best-effort + hard validation (T4, T12), `kickoff_generate`/`design_session_turn` (T7, T11), `create_project_from_draft` orchestration (T12), `write_project_pipeline` (T9), 5-step wizard (T22) with chat+draft (T17), teams+advanced (T18), responsibilities→prompts (T19), wiring via PipelineView (T20), review+create (T21), removals (T14, T15, T23, T24), DOMAIN.md (T25). Structured-emit contract is in the step system prompts (T7) + extraction (T6); the backend is the trust boundary (T6/T7). Ephemeral session (T22, D8). All chat tests use `FakeChatRunner` — no live `claude`.
- **Type consistency:** `DraftPipeline`/`DraftTeam`/`Slice`/`TurnResult`/`Step` are defined once (Rust T2/T3/T7) and mirrored once (TS T15); `to_pipeline`/`to_yaml`/`prompt_files`/`best_effort_validate`/`apply_slice` names are used identically across tasks; `write_project_pipeline_inner` (T9) and `create_project_from_draft_inner` (T12) are the testable inners the commands wrap.
- **No dangling template refs:** removal (T14) is one commit touching api.rs/lib.rs/store.rs/contract_tests.rs/app + deleting template files, sequenced after the new path + frontend IPC removal; `load_active` never referenced templates.
