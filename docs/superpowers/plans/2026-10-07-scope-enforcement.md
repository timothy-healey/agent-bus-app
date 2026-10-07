# Real Scope Enforcement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A team's **Scope** actually confines its worker: isolated from the user's Claude Code setup, ungranted tools removed, distinct read and write paths, explicit plugins, auto mode where the model supports it, hard limits by rule, and every denial reported.

**Architecture:**
- The kernel (`agent_bus_core`) gains `ToolGrant`, `PermissionMode`, `PermissionDenial`/`DenialSource`, three `RunnerConfigProblem`s and `ModelOption.supports_auto_mode`.
- Pipeline Authoring's `Scope` swaps `tools` for `grants` + `plugins` (legacy `tools:` still reads). Workspace makes scope paths absolute and resolves plugins from the Claude config root (injectable).
- The Runners ACL builds the settings file (allow, deny, `autoMode.environment`), the isolated worker argv (`=` form for every variadic flag) and reads denials plus the effective permission mode from the stream. `sandbox-exec` is deleted.
- Runtime computes the effective scope per invocation, picks the mode from the model list, extends pre-flight, writes denials to the audit row and the task log. The app wires the plugin resolver, the per-run repo visibility lookup, migration 019 and a denial-count read; the frontend edits grants/plugins and shows denials.

**Tech Stack:** Rust (Tauri 2, sqlx/SQLite, serde), React 18 + TypeScript, vitest.

**Spec:** `docs/superpowers/specs/2026-10-07-scope-enforcement-design.md`
**Research:** `docs/research/worker-scope-flags.md` (branch `research/worker-scope-flags`), `docs/research/worker-auto-mode.md` (branch `research/worker-auto-mode`)

## Global Constraints

- Every worker argv carries `--setting-sources=` (one element, empty value), `--strict-mcp-config`, `--permission-prompts none`, `--permission-mode <auto|acceptEdits>`; stdin is `/dev/null`.
- Variadic flags use the `=` form as ONE element: `--add-dir=<abs>`, `--disallowed-tools=<comma list>`, `--plugin-dir=<abs>`. The user message stays the last element. `--tools` is never passed.
- Always on: Read, Glob, Grep, Skill, Edit, Write (plus `StructuredOutput`, always allowed).
- Hard limits (always denied): `Bash(git push --force:*)`, `Bash(git push -f:*)`, `Bash(git push --force-with-lease:*)`, `Bash(git push * main)`, `Bash(git push * master)`, `Bash(git push *:main)`, `Bash(git push *:master)`, `Bash(gh pr merge:*)`, `Bash(gh repo create:*)`, `Bash(gh repo delete:*)`, `Bash(git remote add:*)`.
- Without Remote git also denied: `Bash(git push:*)`, `Bash(git fetch:*)`, `Bash(git pull:*)`, `Bash(gh:*)`.
- Read path deny rules: `Edit(//<abs>/**)` and `Write(//<abs>/**)` (`//` + path without its leading `/`).
- `autoMode.environment = ["$defaults", "Repository visibility: <public|private>"]`; the second entry only when the lookup succeeded.
- YAML grants: `bash`, `agent`, `web-fetch`, `web-search`, `remote-git`, `bash(<pattern>)`. `SCHEMA_VERSION` unchanged.
- Audit error class string: `error:permission_mode_mismatch` (snake_case, per the amended run-inspector spec).
- Migration `019_invocation_denials.sql`: `ALTER TABLE invocation_audit ADD COLUMN permission_denials TEXT;`.
- Plugin resolver reads `<config_root>/plugins/installed_plugins.json` and `known_marketplaces.json`; config root is `$CLAUDE_CONFIG_DIR` or `~/.claude`, injectable in tests. Nothing writes there.
- Fixtures are real 2.1.292 captures (sonnet for auto, haiku for the fallback), redacted: no account email/org, home paths, session ids.
- UI copy and prompts: no em dashes. Code comments state the rule, never when or who decided it.
- Commands (from `src-tauri`): `cargo test --workspace --no-fail-fast`; from the repo root: `npx vitest run`, `npx tsc --noEmit`.

