# Spec — Effort levels + model/effort pickers

*Design doc. Wayfinder map: "Map: runner capabilities from the current Claude CLI" (#3). Ticket: "Spec: Effort levels + model/effort pickers" (#6). Research: "Research: can a subscription claude CLI list available models and effort levels?" (#5, `docs/research/subscription-model-effort-listing.md` on `research/model-effort-listing`). A team's **Effort** becomes a CLI effort level passed as `--effort`, replacing the thinking-token budget. Model and Effort are chosen from pickers filled by the CLI's own model list. Terms are as defined in `DOMAIN.md` (Runners).*

## Why this exists

`EffortMode` is a thinking-token budget (`off` / `standard` 1024 / `extended-low` 8192 / `extended-high` 32000 / `custom`), passed as `--max-thinking-tokens`. Current models don't reason by a token budget. The CLI's replacement is `--effort low|medium|high|xhigh|max`, and the levels a model supports vary by model.

The model field is free text, chosen from a hardcoded `CLAUDE_MODELS` list or typed in as a custom id. It is checked by a "Test" probe, which makes a real `--print` call and spends plan usage.

The CLI can report both lists for free. Sending the SDK control request `initialize` to `claude -p --input-format stream-json --output-format stream-json --verbose` returns `models[]`. Each entry has the `value` to pass as `--model`, its `resolvedModel` and `displayName`, and, where effort applies, `supportedEffortLevels`. The request calls no model. The CLI never validates `--effort`: an unknown level only prints a warning, and an unsupported level is silently ignored. So the app must do the validating. Verified on `claude` 2.1.289.

## Decisions

1. **Effort is a CLI effort level or Default.** `Effort = Default | Level(String)`.
   - Default passes no `--effort` flag; the model's own default applies.
   - `--effort <level>` replaces `--max-thinking-tokens` in every `claude-cli` argv.
   - The token budget and `custom` are removed.
2. **Effort is an open value, not a closed enum.** Levels come from the CLI at runtime, so a new CLI level needs no app release and never breaks parsing. Validation happens against the model list (decision 6), never at parse time.
3. **Saved shape: a plain scalar.** A level is written as `effort: high`; Default omits the key.
   - Any legacy `effort: {mode: …}` map loads as Default, whatever its mode. Old pipelines carry no faithful mapping worth keeping.
   - `SCHEMA_VERSION` is unchanged (additive tolerance, as for earlier fields). A legacy file is rewritten in the new shape on its next save.
4. **The model list comes from `initialize`.**
   - It is fetched at app start and on a manual refresh in the Runner fieldset. It is cached to disk, keyed by `claude --version`.
   - If the query fails, the cache is used. With no cache, a curated built-in list is used: the 12 entries the research recorded, with their effort levels.
   - The list is the single source of truth for which models and levels are valid.
5. **Pickers.**
   - **Model** shows the list's entries as given, aliases (`default`, `opus`, `sonnet`, `fable`) and pinned ids alike. Aliases show their `resolvedModel` alongside. The picker saves `value`.
   - **Effort** offers Default plus the selected model's `supportedEffortLevels`. A model without effort support (e.g. `haiku`) offers only Default.
   - Switching to a model that doesn't support the saved level snaps Effort to Default and shows an inline note.
   - A saved model missing from the list shows as "<value> — not available".
6. **Pre-flight check at run start.** Any team whose model is not in the list, or whose effort level that model doesn't support, blocks the run. The message names each offending team and the reason. The check uses whatever list is current (live, cached or curated).
7. **Defaults.** New teams and the bundled `ddd-spec-plan-impl` template get model `default` and effort Default. Role has no effect on runner config.
8. **Other invocations.** Design Session generate (both paths) and the God terminal chat keep their hardcoded model and move from 8192 thinking tokens to `--effort high`. They get no picker.
9. **Audit.** `invocation_audit` gains a nullable `effort` column, written from the argv the app built (NULL = Default). The stream never reports the effort used, so this is the only record of it.
10. **Removed:**
    - the "Test" probe (`probe.rs`, the `test_model` command, `testModel` in the IPC) and the custom-id input;
    - `CLAUDE_MODELS` / `isKnownModel` as the picker source (the curated list moves to the Runners ACL as the fallback);
    - `EffortMode`, its token budgets and the "Budget (tokens)" input.
11. **`anthropic-api` runner:** only the changes needed to keep it compiling. It is hidden from the UI and not maintained by this map.

## Architecture

### Shared kernel (`agent_bus_core`)

`EffortMode` is replaced by:

```rust
pub enum Effort { Default, Level(String) }
```

Serde is hand-written:
- serialises Default as an absent field (`skip_serializing_if`) and `Level` as a bare string;
- deserialises a string as `Level` and a missing field as Default;
- deserialises any map (the legacy `{mode: …}` shape) as Default.

The model list types sit alongside it:

```rust
pub struct ModelOption {
    pub value: String,                         // what to pass as --model
    pub resolved_model: Option<String>,
    pub display_name: String,
    pub description: Option<String>,
    pub effort_levels: Vec<String>,            // empty = no effort support
}

pub enum ModelListSource { Live, Cached, Curated }

pub struct ModelList {
    pub models: Vec<ModelOption>,
    pub source: ModelListSource,
    pub cli_version: Option<String>,
}

impl ModelList {
    pub fn check(&self, model: &str, effort: &Effort) -> Result<(), RunnerConfigProblem>;
}

pub enum RunnerConfigProblem { ModelNotAvailable, EffortNotSupported { level: String } }
```

`check` is the one validation rule. The pre-flight check and the pickers both use it.

### Runners (ACL) — the only code that knows `initialize`

- `model_query.rs`: `ClaudeCliModelSource::fetch` sends `{"type":"control_request","request_id":"m1","request":{"subtype":"initialize"}}`.
  - It reuses the `usage_query.rs` spawn: the same args, temp-dir cwd, process group, 10s timeout and group kill.
  - It finishes on the `control_response` line.
  - `parse_initialize` maps `response.response.models[]` to `ModelOption`. A missing `supportedEffortLevels` becomes an empty list.
  - Fixture: a captured `initialize` sample in `fixtures/`.
- `curated_models.rs`: the fallback `ModelList` (source `Curated`).
- `command.rs` (worker) and `llm_chat/src/command.rs` (chat): emit `--effort <level>` for `Level`, nothing for Default. `--max-thinking-tokens` is removed.
- `anthropic_api.rs` (both): treat any Effort as no thinking field.

### Pipeline Authoring

- `RunnerConfig.effort`, `TeamRunnerConfig.effort` and `PipelineDefaults.default_effort` become `Effort` / `Option<Effort>`. The resolver falls back to Default instead of `Standard`.
- `DraftTeam::new` and the TS draft use model `default` and effort Default. A new `DEFAULT_TEAM_MODEL = "default"` seeds teams. `DEFAULT_MODEL` stays as the hardcoded model for Design Session and chat.
- The `parse → resolve → validate` load path is unchanged. Model and effort are not checked against the list at load, so a pipeline always opens.

### Runtime

- Before a run starts, the engine calls `ModelList::check` for every team's effective runner config. Any problem aborts the start with a `PreflightFailed { problems: Vec<(TeamId, RunnerConfigProblem)> }` error. No task is claimed.
- The worker pool writes `effort` to the `invocation_audit` row at invoke-start.

### Composition root (app)

- At startup, the app loads the cached list from `model-list.json` in the app data dir (not the project root; the list belongs to the installed CLI, not a project) and starts a live fetch. On success, it overwrites the cache and emits `model-list-updated`.
  - If the cached `cli_version` differs from the installed `claude --version`, the cache is used only until the live fetch returns.
- Commands:
  - `model_list() -> ModelList`: current in-memory list.
  - `refresh_model_list() -> ModelList`.
- Migration `018_invocation_effort.sql`: `ALTER TABLE invocation_audit ADD COLUMN effort TEXT;`.
- Design Session and God chat construction pass `Effort::Level("high")`.

### Frontend

- `src/ipc/models.ts`: `getModelList`, `refreshModelList`, and a listener for `model-list-updated`. `CLAUDE_MODELS` is removed.
- `NodeDrawer.tsx` Runner fieldset:
  - a Model `<select>` over the list, with alias rows labelled "opus → claude-opus-5-5";
  - an Effort `<select>` (Default + the model's levels);
  - a refresh button with the list's source ("live" / "cached" / "built-in").
  - The custom-id checkbox, the Test button and the budget input are removed.
- `setTeamModel` applies the snap-to-Default rule and returns whether it snapped, so the drawer can show the note.
- `PipelineView` shows `model · effort` (Default shown as "default effort").
- The run-start error lists the pre-flight problems by team name.

## Data flow

1. App start → cached list (if any) is served immediately → live `initialize` → cache written → `model-list-updated` → pickers re-render.
2. Edit a team → pickers read the current list → save writes `model: <value>` and `effort: <level>` (or no key).
3. Start a run → pre-flight check over the effective configs against the current list → either abort with problems, or start.
4. Each worker invocation → argv gets `--model <value>` and `--effort <level>` if set → audit row records `effort`.

## Error handling

- **Live fetch fails** (timeout, non-zero exit, no `control_response`, error subtype): keep the current list. Show its source ("cached" or "built-in") beside the refresh button. Log the error. Never block editing or running on a failed fetch.
- **Legacy or unreadable effort in YAML:** loads as Default, silently. A non-string, non-map value is still a parse error.
- **Model gone from the list:** the pipeline loads, the picker shows "not available", and the pre-flight check blocks the run until it's changed.
- **CLI accepts an effort it ignores:** prevented by the pre-flight check; the CLI's own warning is not relied on.

## Testing

- **`Effort` serde:**
  - a scalar round-trips;
  - Default omits the key;
  - a missing key gives Default;
  - every legacy `{mode: …}` shape gives Default;
  - the contract test is updated to the new shape.
- **`parse_initialize`:** the fixture gives the 12 entries with the right levels. `haiku` has an empty list; `opus-4-6` lacks `xhigh`. An error response and a missing `models` field are rejected.
- **`ModelList::check`:** unknown model; a level unsupported by the model; Default with a model that has no effort support; a valid level.
- **Argv:** `Level("high")` gives `--effort high`; Default gives no flag; `--max-thinking-tokens` never appears.
- **Pre-flight:** a run with an unsupported combination aborts before claiming any task and names the team.
- **Seed template:** every team has `default` / Default.
- **Frontend:**
  - switching model snaps an unsupported effort to Default and shows the note;
  - an unavailable saved model renders as "not available";
  - the effort picker offers only Default for a model without effort support.
- **Live check (manual):** run a one-team pipeline at `--effort low` and confirm the audit row reads `low`.

## Out of scope

- An app-level setting for the model or effort of Design Session and God chat (decision 8).
- Defaults by role (decision 7).
- Mapping legacy budgets to levels (decision 3).
- Filtering the list by subscription plan; the research did not verify whether `initialize` already does.
- The `anthropic-api` runner beyond compiling (decision 11).
