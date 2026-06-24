---
id: vet-a3-a5-target-repo-2026-06-24
verb: vet
lens: strategic · vet · brief
target: plans/2026-06-24-plan-a3-a5-target-repo.md
verdict: SOUND WITH FIXES
---

# DDD vet — A3 (folder picker) + A5 (project target-repo binding)

Reviewed the plan against `DOMAIN.md` (the seven contexts, the path-resolution
shared kernel, the Workspace + Runtime ubiquitous language) and the affected code
(`workspace/src/{project,store,api}.rs`, `workspace/src/paths.rs`,
`runners/src/scope.rs`, `runtime/src/{api,pool,task}.rs`, `app/src/lib.rs`).

## Summary

The design is **sound**. It places `target_repo` in the context that owns the
concept (Workspace `Project`), binds `${target_repo}` through the existing
**path-resolution shared kernel** (`PathVars`) at the worker-loop/composition
root, and keeps **Runtime ignorant of the Project type** by handing in plain
resolved strings — the same seam discipline the codebase already uses for
`project_root` (RuntimeState/PoolContext) and the R5/S1 "resolve-at-root, never
leak the type" pattern. Task-overrides-project precedence is correct. Migration
discipline (010 append-only, both lists, idempotency bump) matches the
established 007/008/009 pattern. Three findings, all **low**, all language/DRY —
none blocks the build; all applied in-plan.

---

### F1 [low] Off-language naming — register `target_repo` as a Project attribute in the kernel glossary

**What.** `DOMAIN.md` → *Cross-context (the kernel)* describes `${target_repo}`
as "the splose-monorepo or other repo being modified" and lists **Project** as
"a workspace with a root directory, an active pipeline, and a registered set of
pipelines." The plan adds a *project-level default* for `${target_repo}` — a new,
load-bearing fact about the Project — but the glossary doesn't yet name it.

**Cited plan section.** Task 2 (`Project.target_repo` field); Decisions DD1/DD2.

**Why it matters.** The whole point of A5 is "set the repo ONCE on the project
instead of per-team." That the Project now *binds* `${target_repo}` (and the
per-task value overrides it) is exactly the kind of ubiquitous-language fact the
glossary exists to pin — otherwise the next reader sees `${target_repo}` as
task-only (its pre-A5 reality) and misses the project default.

**Amendment.** In the backlog/DOMAIN update step, register in `DOMAIN.md`:
(a) extend **Project** to "…a root directory, an optional **target repo**, an
active pipeline…"; (b) extend the **Path variables** entry so `${target_repo}`
reads "…the repo a task targets; **defaults to the Project's `target_repo`** and
is overridden per-task at inject (A5)." Naming itself (`target_repo`,
`project_target_repo`, `workspace_set_target_repo`) is on-language — no rename.

**Status:** resolved — DOMAIN.md language to be updated in the backlog step (Task 13).

---

### F2 [low] Adds where a refactor fits / DRY — the task-overrides-project precedence rule lives in two Runtime sites

**What.** The precedence "task value wins, else project default" is encoded
twice: once in `pool.rs` (`effective_target_repo`, for the PathVars build) and
once in `api.rs` (`inject_topic_inner`, for the stored task default). They must
agree forever; if one changes (e.g. trim-empty handling), they silently diverge.

**Cited plan section.** Task 4 Step 1/4 (`effective_target_repo`); Task 5 Step 4
(`inject_topic_inner` `.filter(...).or_else(...)`).

**Why it matters.** It's a single domain rule ("project is the default for
`${target_repo}`; task overrides") split across two functions in the same
context. Low blast radius (both in `runtime`), but it's the *Refactor before you
add* law in miniature — one rule, one home.

**Amendment.** Keep both call sites (they bind different things — the live
PathVars vs the stored task field — at genuinely different moments), but make
`effective_target_repo` the **single** precedence function and have
`inject_topic_inner` call it instead of its own inline `.filter().or_else()`.
Make it `pub` on `runtime::pool` (or move to a small `runtime` helper) and reuse.
This is a one-line reuse, not a new abstraction. Note in the plan that the two
sites are *deliberately separate bindings of the same rule*, sharing one
precedence fn.

**Status:** resolved — Task 5 amended to call `pool::effective_target_repo`.

---

### F3 [low] Cross-boundary check — confirm the dialog plugin idiom is sealed at the IPC wrapper (UI/Workspace concern)

**What.** A3 adds `@tauri-apps/plugin-dialog` / `tauri-plugin-dialog`. The plan
already routes the native call through a `pickFolder()` IPC wrapper and mocks it
in tests — good. The only risk is the `open({directory:true})` idiom leaking into
components (so every consumer imports the plugin) rather than staying behind the
one wrapper.

**Cited plan section.** Task 8 Step 2 (`pickFolder` wraps `open(...)`); Task 9
(`FolderPickerField` imports `pickFolder`, not `open`).

**Why it matters.** Same ACL-seal discipline as the keychain/git/HTTP seams
(DOMAIN.md S1/S2/R1): the third-party idiom (`@tauri-apps/plugin-dialog`'s
`open`) should cross into our code in exactly one place. `FolderPickerField` and
the wizard/Settings must depend on `pickFolder()`, never on the plugin directly.

**Why it's only low / confirm-only.** The plan already does this — `pickFolder`
is the sole importer of `open`, and `FolderPickerField` imports `pickFolder`.
This finding just makes the invariant explicit so it isn't eroded later.

**Amendment.** Add a one-line doc-comment on `pickFolder()`: "Sole crossing point
for the `@tauri-apps/plugin-dialog` idiom — components depend on this wrapper, not
the plugin." No structural change.

**Status:** resolved — doc-comment added to `pickFolder` (Task 8).

---

## What the room checked and cleared

- **Concept ownership (architect).** `target_repo` is a Workspace `Project`
  attribute — correct. The `${target_repo}` *variable* stays owned by the
  Workspace path-resolution kernel (`paths.rs`); the plan adds no new path
  variable, only a new *source* for the existing one. No new kernel, no unowned
  shared type.
- **Runtime ignorance (architect/engineer).** Runtime receives the project
  default as `Option<String>` (RuntimeState) / `Option<PathBuf>` (PoolContext) —
  plain resolved values handed in at `load_active`, exactly mirroring the existing
  `project_root` plumbing. Runtime never imports `workspace::Project`. No
  cross-boundary dependency; no new edge. Confirmed against `runtime/src/api.rs`
  (RuntimeState already holds `project_root: String`) and `pool.rs` (PoolContext
  already holds `project_root: PathBuf`).
- **Precedence (engineer).** task `target_repo` overrides project default; the
  worker resolves `${target_repo}` via PathVars built from `task.or(project)`, and
  inject stamps the default onto the stored task. The pre-A5 task-level path
  (`Task::injected`/`forked`/`spawn_continuation` already carry `target_repo`)
  is preserved — fork lanes and continuations inherit, unchanged.
- **Migration discipline (engineer).** 010 is a nullable `ADD COLUMN` (append-only,
  001–009 untouched), registered in BOTH the `run_migrations` const list and the
  `tauri-plugin-sql` vec, with the idempotency test bumped 9→10 and a
  column-presence assertion — identical to the 007/008/009 precedent. The
  user_version gate makes the non-idempotent ALTER safe on re-run.
- **Refactor-before-add (engineer).** The plan reuses the existing task-level
  binding rather than reinventing it; `FolderPickerField` is genuinely new and
  reused twice (wizard + Settings) — a justified addition, not duplication.