## Plan-level rulings (spec ambiguities)

- **Effective mode signal.** On 2.1.292 a haiku run with `--permission-mode auto` reports `"permissionMode":"auto"` in `init` and then `"permissionMode":"default"` in a following `system/status` event (captured: `permission-mode-fallback.jsonl`). The accumulator tracks the LAST `permissionMode` reported by any `system` event; a mismatch with the requested mode is `RunnerError::PermissionModeMismatch { requested, actual }`.
- **Denial tagging.** A `result.permission_denials` entry is `classifier` when a `system/permission_denied` event with the same `tool_use_id` has `decision_reason_type == "classifier"`; otherwise `rule` (a denied Write emits no system event at all, captured).
- **Known tools.** Grant → tools: `Bash` grant → `Bash`, `Monitor`; a Bash pattern keeps `Bash` (not `Monitor`); `Agent` → `Agent`, `ListAgents`, `SendMessage`; `WebFetch`, `WebSearch` → themselves. Always removed: `NotebookEdit`, `CronCreate`, `CronDelete`, `CronList`, `ScheduleWakeup`, `RemoteTrigger`, `PushNotification`, `Workflow`, `EnterWorktree`, `ExitWorktree`, `DesignSync`, `ShareOnboardingGuide`, `ReportFindings`. Bookkeeping tools that touch no files, shell or network (`ToolSearch`, `TaskCreate`/`TaskGet`/`TaskList`/`TaskUpdate`/`TaskStop`) stay. Remote git needs Bash to be useful; it adds no tool.
- **`${target_repo}` for teams on a worktree.** It binds to the task's worktree whenever the task runs in one (implementers, and code-reviewers who inherit it), so the template's code-reviewers "read worktree" is `reads: ["${target_repo}"]`.
- **Working dir.** The worker's cwd is always a read path (so it is denied for Edit/Write unless covered by a write); an implementer's cwd is always a write path.
- **Write inside read.** Deny rules beat allows, so a read path that contains a write path cannot be denied wholesale. It is descended: each existing entry on the way to the write path that is not an ancestor of a write path is denied (`<entry>/**` for directories, the exact path for files). Entries created later are not covered. The run artifacts root is the main case: other teams' artifact folders are denied, the team's own is writable.
- **Denials on failure.** `RunnerError::NoStructuredOutput` and `PermissionModeMismatch` carry the denials collected so far, so a failed invocation still records them.
- **Card badge source.** A runtime read `denial_counts() -> HashMap<task_id, u32>` sums the audit column per task; the frontend refetches it on `task-changed`.
- **Visibility once per run.** The app caches `gh repo view --json visibility` per run id inside the activation's context builder.

## Review Focus

1. **A read path containing a write path** (the artifacts root) must not deny the team's own artifact folder. Pinned in Task 3.
2. **A haiku/legacy team requesting auto** must surface as `permission_mode_mismatch`, not silently lose edit rights. Pinned in Tasks 3 and 5.
3. **A pipeline YAML written before this change** (`tools: [Read, Write, Bash]`) must still load and keep its Bash grant. Pinned in Task 2.
4. **A plugin installed from a directory marketplace whose cache folder is missing** (ddd-council) must resolve. Pinned in Task 2.
5. **The positional prompt** must survive every variadic flag. Pinned in Task 3.

---

### Task 1: Kernel types

**Files:** Create `src-tauri/agent_bus_core/src/scope.rs`; modify `agent_bus_core/src/lib.rs`, `model_list.rs`; `runners/src/model_query.rs`, `runners/src/curated_models.rs`.

