# Spec — Brainstorming new-project wizard (sub-project 3, the headline)

*Design doc. Brainstormed + DDD-vetted 2026-06-23. Third of three sub-projects. Depends on sub-project 1 (`llm_chat`) and sub-project 2 (parallel flow / fork+join).*

## Why this exists

Creating a project today silently instantiates the fixed DDD template and writes **no prompt files** (so workers run with empty prompts). This replaces that with a guided, AI-assisted wizard: the user describes what they're building, and a per-step interactive design session produces a tailored pipeline — the required teams, each team's responsibility as a prompt, and how they flow — then writes it (YAML + prompt files) and activates it.

## Decisions (from brainstorm 2026-06-23)

- **5 steps:** (1) Basics & goal (+ one-shot description) → (2) Teams → (3) Responsibilities→prompts → (4) Wiring → (5) Review & create.
- **One-shot kickoff → pre-populate:** the description generates a full draft once; steps 2–4 open pre-filled and are refined by chat.
- **Per-step multi-turn chat** (over `llm_chat`, sub-project 1) drives a **live, editable draft panel** beside it (layout A: chat left, draft right). **Two-way bound:** chat emits a structured slice that updates the draft and is narrated; manual edits to the draft are fed into the next chat turn so the agent interprets them.
- **Team technical config:** smart defaults inferred at kickoff + an optional per-team **advanced panel** (model/effort/tools/scope) — not a required step.
- **Wizard-only:** the fixed-template auto-instantiation is removed; every new project is designed here and always gets prompt files.
- **Ephemeral:** the Design Session (steps, draft, chat transcripts) lives in frontend state for the session; closing mid-wizard discards it. No new persistence.

## DDD decisions (from the council vet 2026-06-23)

- **Design Session = a Pipeline Authoring concept** (settled in the sub-project-1 vet): an ephemeral authoring dialogue, **distinct** from Conversational Control's `Conversation` aggregate. It consumes `llm_chat` (kernel-only ACL) → acyclic.
- **`DraftPipeline` is distinct from the validated `Pipeline` aggregate** (vet F2). A half-built draft can't satisfy `validate.rs`, so the wizard manipulates a `DraftPipeline` (the in-progress structure). Best-effort validation runs during editing to surface errors live; **hard** validation runs at create and must pass before any file is written. Only a valid `DraftPipeline` becomes a `Pipeline`.
- **Workspace writes files; Pipeline Authoring serializes** (vet F1). Workspace owns the project filesystem layout (`prompts/`, `pipelines/`, and `read_artifact`). It gains an OHS command to write the pipeline YAML + per-team prompt files. Pipeline Authoring validates the `DraftPipeline` and serializes it to content. `create_project_from_draft` orchestrates: validate → `Workspace.create_project` + write files → activate. Pipeline Authoring never touches the filesystem directly (no leaky boundary).

## Frontend (`web-app`)

`NewProjectWizard` replaces `ProjectWizard`. Ephemeral `DesignSession` React state: `{ step, draft: DraftPipeline, basics: {name, root, description}, perStepChat: Record<step, Turn[]> }`.

- **Step 1 — Basics & goal:** name, root path, one-shot description textarea. "Generate" → `kickoff_generate(description)` → `DraftPipeline`; advance.
- **Steps 2–4:** the shared **chat + live-draft** component. Each step scopes the chat to its slice (`dialogue_id = "<wizard-session>:<step>"`) and renders the relevant draft view:
  - Teams → team cards (add/remove/rename; advanced panel per card).
  - Responsibilities → each team's prompt text (editable).
  - Wiring → the fork/join flow (sub-project 2's viewer), editable.
- **Step 5 — Review & create:** renders the assembled pipeline + all prompt files; "Create" → `create_project_from_draft(name, root, draft)`; on success, closes and activates.

Two-way sync detail: the `DraftPipeline` is the single source of truth in state. A chat turn sends `(step, draft, user_message)`; the reply carries `updated_draft` (applied) + prose (shown). A manual edit mutates `draft` locally; the *next* chat turn sends the mutated draft, so the agent's narration reflects it.

## Backend OHS (thin)

Pipeline Authoring (`pipeline` crate) — depends on `llm_chat`:
- `kickoff_generate(description: String) -> DraftPipeline` — one-shot over `llm_chat` with a generation system prompt; parse + best-effort validate.
- `design_session_turn(step, draft: DraftPipeline, user_message) -> { reply_text, updated_draft }` — one chat turn over `llm_chat` with a **step-specific** system prompt instructing the agent to return prose **plus a fenced JSON slice** for that step (teams list / a team's prompt / the fork-join wiring). Backend extracts the JSON, applies it to the draft, best-effort validates, returns prose + updated draft. Invalid JSON → draft unchanged, prose surfaces the issue.
- `create_project_from_draft(name, root, draft) -> Project` — **hard** validate; orchestrate Workspace create + write; activate. Errors if invalid (nothing written).

Workspace (`workspace` crate) — new OHS:
- `write_project_pipeline(root, pipeline_yaml, prompts: Vec<(path, content)>)` — writes `pipelines/<id>.yaml` + `prompts/<team>.md` under the project root (it owns the layout). Path-scoped with the same escape guard as `read_artifact`.

**Structured-emit contract:** the agent emits a fenced ```json block holding only the current step's slice, against a documented mini-schema per step (kept in the system prompt). The backend is the trust boundary — it parses and validates; the model's prose is never trusted to mutate state directly.

## Removed

- App.tsx's `listPipelines`-then-`instantiateTemplate` auto-instantiation is removed; the wizard is the only new-project path. The bundled template + `instantiate_template`/`pipeline_list_templates` commands are dropped (the wizard writes YAML directly). Existing projects with a pipeline are unaffected.

## Testing

- **Pure:** draft→YAML serialization; slice-merge (applying a teams/prompt/wiring JSON slice to a `DraftPipeline`); best-effort vs hard validation paths.
- **Backend (FakeChatRunner, no live `claude`):** `kickoff_generate` produces a valid draft from a canned reply; `design_session_turn` applies a slice + returns prose; invalid-JSON turn leaves the draft unchanged; `create_project_from_draft` rejects an invalid draft (writes nothing) and, on a valid draft, calls Workspace write with the expected YAML + prompt set.
- **Workspace:** `write_project_pipeline` writes the files under the root and rejects path escapes.
- **Frontend:** wizard step navigation; two-way sync (chat updates draft; manual edit persists into the next turn's request); review renders the assembled pipeline; contract tests for `DraftPipeline` IPC shape.

## DOMAIN.md / config updates (apply when this lands)

- Pipeline Authoring ubiquitous language: **Design Session** ("an ephemeral, AI-assisted authoring dialogue that produces a pipeline; distinct from the terminal's `Conversation`"), **DraftPipeline** ("an in-progress, not-yet-valid pipeline the wizard edits; becomes a `Pipeline` only when it passes hard validation").
- Workspace ubiquitous language: note the new write surface (`write_project_pipeline`) alongside `read_artifact`.
- Remove **Template** from Workspace's language (the bundled-template path is dropped), or mark it v1.1-future if templates return as wizard seeds later.

## Roadmap — feature complete

With sub-projects 1 (`llm_chat`), 2 (parallel flow), and 3 (this) specced, the brainstorming new-project wizard is fully designed. Next: implementation plans (`superpowers:writing-plans`) per sub-project, in dependency order — 1 and 2 (independent) before 3.
