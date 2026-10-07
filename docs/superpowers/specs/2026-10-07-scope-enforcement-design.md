# Spec — Real scope enforcement

*Design doc. Wayfinder map: "Map: runner capabilities from the current Claude CLI" (#3). Ticket: "Spec: real scope enforcement" (#10). Research: "Research: what --bare, --strict-mcp-config and --disallowed-tools change for a worker" (#9, `docs/research/worker-scope-flags.md` on `research/worker-scope-flags`) and the auto-mode probe run for this ticket (`docs/research/worker-auto-mode.md` on `research/worker-auto-mode`). A team's **Scope** actually confines its worker:*
- *it runs isolated from the user's Claude Code setup;*
- *tools it isn't granted are removed;*
- *its read and write paths are distinct;*
- *the plugins it declares are loaded explicitly;*
- *auto mode judges the risky actions left over;*
- *hard limits are enforced by rule.*

*Every denial is reported. Terms are as defined in `DOMAIN.md` (Runners, Workspace).*

## Why this exists

A team's scope promises more than it delivers:

- **Reads and writes are the same.** Both become `--add-dir`, and under `--permission-mode acceptEdits` any added directory is writable.
- **Paths are not absolute.** The editor stores scope paths relative to the repo, and they reach `--add-dir` unresolved, so they resolve against the worker's cwd.
- **Implementers can reach the main repo.** `${target_repo}` still points at the main repo even though implementers work in a worktree.
- **`tools` is a raw allow list.** It gates only commands that need approval: read-only Bash (`ls`, `cat`) runs regardless, and `Agent` is not gated at all.
- **Workers inherit the user's setup.** They load the user's hooks and `enabledPlugins`. That is the only way they get skills today, and it makes runs depend on whoever's machine they run on.
- **Nothing reports denials.** A denied action is visible only in the model's own prose.
- **The sandbox layer is unused.** The `sandbox-exec` layer is never wired up, and the toggle it needs no longer exists.
- **The template has no scopes.** Every team in the bundled DDD template has an empty scope.

The CLI offers what's needed, verified on 2.1.289 and 2.1.292 with subscription auth:

- `--setting-sources=` drops user and project settings, hooks and plugins, but keeps auth. `--bare` breaks subscription auth.
- `--plugin-dir=<path>` loads one plugin explicitly.
- `--disallowed-tools=` removes a tool for the worker and its subagents. Removal never shows in `permission_denials`.
- `--settings` allow and deny rules still apply and always beat the auto-mode classifier.
- `--permission-mode auto` works headless on Sonnet and Opus 4.6+. On other models it silently falls back to `default`; `init.permissionMode` shows which mode is actually in effect.
  - The classifier adds no usage or Cost.
  - It judges Bash and remote actions against the task's intent.
  - Its blocks show in `permission_denials` and as `system/permission_denied` events with `decision_reason_type: "classifier"`.
  - Its verdicts are not deterministic, so anything that must hold is a rule.

## Decisions

1. **Isolated workers.** Every worker invocation passes:
   - `--setting-sources=`
   - `--strict-mcp-config`
   - `--permission-prompts none`

   It runs with stdin from `/dev/null`. Workers never load the user's hooks, plugins or MCP servers.
2. **Permission mode by model.**
   - **Mode:** `auto` when the team's model has `supportsAutoMode` in the model list (from the Effort spec's `initialize` list), otherwise `acceptEdits`.
   - **Mode check:** the runner reads `init.permissionMode`. A mismatch with the requested mode is an operational failure (`error:permission-mode-mismatch`).
   - **Context for the classifier:** the settings file carries `autoMode.environment: ["$defaults", "Repository visibility: <public|private>"]`. Visibility comes from `gh repo view --json visibility` on the target repo, once per run. The line is left out if that lookup fails.
3. **Tools.**
   - **Always on:** Read, Glob, Grep, Skill, Edit, Write. Where Edit and Write may touch is limited by decision 4.
   - **Grantable per team:**
     - `Bash` (all commands) or `Bash(<pattern>)` entries;
     - `Agent`;
     - `WebFetch`;
     - `WebSearch`;
     - **Remote git** (decision 5).
   - **Removal:** every known tool not granted is passed in `--disallowed-tools=<comma list>`, using the `=` form. `--tools` is never used, because it removes `Skill`.
   - **Bash patterns** become `permissions.allow` entries.
   - **Known limitation:** in auto mode, broad allows (`Bash`) are ignored by the CLI and go to the classifier; in `acceptEdits`, read-only Bash runs even when not listed. Both are accepted behaviour.