**Interfaces (produces):**
- `enum ToolGrant { Bash, BashPattern(String), Agent, WebFetch, WebSearch, RemoteGit }` — serde as a string (`bash`, `bash(<p>)`, `agent`, `web-fetch`, `web-search`, `remote-git`); `ToolGrant::parse(&str) -> Option<ToolGrant>`, `as_str()`; `ToolGrant::from_legacy_tool(&str) -> Option<ToolGrant>`.
- `enum PermissionMode { Auto, AcceptEdits }` with `as_cli() -> &'static str` (`auto`, `acceptEdits`).
- `struct PermissionDenial { tool_name: String, tool_input: serde_json::Value, source: DenialSource }`, `enum DenialSource { Rule, Classifier }` (serde lowercase).
- `RunnerConfigProblem::{PluginNotFound { name }, PathUnresolvable { pattern }, RemoteGitWithoutAutoMode}`.
- `ModelOption.supports_auto_mode: bool` (`#[serde(default)]`), `ModelList::supports_auto_mode(&self, model) -> bool`, `ModelList::permission_mode(&self, model) -> PermissionMode`.

- [ ] **Step 1: failing tests** in `scope.rs`: grant round-trip for every variant (`"bash(git diff:*)"` ⇄ `BashPattern("git diff:*")`), `"Bash(git log:*)"` parses case-insensitively, unknown string fails to deserialize, legacy mapping (`Bash`, `Bash(x)`, `Agent`, `WebFetch`, `WebSearch` map; `Read`/`Write` give `None`), `PermissionMode::as_cli`, `PermissionDenial` JSON `{"tool_name","tool_input","source":"rule"}`. In `model_list.rs`: `permission_mode` gives Auto for a flagged model, AcceptEdits for an unflagged or unknown one; a cached JSON without the key reads `false`. In `model_query.rs`: the fixture gives `supports_auto_mode` true for `sonnet`, false for `haiku`.
- [ ] **Step 2:** `cargo test -p agent_bus_core -p runners model` FAIL.
- [ ] **Step 3: implement**; curated list flags every model but haiku.
- [ ] **Step 4:** PASS. **Step 5: commit** `feat(core): scope kernel types and auto-mode support flag`.

### Task 2: Scope model, path resolution, plugin resolver

**Files:** modify `pipeline/src/model.rs`, `draft.rs`, `contract_tests.rs`, `parse.rs`, `seed_template.rs` (and test constructors of `Scope` across crates); modify `workspace/src/paths.rs`; create `workspace/src/plugins.rs`, `workspace/src/visibility.rs`.

**Interfaces (produces):**
- `Scope { reads, writes, grants: Vec<ToolGrant>, plugins: Vec<String> }`; deserialises legacy `tools:` when `grants` is absent.
- `workspace::paths::resolve_absolute(pattern, vars) -> Result<PathBuf, PathResolveError>` (new variant `PathResolveError::RelativeWithoutBase(String)`); relative results join `vars.target_repo`; `.`/`..` normalised lexically.
- `workspace::plugins::{claude_config_root() -> PathBuf, PluginResolver::new(root), resolve(&self, name) -> Result<PathBuf, PluginNotFound>, list(&self) -> Vec<PluginInfo { name, marketplace, path }>}`.
- `workspace::visibility::repo_visibility(repo: &Path) -> Option<String>` (`public`/`private`/`internal`), `parse_visibility(stdout) -> Option<String>`.

- [ ] **Step 1: failing tests:** legacy YAML `tools: [Read, Write, Bash, "Bash(git diff:*)", WebFetch, Glob]` loads as `[Bash, BashPattern("git diff:*"), WebFetch]`; serialise writes `grants`/`plugins`, never `tools`; contract keys `reads, writes, grants, plugins`; seed scopes match decision 9 table (reads/writes/grants/plugins per team); `resolve_absolute("docs", repo=/r)` = `/r/docs`, `"${project}/a/../b"` = `/p/b`, relative without repo is `RelativeWithoutBase`; plugin resolver over a temp root: `superpowers` via `installPath`, `ddd-council@ddd-council` falling back to the directory marketplace (with and without `.claude-plugin/marketplace.json` `source: "./"`), unknown → `PluginNotFound`, missing files → not found, `list()` merges both sources sorted and deduped; `parse_visibility("PUBLIC\n") == Some("public")`.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** `cargo test -p pipeline -p workspace` PASS.
- [ ] **Step 5: commit** `feat(scope): grants, plugins, absolute paths and plugin resolution`.

### Task 3: Runners ACL: settings, argv, stream, stdin

