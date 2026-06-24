# A4 — Skill/Command Autocomplete Implementation Plan (chunk ③)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Detailed design: `docs/superpowers/specs/2026-06-24-a4-skill-autocomplete-design.md` (read first; source of truth). This plan is the task decomposition + chunk-② reconciliation + verification gates.

**Goal:** Discovery-backed `/`-autocomplete + mirror-overlay token highlighting when authoring a team's responsibility prompt — over the **NodeDrawer** prompt field (the graph-builder's Team editor), backed by a real filesystem scanner behind a `SkillCatalog` seam, with per-project configurable skill sources.

**Architecture:** A new generic-subdomain `skills` crate (`SkillCatalog` trait + `FsSkillScanner` + `FakeSkillCatalog`, `SkillEntry` DTO). A `list_skills(project_id)` OHS command resolves roots = global `~/.claude` + the project's configured sources, scans, merges, tags by source. Frontend: `useSkillCatalog` + a `SkillAutocomplete` wrapper on the drawer textarea. Authoring-time only; does not change worker invocation.

**Tech Stack:** Rust (serde, a YAML frontmatter parser — reuse `serde_yaml` already in the workspace), SQLite (sqlx), React/TS, vitest.

---

## Chunk-② reconciliation (do FIRST)

- Prompt editing now lives in **`src/wizard/canvas/NodeDrawer.tsx`** (the `TeamEditor` component, `<textarea aria-label={`prompt for ${id}`}>` bound to `setPromptBody`). A4's `SkillAutocomplete` wraps **THAT** textarea. The old `PromptsStep` is retired (still in-tree with tests; do not target it).
- `Project` already has `target_repo` (migration 010, A5). The new `skill_sources` is migration **011**. Reuse A5's pattern (`workspace_set_target_repo` → mirror as `workspace_set_skill_sources`) and A3's `FolderPickerField`/`pickFolder` seam for the Settings editor.

## Tasks (TDD)

- [ ] **Task 1 — `skills` crate scaffold + DTO.** New crate `src-tauri/skills/` (add to the workspace `Cargo.toml` members). Define `SkillEntry { name, kind: SkillKind(Skill|Command), namespace: Option<String>, description, verbs: Vec<String>, source: SkillSource(Global|Project) }`, the `SkillCatalog` trait (`fn list(&self, roots: &[ClaudeRoot]) -> Vec<SkillEntry>` where `ClaudeRoot { path, source }`), and `FakeSkillCatalog`. Serde-derive + a wire-contract test pinning the JSON shape. Commit.
- [ ] **Task 2 — `FsSkillScanner` parsing** (pure where possible) + fixture-tree tests. Per root, scan: `plugins/cache/*/*/*/skills/*/SKILL.md` (frontmatter name/description; namespace = plugin), `skills/*/SKILL.md` (direct), `plugins/cache/*/*/*/commands/*.md` + `commands/*.md` (kind Command). Rules: newest semver version wins; cross-plugin name collisions kept (qualified); malformed/missing frontmatter skipped (never crash); missing root → nothing. Build fixture `.claude` trees in a tempdir in-test. Commit.
- [ ] **Task 3 — best-effort verb extraction** + tests. Priority: (1) a plugin-declared `commands/` list → verbs; (2) a `SKILL.md` router table whose header has a "Verb" column → first column; (3) else `verbs: []`. Parsing never errors (bad table → empty). Test against a ddd-council-shaped router-table fixture (verbs) and a verb-less skill (empty). Commit.
- [ ] **Task 4 — `Project.skill_sources` + migration 011.** Append-only nullable JSON TEXT column `skill_sources` on `projects` (migration `011_skill_sources.sql`; register in BOTH `app/src/lib.rs` migration lists; bump the `user_version` idempotency test 10→11). Add `skill_sources: Vec<String>` to the `Project`/`workspace` model (JSON-encoded; default empty), and a `workspace_set_skill_sources(id, paths)` OHS command (mirror `workspace_set_target_repo`). Tests. Commit.
- [ ] **Task 5 — `list_skills(project_id)` OHS command.** At the composition root (`app/src/lib.rs`): build roots = `[Global ~/.claude]` + project.skill_sources (each as a `Project` `.claude` root), call the scanner, merge, tag by source, apply collision precedence (project wins; loser qualified). Register in `tauri::generate_handler!` and `tools()`. Wire the real `FsSkillScanner` (held in state or built per-call). Test the merge/precedence with `FakeSkillCatalog`. Commit.
- [ ] **Task 6 — Frontend IPC + hook.** `src/ipc/skills.ts` (`listSkills(projectId) → SkillEntry[]`, `SkillEntry`/`SkillKind`/`SkillSource` types, wire-contract test) + `src/hooks/useSkillCatalog.ts` (load for the active project at boot + a manual `refresh`, in-memory cache). Tests with mocked `listSkills`. Commit.
- [ ] **Task 7 — `SkillAutocomplete` component** (pure logic unit-tested; component orchestration tested). Caret-anchored popover: `/` opens it, filters as typed; rows show insert form + `· skill`/`· command` + `· global`/`· project` + description; ↑/↓/⏎/Esc. Verb cascade for entries with `verbs`. Mirror-overlay layer that tints recognized `/skill` tokens (recognized = matches a catalog entry; unrecognized untinted). Pure helpers (token scan, match, insert form incl. bare vs qualified) unit-tested. Commit.
- [ ] **Task 8 — Wire into NodeDrawer.** Replace the `TeamEditor` prompt `<textarea>` with `SkillAutocomplete` (same `setPromptBody` onChange; feed the catalog from `useSkillCatalog`). Component test: typing `/` in the drawer prompt shows the popover; selecting inserts the token; recognized tokens highlight. Commit.
- [ ] **Task 9 — Settings skill-sources editor.** Settings → Projects: a per-project Skill sources list — global `~/.claude` shown locked/always-on; add via `FolderPickerField` (`pickFolder`); remove; persists via `workspace_set_skill_sources`. Tests (mock `pickFolder`/the command). Commit.
- [ ] **Task 10 — Impeccable pass (frontend-facing).** Run the impeccable skill over the popover/overlay/settings surface: focus-visible, tokens (no px/hex literals), `ui/Button`, a11y (popover roles/`aria-activedescendant`, keyboard nav, Esc), empty/loading states. Apply safe fixes. Commit.

## Verification gates (all must pass before tag)

- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-a4`

## Spec coverage

- `skills` generic-subdomain crate + `SkillCatalog` seam + `SkillEntry` + `list_skills` → Tasks 1,2,5. ✓
- Unified skills + commands, `.claude`-layout scan → Task 2. ✓
- Version/collision/robustness rules → Task 2. ✓
- Best-effort verb cascade → Tasks 3,7. ✓
- Mirror-overlay highlight → Task 7. ✓
- Configurable per-project sources (migration 011, Settings editor, tags, precedence) → Tasks 4,5,9. ✓
- Lands on the NodeDrawer prompt field (chunk-② reconcile) → Task 8. ✓
- Fixture/fake testing; live scan structural-only → Tasks 2,3. ✓

## Constraints

Local commits only, NEVER push. Commit per task. The `skills` crate is a generic subdomain (like `secrets`) — seam-sealed: filesystem/`SKILL.md` parsing never crosses the trait, only `SkillEntry`. Authoring-time only; no change to worker invocation. `list_skills` resolves project sources at the composition root — the `skills` crate never learns the `Project` type.
