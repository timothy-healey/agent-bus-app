# Spec — A4 · Skill/command autocomplete in prompt authoring

*Design doc. Brainstormed 2026-06-24 (visual companion). Backlog item **A4** (Pipeline authoring). Its own plan → implementation cycle. Authoring-time only — it does NOT change how the worker invokes skills.*

## Why this exists

Today a team's responsibility prompt is a bare `<textarea>` (`PromptsStep`, reused by `PipelineEditor`). Operators hand-type skill references like `/ddd-council vet` or `/superpowers:brainstorming` with no help, no feedback, and no idea which skills are actually installed on the machine the worker runs on. A4 adds **discovery-backed autocomplete + token highlighting** so the operator types *correct* references against the *real* set of skills/commands available to the worker.

This is authoring sugar. The worker still runs `claude --print` exactly as before; A4 only improves the text the author writes.

## Decisions (from the brainstorm)

1. **Scope: real discovery + autocomplete, verbs best-effort.** A real filesystem scanner (not a static list), behind a seam. Verb cascade only for skills that declare verbs in a recognizable way (e.g. `ddd-council`); every other skill is name-only.
2. **Catalog entries: skills + slash commands, unified.** Both plugin skills (`SKILL.md`) and slash commands (plugin `commands/` + user `~/.claude/commands/`), one ranked `/`-triggered list.
3. **Highlighting: mirror-overlay.** Keep the plain `<textarea>`; render a synced highlight layer behind it that tints recognized `/skill` tokens (the standard highlight-within-textarea technique). No contenteditable rewrite — the textarea is shared by the wizard Prompts step and the Pipeline editor, so low-risk matters.
4. **Sources: configurable per project.** Global `~/.claude` is always scanned; the operator may add one or more project `.claude` roots (reusing A3's `FolderPickerField`). The catalog merges them, tags each entry by source, and the project source wins on a name collision. This reflects what the worker actually sees (Claude Code layers global + project-scope skills by cwd) and enables deliberate, reproducible per-project skill sets.

## Architecture

A new **`skills` crate** — a *generic subdomain* seam, NOT an 8th bounded context (same call as S1's `secrets`). It owns skill discovery; the filesystem/`SKILL.md`-parsing idiom is sealed inside the trait, and only the `SkillEntry` DTO crosses.

```
skills crate (generic subdomain)
  trait SkillCatalog { fn list(&self, roots: &[ClaudeRoot]) -> Vec<SkillEntry> }
    ├─ FsSkillScanner   (real — walks the filesystem)
    └─ FakeSkillCatalog (tests — canned entries)
        │ SkillEntry DTO (the only type that crosses; wire-contract test)
        ▼
  OHS command  list_skills(project_id) -> Vec<SkillEntry>   (app/src/lib.rs, composition root)
        │  resolves roots = [global ~/.claude] + project.skill_sources
        ▼
  Frontend (Pipeline Authoring editor surface — pure frontend)
    ipc/skills.ts (listSkills) → useSkillCatalog (load at boot + manual refresh, in-memory cache)
      → SkillAutocomplete over the shared prompt textarea:
          caret popover · mirror-overlay highlight · verb cascade
      used by BOTH PromptsStep (wizard) and PipelineEditor
```

No database read/write for discovery (read-only filesystem, nothing persisted). The one persistence change is the per-project source config (below).

## The `SkillEntry` DTO

```rust
pub struct SkillEntry {
    pub name: String,             // bare invocation name, e.g. "ddd-council", "init-session"
    pub kind: SkillKind,          // Skill | Command
    pub namespace: Option<String>,// plugin name, for the "plugin:skill" qualified form
    pub description: String,      // from SKILL.md / command frontmatter (may be empty)
    pub verbs: Vec<String>,       // best-effort; [] when none declared
    pub source: SkillSource,      // Global | Project  (for the popover tag + collision precedence)
}
```

`SkillKind` and `SkillSource` are small enums owned by the `skills` crate. A serde wire-contract test pins the JSON shape the frontend consumes (the discipline established for `WorktreeEntry`/IPC types).

## Discovery scanner (`FsSkillScanner`)

A **source** is a `.claude` directory. The scanner knows the `.claude` internal layout and scans, per root:

- **Plugin skills:** `<root>/plugins/cache/<marketplace>/<plugin>/<version>/skills/<name>/SKILL.md` → parse YAML frontmatter (`name`, `description`); `kind: Skill`, `namespace: <plugin>`.
- **Direct skills:** `<root>/skills/<name>/SKILL.md` (project `.claude` layout) → `kind: Skill`, no namespace.
- **Plugin commands:** `<root>/plugins/cache/.../<version>/commands/*.md` → `kind: Command`.
- **Direct commands:** `<root>/commands/*.md` (e.g. user `~/.claude/commands/init-session.md` → `/init-session`) → `kind: Command`.

Resolution rules:

- **Versions:** a plugin may have multiple version dirs — take the newest by semver so a skill isn't listed N times.
- **Collisions:** same name within a source → newest version wins. Same name across **sources** → the **project** source wins; the losing entry is still offered under its qualified `namespace:name` form. Within one source, a name unique across plugins inserts bare `/name`; an ambiguous name inserts the qualified `/plugin:name`.
- **Robustness:** malformed/missing frontmatter → skip that one entry, never fail the scan. A missing root (e.g. no `~/.claude`, or a configured project root that doesn't exist) → contributes nothing, no error.

**Best-effort verb extraction** (lenient; never fails discovery):

1. If the plugin declares commands explicitly (a `commands/` dir or a manifest list) → use those as verbs.
2. Else, look in `SKILL.md` for a recognizable **router table** — a markdown table whose header contains a "Verb" column (`ddd-council`'s `| Verb | Reference | … |`) — and take the first column.
3. No recognizable pattern → `verbs: []` (name-only insert).

So `ddd-council` gets its `vet/critique/map/boundaries/…` cascade; `brainstorming` just inserts `/superpowers:brainstorming`.

**Refresh:** scanned once at app boot and on an explicit "refresh skills" action; cached in memory only.

## Per-project source config

`Project` gains **`skill_sources`** — a list of extra `.claude` roots beyond global, stored as a **JSON-encoded TEXT column** on `projects` (migration **011**, append-only nullable; `null` = global only). Registered in both `app/src/lib.rs` migration lists and the `user_version` idempotency test bumped 10→11. (A JSON column over a child table: the list is small and always read/written whole — same pragmatism as the rest of the project row.)

Surfaced by:
- a new optional field on the relevant create/update path, and
- a **Settings → Projects → Skill sources** editor (list global as locked/always-on; add via `FolderPickerField`; remove), mirroring A5's per-project target-repo editor.

The list is read at the composition root and handed to the scanner as resolved roots — Pipeline Authoring/the `skills` crate never learn about the `Project` type (same seam discipline as `project_root`/`target_repo`).

## Frontend

A `SkillAutocomplete` wrapper around the shared prompt textarea (`src/wizard/PromptsStep.tsx` + `src/wizard/PipelineEditor.tsx` both adopt it):

- **Trigger + popover:** typing `/` opens a caret-anchored popover that filters as the operator types. Each row shows the insert form, a `· skill`/`· command` kind tag, a `· global`/`· project` source tag, and the description. Keyboard: ↑/↓ move, ⏎ selects, Esc dismisses.
- **Verb cascade:** selecting a skill that has `verbs` opens a second cascade of its verbs; selecting one appends ` <verb>`. A skill with no verbs just inserts the name and closes.
- **Mirror-overlay highlight:** a layer synced to the textarea's scroll/metrics tints `/skill` tokens that match a catalog entry. Unrecognized `/foo` tokens simply aren't tinted — implicit "not installed / typo" feedback, no separate validation system (YAGNI).
- **Data:** `ipc/skills.ts` (`listSkills(projectId)` + `SkillEntry`/`SkillKind`/`SkillSource` types) and a `useSkillCatalog` hook (load at boot for the active project + manual refresh, in-memory cache). A "refresh skills" affordance lives next to the editor.

The component is the only place that knows the popover/overlay mechanics; it consumes `SkillEntry[]` and emits text changes through the existing `setPromptBody`/draft path — no change to how prompts are stored or written.

## Testing (all green, no live filesystem dependence)

**Rust (`skills` crate):**
- `FsSkillScanner` parsing over a **fixture `.claude` tree** (a temp dir built in-test): plugin skills, direct skills, plugin + user commands; frontmatter → name/description/namespace.
- Version resolution (newest wins), cross-source collision (project wins, global offered qualified), cross-plugin ambiguity (qualified insert).
- Robustness: malformed/missing frontmatter skipped; missing root contributes nothing; empty/absent `~/.claude` → empty catalog.
- Verb extraction: a `ddd-council`-shaped router table → verbs; a verb-less skill → `[]`; an unparseable table → `[]` (never errors).
- `SkillEntry` serde wire-contract test (shape the frontend depends on).
- `FakeSkillCatalog` returns canned entries for downstream/root tests.

**Frontend (vitest):**
- `useSkillCatalog` loads via a mocked `listSkills`, exposes entries + refresh.
- `SkillAutocomplete`: `/` opens the popover; typing filters; ⏎/click inserts the correct form (bare vs qualified); verb cascade appends a verb; Esc dismisses; source/kind tags render.
- Mirror-overlay: recognized tokens get the highlight class, unrecognized ones don't.
- Settings skill-sources editor: add (mocked `pickFolder`) / remove updates the project; global shown locked.

**Caveat (honest scope):** the live filesystem scan of a real `~/.claude` can't be asserted deterministically in CI; coverage is via in-test fixture trees + `FakeSkillCatalog`. The real `FsSkillScanner` against the actual install is structural-only (the same parse path the fixtures exercise) — the same honesty applied to the keychain/git/E2E seams.

## DDD notes

- **Generic subdomain**, not a new bounded context (`skills`, like `secrets`).
- **Seam-sealed:** filesystem paths and `SKILL.md` parsing never cross the `SkillCatalog` trait; only `SkillEntry` does.
- **No new cross-context edge:** the project's `skill_sources` are resolved to plain roots at the composition root; the `skills` crate and Pipeline Authoring never learn the `Project` type.
- **Authoring-time only:** no change to the Runner/worker invocation path.

## Out of scope / non-goals

- Changing how the worker invokes or resolves skills (that's the worker's `claude` environment, untouched).
- A standalone reference-validation/linting system beyond the implicit "unrecognized token isn't highlighted."
- A rich-text/contenteditable prompt editor (explicitly rejected in favor of the overlay).
- Universal verb schemas — verb support is best-effort for skills that declare verbs.
- Watching the filesystem for live skill changes (manual refresh only).

## Relationship to other items

- Builds on **A3** (`FolderPickerField`, `pickFolder` seam) for the source editor and **A5** (per-project config precedent: `target_repo`).
- The "which project dir is the worker's scope" ambiguity is the **L1** (worker cwd / artifact) question deferred to a separate brainstorm; A4's *configurable* sources deliberately decouple from it (the operator names the roots explicitly).
- Independent of **A6** ("Create & start" kickoff), the next A item to brainstorm.