**Files:** modify `runners/src/scope.rs`, `command.rs`, `claude_cli.rs`, `output.rs`, `stream_json.rs`, `fake.rs`, `anthropic_api.rs`, `lib.rs`; fixtures `permission-denied-rule.jsonl`, `permission-denied-classifier.jsonl`, `permission-mode-fallback.jsonl` (captured); `app/src/process_registry.rs` (stdin null).

**Interfaces (produces):**
- `WorkerScope { reads: Vec<String>, writes: Vec<String>, grants: Vec<ToolGrant>, repo_visibility: Option<String> }` (absolute paths).
- `build_settings(&WorkerScope, list_dir: &dyn Fn(&Path) -> Vec<(PathBuf, bool)>) -> SettingsFile { permissions, auto_mode: AutoMode { environment } }`.
- `disallowed_tools(&[ToolGrant]) -> Vec<String>`, `HARD_LIMITS`, `REMOTE_GIT_DENY`.
- `prepare(project_root, team_id, task_id, ts, &WorkerScope) -> ScopeSettings { settings_path, add_dirs, disallowed_tools }`.
- `InvocationRequest` loses `sandbox_profile`, gains `permission_mode: PermissionMode`, `disallowed_tools: Vec<String>`, `plugin_dirs: Vec<String>`.
- `RunnerOutput.permission_denials: Vec<PermissionDenial>`; `RunnerError::NoStructuredOutput { detail, usage, denials }`; `RunnerError::PermissionModeMismatch { requested, actual, usage, denials }`; `RunnerError::denials()`.
- `StreamAccumulator::finish(self, model, kind, requested: PermissionMode)`.
- `sandbox_profile`, `sandbox_wrap`, `SANDBOX_BIN` deleted.

- [ ] **Step 1: failing tests** (spec Testing → Settings/Argv/Accumulator):
  - settings: a read path yields `Edit(//r/**)` + `Write(//r/**)`; a write path doesn't; a read inside a write isn't denied; a write inside a read denies only the siblings (lister fake: `/a/x` dir, `/a/own` dir, `/a/f.md` file → `Edit(//a/x/**)`, `Edit(//a/f.md)`, nothing for `/a/own`); every hard limit present; remote-git denies present only without the grant; Bash patterns become `Bash(<p>)` allows; `autoMode.environment[0] == "$defaults"`, visibility line only when known.
  - argv: `--setting-sources=`, `--strict-mcp-config`, `--permission-prompts none`, `--permission-mode` follows the request; every `--add-dir`/`--disallowed-tools`/`--plugin-dir` element uses `=`; no bare variadic flag; the last element is the prompt; `--disallowed-tools=` lists exactly `disallowed_tools`; `Skill` never in it; one `--plugin-dir=` per plugin; no `--tools`.
  - disallowed: no grants removes Bash, Monitor, Agent, WebFetch, WebSearch and the always-removed set; `Bash` grant keeps Bash and Monitor; `BashPattern` keeps Bash but removes Monitor; never Read/Glob/Grep/Skill/Edit/Write/StructuredOutput.
  - accumulator: rule fixture → two `Rule` denials (Write without event, Bash with `subcommandResults`); classifier fixture → one `Classifier` denial; fallback fixture with `Auto` requested → `PermissionModeMismatch { requested: "auto", actual: "default" }`; a fixture requested as `AcceptEdits` whose init says `acceptEdits` passes.
  - stdin: the killable spawner runs `sh -c 'read x; echo $?'` and gets a non-zero status line (EOF), not a hang.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** `cargo test -p runners` PASS.
- [ ] **Step 5: commit** `feat(runners): isolated worker argv, scope settings and denial parsing`.

### Task 4: Runtime: effective scope, mode, pre-flight, audit, log

**Files:** modify `runtime/src/engine.rs`, `preflight.rs`, `invocation_audit.rs`, `api.rs`; create `app/migrations/019_invocation_denials.sql`; app migration list + test.

