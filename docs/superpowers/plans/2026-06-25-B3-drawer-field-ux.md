# B3 — Node-drawer field UX (G7, G8, G6)

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Design source: `docs/v1.1-backlog.md` items **G7/G8/G6**. Mostly node-drawer + a small backend surface (`list_dir`, model-unavailable classification, a probe). Layers on B1/B2.

**Goal:** A themed in-app file/folder picker (replacing the native OS dialog), informational tooltips on unclear authoring fields, and a model **selector** with availability handling.

**Architecture:** New reusable `FileTreePicker` (frontend) over a new `list_dir` OHS command (seals fs listing behind a command, like the other seams). The `NodeDrawer` Scope fields use it (multi-select within the repo); root/target reuse it (single-folder). A curated model list drives a selector; the runner classifies a "model unavailable" error distinctly; a `test_model` probe command does a 1-token call.

**Tech Stack:** React/TS, Rust (sqlx not needed; std fs behind the command), the existing seam discipline.

---

## Tasks

- [ ] **Task 1 — `list_dir` OHS command (backend).** A workspace command `list_dir(path) -> Vec<DirEntry { name, path, is_dir }>` (tilde-expanded; sorted dirs-first; tolerant of unreadable dirs → error surfaced, not a panic). Seal std::fs behind the command (no fs idiom crosses beyond the `DirEntry` DTO + wire-contract test). Register in `generate_handler!`. Tests (tempdir). Commit.
- [ ] **Task 2 — `FileTreePicker` component (G7).** A themed in-app tree/browser rooted at a base path, lazily expanding via `list_dir`: single-select (folder) mode for root/target; multi-select (files + folders) mode for scope. Returns absolute or repo-relative paths as appropriate. Replace the native `pickFolder`/`FolderPickerField` Browse action with this (keep manual text entry as fallback). Pure selection logic unit-tested; component tests with a mocked `list_dir`. Commit.
- [ ] **Task 3 — Wire pickers into the drawer + basics (G7).** Basics root/target use `FileTreePicker` (single-folder); `NodeDrawer` Scope reads/writes use it (multi-select within the target repo, rooted at the project's target_repo). Keep comma-separated text as the editable fallback. Tests. Commit.
- [ ] **Task 4 — Info tooltips (G8).** Add an informational `?` affordance (accessible tooltip/popover) with concise help next to the unclear fields: Scope reads/writes/tools (first), then Role, Scale (min/max), Store capacity, Runner. Tokenized, `aria-describedby`/button-triggered, keyboard-reachable. Commit.
- [ ] **Task 5 — Model selector + availability (G6).** Replace the free-text model input in `NodeDrawer` with a **selector** of known current Claude model IDs (a curated constant list — opus/sonnet/haiku families) + a free-text **override** (flagged "unverified"). Backend: the runner **classifies a "model not found/unavailable" error distinctly** (a new `RunnerError` variant or error-class mapping) so it surfaces as "model unavailable — pick another" rather than a generic failure (note: full card-side detail is L3's job; here ensure the class exists + a clear message). Add a `test_model(model)` command + a **"Test"** button that fires a 1-token probe and reports ok/unavailable (live call — structural-only in tests, like other live-claude paths). Tests for the selector + the error classification (fake runner) + the command shape. Commit.
- [ ] **Task 6 — Impeccable pass (frontend-facing).** Over the picker, tooltips, model selector + Test button: tokens, `ui/*`, focus-visible, a11y (tree roles/keyboard, tooltip semantics, selector labelled, Test button busy/result states), empty/error states. Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-B3`

## Spec coverage
- In-app picker + `list_dir` (G7) → Tasks 1–3. ✓
- Info tooltips (G8) → Task 4. ✓
- Model selector + availability (curated list + override + runtime class + Test probe) (G6) → Task 5. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. Seal fs/probe idioms behind OHS commands (no leak past the `DirEntry`/result DTOs). The live model probe is structural-only (no headless claude). Keep `PipelineCanvas`/`NodeDrawer` contracts stable for B4.