4. **Paths.**
   - **Writes:** each becomes `--add-dir=<abs>`.
   - **Reads:** each becomes `--add-dir=<abs>` plus deny rules `Edit(//<abs>/**)` and `Write(//<abs>/**)`. A read path inside a write path is not denied: the more specific write wins, so the read deny rule is left out.
   - **Absolute paths:** every scope path becomes absolute before reaching the CLI. A relative path is resolved against the effective target repo. An unresolvable path is a pre-flight error.
   - **Implementers:** `${target_repo}` binds to the worktree, and the worktree is always a write path.
   - **Run artifacts:** every team may read the run's whole artifacts root, so reviewers can read what producers wrote. A team writes only its own artifact folder, which the engine already adds.
5. **Remote git grant.**
   - **Without it,** these are denied by rule: `Bash(git push:*)`, `Bash(git fetch:*)`, `Bash(git pull:*)`, `Bash(gh:*)`.
   - **With it,** they are not denied, and in auto mode the classifier decides whether the task really asked for them.
   - **Always denied, grant or not:**
     - `Bash(git push --force:*)`, `Bash(git push -f:*)`, `Bash(git push --force-with-lease:*)`;
     - pushes naming `main` or `master` (`Bash(git push * main)`, `Bash(git push * master)` and their `:main` / `:master` refspec forms);
     - `Bash(gh pr merge:*)`, `Bash(gh repo create:*)`, `Bash(gh repo delete:*)`;
     - `Bash(git remote add:*)`.
   - **Prefix-matching gaps:** forms that dodge prefix matching (`git -C <dir> push`) are left to the classifier. In `acceptEdits` they are blocked anyway, because remote commands aren't read-only.
   - **Pre-flight error:** a team with the grant on a model without auto mode, because no classifier would judge intent.
6. **Plugins.**
   - **Declaring:** a team declares `plugins: [<name>]`, written as a plugin name or `name@marketplace`.
   - **Lookup:** the app looks each up in `~/.claude/plugins/installed_plugins.json` and uses its `installPath`. When that folder is missing, it falls back to the marketplace's directory source in `known_marketplaces.json`, which is the ddd-council case.
   - **Passing:** each resolved plugin becomes one `--plugin-dir=<path>`.
   - **Editor:** picking a skill through the existing autocomplete adds its plugin to the team.
   - **Pre-flight error:** a plugin that can't be resolved.
7. **Denials are reported, never fatal.**
   - **Where they come from:** `result.permission_denials`, joined with `system/permission_denied` events to tag each one `rule` or `classifier`. They are collected per invocation.
   - **Where they show:** a badge with the count on the task card, the full list in the task's log, and a nullable `permission_denials` JSON column on `invocation_audit`.
   - The run's outcome is unchanged.
8. **The `sandbox-exec` layer is deleted**, along with the network-access help text and the experimental note in the glossary.
9. **Template scopes** for `ddd-spec-plan-impl`:

   | Team | Reads | Writes | Grants | Plugins |
   |---|---|---|---|---|
   | research | `${target_repo}` | own artifacts | — | — |
   | spec-writers | `${target_repo}` | own artifacts | — | superpowers |
   | spec-reviewers | `${target_repo}` | own artifacts | — | — |
   | plan-writers | `${target_repo}` | own artifacts | — | superpowers |
   | plan-reviewers | `${target_repo}` | own artifacts | — | ddd-council |
   | implementers | — | worktree, own artifacts | `Bash`, `Agent`, Remote git | superpowers |
   | code-reviewers | worktree | own artifacts | `Bash(git diff:*)`, `Bash(git log:*)`, Remote git | — |

   All teams also read the run artifacts (decision 4). No team gets web access by default.

## Architecture

### Shared kernel (`agent_bus_core`)

```rust
pub enum ToolGrant { Bash, BashPattern(String), Agent, WebFetch, WebSearch, RemoteGit }

pub enum PermissionMode { Auto, AcceptEdits }

pub struct PermissionDenial {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub source: DenialSource,
}

pub enum DenialSource { Rule, Classifier }
```

`RunnerConfigProblem` (from the Effort spec) gains `PluginNotFound { name }`, `PathUnresolvable { pattern }` and `RemoteGitWithoutAutoMode`.

### Pipeline Authoring

- `Scope` keeps `reads` and `writes`. `tools: Vec<String>` becomes `grants: Vec<ToolGrant>`, and `plugins: Vec<String>` is added.
- YAML: `grants: [bash, agent, remote-git, "bash(git diff:*)"]`, `plugins: [superpowers]`.
  - Legacy `tools:` entries are read as `BashPattern` when they look like `Bash(...)`. `Bash` reads as `Bash`, `Agent`/`WebFetch`/`WebSearch` map to their grants, and anything else is dropped.
  - `SCHEMA_VERSION` is unchanged.
- The seed template sets the scopes in decision 9.

### Workspace

- `resolve` makes every pattern absolute against the effective target repo.
- `${target_repo}` binds to the task's worktree for implementers.
- The plugin resolver lives here: it reads `installed_plugins.json` and `known_marketplaces.json` under the Claude config root and returns `Result<PathBuf, PluginNotFound>`.

### Runners (ACL)