**Interfaces (produces):**
- `EngineContext` gains `model_list: Option<Arc<RwLock<ModelList>>>`, `plugin_resolver: Option<Arc<PluginResolver>>`, `repo_visibility: Option<String>`.
- `engine::effective_scope(team, task_worktree, working_dir, artifact_base, artifact_dir, vars) -> Result<WorkerScope, String>`.
- `preflight::check_pipeline(pipeline, list, env: &PreflightEnv { project_root, target_repo, plugins: Option<&dyn Fn(&str) -> bool> })`.
- `ErrorClass::PermissionModeMismatch` → `permission_mode_mismatch`.
- `InvocationAuditStore::record_settle(id, outcome, usage, denials: &[PermissionDenial], now)`; `InvocationRow.permission_denials: Vec<PermissionDenial>`; `denial_counts() -> HashMap<String, u32>`.
- `LogKind::Denial`; the engine emits one `Denial` delta per denial: `"<tool> (<rule|classifier>): <input summary>\n"`.
- `RuntimeState::with_plugin_resolver`, `api::denial_counts` Tauri command.

- [ ] **Step 1: failing tests:** implementer `${target_repo}` = worktree and worktree is a write; code-reviewer on an inherited worktree reads it; relative read resolves against the target repo; artifacts root is a read and own folder a write; request carries `Auto` for a flagged model and `AcceptEdits` otherwise; plugin dirs resolved; denials reach the audit row and the log sink; a mismatch error audits `error:permission_mode_mismatch` and takes the operational-failure path; pre-flight names the team for each new problem (plugin, path, Remote git on haiku); migration 019 adds a nullable column; `denial_counts` sums per task.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** `cargo test -p runtime -p app` PASS.
- [ ] **Step 5: commit** `feat(runtime): enforce scopes per invocation and report denials`.

### Task 5: Composition root

**Files:** modify `app/src/lib.rs`, `pipeline_activator.rs`.

- Plugin resolver over `claude_config_root()` shared by pre-flight, the activator and a `list_plugins` command.
- Per-run visibility cache in the context builder (`gh` lookup once per run id).
- Register `list_plugins` and `denial_counts`; task-log sink forwards `denial` kind immediately (no throttle).
- [ ] Tests: `list_plugins_inner` over a temp root; visibility cache calls the lookup once per run. **Commit** `feat(app): wire plugins, repo visibility and denial reads`.

### Task 6: Frontend

**Files:** `src/ipc/pipeline.ts`, `src/ipc/models.ts`, `src/ipc/runtime.ts`, `src/ipc/plugins.ts` (new), `src/wizard/draft.ts`, `src/wizard/canvas/NodeDrawer.tsx`, `src/wizard/canvas/SkillAutocomplete.tsx`, `src/components/ui/Card.tsx`, `BoardView.tsx`, `ListView.tsx`, `CardDrawer.tsx`, `src/hooks/useTaskLog.ts`, `src/hooks/useDenialCounts.ts` (new), `src/lib/outcomeLabel.ts`, `src/App.tsx`, affected tests.

- Scope fieldset: Reads/Writes unchanged; grant checkboxes (Bash (all), Agent, WebFetch, WebSearch, Remote git); Bash patterns (comma-separated); plugin checkboxes over `listPlugins()` (plus any declared but not installed, marked); help text: removed versus denied, Remote git needs an auto-mode model.
- Picking a namespaced skill in the prompt autocomplete adds its plugin.
- Card denial badge (`denials` prop, hidden at 0); history tab lists each denial (tool, input, tag); live log renders `denial` segments.
- [ ] Tests first (vitest): draft mutators, grant checkbox toggles, autocomplete adds plugin, badge renders count, history shows denials, outcome label. **Commit** `feat(ui): grants, plugins and permission denials`.

### Task 7: Cleanup, live check, verification

- Delete the remaining sandbox references (runners lib doc), update `docs/v1.1-backlog.md`/`roadmap-remaining.md` only where they describe sandbox as current.
- Live check (manual, scratch repo + LOCAL bare origin): an `#[ignore]` runners test builds settings + argv for an implementer scope with Remote git and runs the real CLI: push of the branch succeeds; force-push is denied by rule and reported as a `Rule` denial.
- Full verification commands; code review; PR.