- `scope.rs` `build_settings` produces:
  - `permissions.allow`: the Bash patterns;
  - `permissions.deny`: the hard limits (decision 5), the remote-git denies when the grant is absent, and the read-path Edit/Write denies;
  - `autoMode.environment` (decision 2).

  `sandbox_profile` and `command::sandbox_wrap` are deleted.
- `command.rs` worker argv:
  - adds `--setting-sources=`, `--strict-mcp-config`, `--permission-prompts none`;
  - adds `--permission-mode <auto|acceptEdits>`, one `--plugin-dir=` per plugin, and `--disallowed-tools=` with the tools not granted;
  - puts every `--add-dir` in the `=` form.
- The spawner sets stdin to null.
- `StreamAccumulator`:
  - records `init.permissionMode`;
  - collects `system/permission_denied` events;
  - reads `result.permission_denials`;
  - builds `Vec<PermissionDenial>` on `RunnerOutput`.

  A mode mismatch becomes `RunnerError::PermissionModeMismatch`.

### Runtime

- **Pre-flight** (the Effort spec's check) adds plugin resolution, path resolution and the Remote-git-on-non-auto-model check.
- **Invocation:**
  - The engine picks the `PermissionMode` from the model list.
  - It looks up repo visibility once per run.
  - It writes denials to the audit row.
  - It emits them on the task's log stream.

### Composition root (app)

- The next free migration (after the Effort spec's `018`) adds `ALTER TABLE invocation_audit ADD COLUMN permission_denials TEXT;`.
- It also exposes the plugin list (installed plus directory marketplaces) for the editor.

### Frontend

- `NodeDrawer.tsx` Scope fieldset:
  - **Reads and Writes:** unchanged pickers.
  - **Grants:** checkboxes for Bash (all), Agent, WebFetch, WebSearch and Remote git, plus a list of Bash patterns.
  - **Plugins:** a multi-select over the installed plugins.
  - **Autocomplete:** a skill picked in the prompt adds its plugin.
- **Help text:** explains removed versus denied, and that Remote git needs an auto-mode model.
- **Task card:** a denial badge (count). The task log lists each denial with its tool, input and `rule` / `classifier` tag.

## Data flow

1. Run start → pre-flight: models and effort, plugins, paths, Remote git against the model's mode → abort with problems, or start. The repo's visibility is looked up once.
2. Each worker invocation → effective scope → settings file (allow, deny, `autoMode`) plus argv (isolation, mode, plugins, disallowed tools, add-dirs) → spawn with null stdin.
3. Stream → `init.permissionMode` checked → denials collected → result.
4. Settle → denials go to the audit row, the task card badge and the log. The outcome is unaffected.

## Error handling

- **Mode mismatch:** an operational failure (retry, then needs-human), audited as `error:permission-mode-mismatch`.
- **Plugin, path or Remote git problem:** a pre-flight error naming the team. Nothing is spawned.
- **Visibility lookup fails** (no `gh`, no remote, not authenticated): the line is left out of `autoMode.environment`, and the run continues.
- **Classifier blocks a needed action:** reported as a denial. The worker usually works around it or explains in its output, and the reviewer or human sees the badge.

## Testing

- **Settings:**
  - read paths produce Edit/Write denies;
  - write paths don't;
  - a read inside a write path isn't denied;
  - hard limits are always present;
  - remote-git denies are present only without the grant;
  - `autoMode.environment` includes `"$defaults"`.
- **Argv:**
  - every variadic flag uses the `=` form and the positional prompt survives;
  - `--disallowed-tools=` lists exactly the tools not granted;
  - Skill is never removed;
  - `--plugin-dir=` appears once per plugin;
  - the mode follows `supportsAutoMode`.
- **Accumulator:**
  - captured fixtures of a rule denial and a classifier block produce correctly tagged `PermissionDenial`s;
  - an `init` whose mode is `default` when `auto` was requested gives `PermissionModeMismatch`.
- **Resolution:**
  - relative paths become absolute against the target repo;
  - implementers' `${target_repo}` is the worktree;
  - plugin lookup works from `installPath` and from a directory marketplace, and a missing plugin gives `PluginNotFound`.
- **Pre-flight:** each new problem blocks the run and names the team.
- **Legacy `tools:`** maps to grants as specified.
- **Seed template:** the scopes match decision 9.
- **Frontend:** grant checkboxes; adding a plugin from autocomplete; the denial badge and log rendering.
- **Live check (manual):** an implementer in a scratch repo with a local bare `origin`, asked to push its branch, pushes it. The same team asked to force-push is denied by rule, and the denial shows on the card.

## Out of scope

- OS-level confinement (sandbox-exec, containers). The CLI's permission system is the boundary.
- Per-task allow rules generated from the task text (e.g. "push branch X"). Intent is left to the classifier.
- Prompt changes to the DDD template beyond its scopes. This is still fog on the map, which this spec partly clears.
- The `anthropic-api` runner, which ignores scope.
