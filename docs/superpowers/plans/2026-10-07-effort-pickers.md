# Effort Levels + Model/Effort Pickers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A team's Effort becomes a CLI effort level passed as `--effort` (or Default, no flag), and Model and Effort are chosen from pickers filled by the CLI's own model list, read for free from the `initialize` control request.

**Architecture:**
- The kernel gains `Effort { Default, Level(String) }` with hand-written serde, and the model-list types (`ModelOption`, `ModelList`, `ModelListSource`, `RunnerConfigProblem`) with the one validation rule `ModelList::check`. A `ModelSource` trait mirrors `UtilizationSource`.
- The Runners ACL is the only code that knows `initialize`: `model_query.rs` (parser + `ClaudeCliModelSource`, sharing one control-request spawn with `usage_query.rs`) and `curated_models.rs` (the built-in fallback).
- Runtime runs a pre-flight check over every team's effective runner config before a run is created, and the audit row records the effort used.
- The app owns one in-memory list (`Arc<RwLock<ModelList>>`), shared with Runtime. It serves the disk cache at boot, fetches live in the background, writes the cache and emits `model-list-updated`.
- The frontend replaces the curated `CLAUDE_MODELS` selector, custom-id input, Test probe and budget input with two `<select>`s over the list.

**Tech Stack:** Rust (Tauri 2, sqlx/SQLite, serde, serde_yaml), React 18 + TypeScript, vitest.

**Spec:** `docs/superpowers/specs/2026-10-07-effort-pickers-design.md`

## Global Constraints

- `initialize` request line, verbatim: `{"type":"control_request","request_id":"m1","request":{"subtype":"initialize"}}`
- `claude` args for control requests, verbatim: `-p --input-format stream-json --output-format stream-json --verbose --no-session-persistence --setting-sources= --strict-mcp-config`, cwd = system temp dir, own process group, 10s timeout, group kill.
- Effort YAML: a level is a bare scalar (`effort: high`); Default omits the key; any map (legacy `{mode: …}`) loads as Default; any other non-string value is a parse error. `SCHEMA_VERSION` unchanged.
- Worker and chat argv: `--effort <level>` for `Level`, no flag for Default. `--max-thinking-tokens` never appears.
- New teams and the `ddd-spec-plan-impl` seed: model `default` (`DEFAULT_TEAM_MODEL`), effort Default. `DEFAULT_MODEL` (`claude-opus-5-5`) stays for Design Session and God chat, which use `Effort::Level("high")`.
- Migration `018_invocation_effort.sql`: `ALTER TABLE invocation_audit ADD COLUMN effort TEXT;` (NULL = Default). Registered in both migration lists in `app/src/lib.rs`.
- Cache file: `model-list.json` in the app data dir.
- Event: `model-list-updated`. Commands: `model_list`, `refresh_model_list`.
- User-facing strings: `<value> — not available`; `Default`; source labels `live` / `cached` / `built-in`; PipelineView `default effort`.
- The fixture is a real `initialize` capture with `account.email` and `account.organization` redacted (the repo is public), and the operator-specific `commands`, `agents`, `user_output_styles_dir` and `pid` trimmed.
- **Plan-level ruling — Option<Effort> serialisation:** `TeamRunnerConfig.effort` and `PipelineDefaults.default_effort` skip serialising both `None` and `Some(Default)`, so Default always omits the key. A team therefore cannot pin Default over a pipeline-level `default_effort` level; the app never writes pipeline defaults, so this only affects hand-written YAML.
- **Plan-level ruling — pre-flight scope:** only `claude-cli` teams are checked; the model list is the CLI's, and the hidden `anthropic-api` runner takes API model ids.
- **Plan-level ruling — stale cache:** if the cached `cli_version` differs from the installed one, the cache is still served (source `cached`) until the live fetch returns; a failed live fetch keeps it (the spec's "keep the current list" rule).
- Commands, from the repo root: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`, `npx vitest run`, `npx tsc --noEmit`.
- Code comments state the rule, never when or who decided it.

## Review Focus

1. **A legacy pipeline YAML with every old effort shape** (`{mode: off}`, `{mode: standard}`, `{mode: custom, budget_tokens: 16000}`, flow style `{ mode: standard }`) must still open, with Default effort, and re-save without an `effort` key. Pinned in Task 2.
2. **A `claude` that prints `control_response` and then keeps running** (stdout held open by a child) must return the list as soon as the line arrives, not wait for the 10s timeout. Pinned in Task 3.
3. **The model picker on a pipeline whose saved model is not in the list** must show `<value> — not available` as the selected option, not silently show the first model (which would make the picker lie about what will run). Pinned in Task 7.
4. **Switching from a model with `xhigh` to `claude-opus-4-6` with effort `xhigh`** snaps effort to Default and shows the note; switching to a model that *does* support the level keeps it and shows no note. Pinned in Task 6 (pure) and Task 7 (drawer).
5. **An unreadable or corrupt `model-list.json`** (truncated write, old shape) must be ignored, falling back to the built-in list, never crash boot. Pinned in Task 5.

---

### Task 1: Kernel `Effort`, model-list types and `ModelList::check`

**Files:**
- Create: `src-tauri/agent_bus_core/src/model_list.rs`
- Modify: `src-tauri/agent_bus_core/src/runner.rs` (add `Effort`, `DEFAULT_TEAM_MODEL`; `EffortMode` stays until Task 2)
- Modify: `src-tauri/agent_bus_core/src/lib.rs` (`pub mod model_list; pub use model_list::*;`)

**Interfaces:**
- Produces:
  - `pub enum Effort { Default, Level(String) }` — `Clone, Debug, PartialEq, Eq, Default(=Default)`; `fn is_default(&self) -> bool`; `fn level(&self) -> Option<&str>`; hand-written `Serialize`/`Deserialize`.
  - `pub fn effort_is_unset(e: &Option<Effort>) -> bool` — for `skip_serializing_if` on `Option<Effort>` fields.
  - `pub const DEFAULT_TEAM_MODEL: &str = "default";`
  - `ModelOption { value, resolved_model: Option<String>, display_name, description: Option<String>, effort_levels: Vec<String> }`
  - `ModelListSource { Live, Cached, Curated }` serialised lowercase.
  - `ModelList { models, source, cli_version: Option<String> }` with `fn find(&self, value: &str) -> Option<&ModelOption>` and `fn check(&self, model: &str, effort: &Effort) -> Result<(), RunnerConfigProblem>`.
  - `RunnerConfigProblem { ModelNotAvailable, EffortNotSupported { level: String } }` — serialised tagged on `kind`, snake_case.
  - `trait ModelSource: Send + Sync { fn fetch(&self) -> Result<ModelList, String>; }`

- [ ] **Step 1: Write the failing tests** — in `runner.rs` tests:

```rust
#[test]
fn effort_level_serialises_as_a_bare_string() {
    assert_eq!(serde_json::to_value(Effort::Level("high".into())).unwrap(), serde_json::json!("high"));
}

#[test]
fn effort_deserialises_a_string_as_a_level() {
    let e: Effort = serde_json::from_str("\"xhigh\"").unwrap();
    assert_eq!(e, Effort::Level("xhigh".into()));
}

#[test]
fn every_legacy_mode_map_deserialises_as_default() {
    for legacy in [
        r#"{"mode":"off"}"#,
        r#"{"mode":"standard"}"#,
        r#"{"mode":"extended-low"}"#,
        r#"{"mode":"extended-high"}"#,
        r#"{"mode":"custom","budget_tokens":16000}"#,
        r#"{}"#,
    ] {
        let e: Effort = serde_json::from_str(legacy).unwrap();
        assert_eq!(e, Effort::Default, "{legacy}");
    }
}

#[test]
fn a_non_string_non_map_effort_is_an_error() {
    assert!(serde_json::from_str::<Effort>("42").is_err());
    assert!(serde_json::from_str::<Effort>("true").is_err());
    assert!(serde_json::from_str::<Effort>("[\"high\"]").is_err());
}

#[test]
fn a_struct_field_omits_default_and_reads_a_missing_key_as_default() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(default, skip_serializing_if = "Effort::is_default")]
        effort: Effort,
    }
    assert_eq!(serde_json::to_string(&Holder { effort: Effort::Default }).unwrap(), "{}");
    let h: Holder = serde_json::from_str("{}").unwrap();
    assert_eq!(h.effort, Effort::Default);
    let h: Holder = serde_json::from_str(r#"{"effort":"low"}"#).unwrap();
    assert_eq!(h.effort, Effort::Level("low".into()));
}

#[test]
fn effort_is_unset_covers_none_and_default() {
    assert!(effort_is_unset(&None));
    assert!(effort_is_unset(&Some(Effort::Default)));
    assert!(!effort_is_unset(&Some(Effort::Level("high".into()))));
}

#[test]
fn default_team_model_is_the_default_alias() {
    assert_eq!(DEFAULT_TEAM_MODEL, "default");
}
```

And in `model_list.rs` tests:

```rust
fn list() -> ModelList {
    ModelList {
        models: vec![
            ModelOption { value: "opus".into(), resolved_model: Some("claude-opus-5-5".into()), display_name: "Opus 5.5".into(), description: None, effort_levels: vec!["low".into(), "high".into(), "xhigh".into()] },
            ModelOption { value: "haiku".into(), resolved_model: Some("claude-haiku-4-5-20251001".into()), display_name: "Haiku 4.5".into(), description: None, effort_levels: vec![] },
        ],
        source: ModelListSource::Curated,
        cli_version: None,
    }
}

#[test]
fn an_unknown_model_is_not_available() {
    assert_eq!(list().check("claude-nope", &Effort::Default), Err(RunnerConfigProblem::ModelNotAvailable));
}

#[test]
fn a_level_the_model_lacks_is_not_supported() {
    assert_eq!(
        list().check("opus", &Effort::Level("max".into())),
        Err(RunnerConfigProblem::EffortNotSupported { level: "max".into() })
    );
}

#[test]
fn default_is_fine_on_a_model_without_effort_support() {
    assert_eq!(list().check("haiku", &Effort::Default), Ok(()));
    assert_eq!(
        list().check("haiku", &Effort::Level("low".into())),
        Err(RunnerConfigProblem::EffortNotSupported { level: "low".into() })
    );
}

#[test]
fn a_supported_level_is_valid() {
    assert_eq!(list().check("opus", &Effort::Level("xhigh".into())), Ok(()));
}

#[test]
fn model_list_json_shape_matches_ts() {
    let v = serde_json::to_value(list()).unwrap();
    assert_eq!(v["source"], "curated");
    let m = &v["models"][0];
    let mut keys: Vec<_> = m.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["description", "display_name", "effort_levels", "resolved_model", "value"]);
    assert!(v.as_object().unwrap().contains_key("cli_version"));
    assert_eq!(serde_json::to_value(ModelListSource::Live).unwrap(), "live");
    assert_eq!(serde_json::to_value(ModelListSource::Cached).unwrap(), "cached");
    let p = serde_json::to_value(RunnerConfigProblem::EffortNotSupported { level: "max".into() }).unwrap();
    assert_eq!(p, serde_json::json!({"kind": "effort_not_supported", "level": "max"}));
}

#[test]
fn model_source_is_object_safe() {
    struct Fixed;
    impl ModelSource for Fixed {
        fn fetch(&self) -> Result<ModelList, String> { Ok(list()) }
    }
    let s: Box<dyn ModelSource> = Box::new(Fixed);
    assert_eq!(s.fetch().unwrap().models.len(), 2);
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test --manifest-path src-tauri/Cargo.toml -p agent_bus_core` → compile errors (`Effort`, `ModelList` undefined).

- [ ] **Step 3: Implement** — in `runner.rs`:

```rust
/// The model new teams use: the CLI's `default` alias, which tracks the
/// recommended model.
pub const DEFAULT_TEAM_MODEL: &str = "default";

/// How hard the model reasons: a CLI effort level, or Default (no `--effort`
/// flag; the model's own default applies). Levels are open strings so a new CLI
/// level needs no app release; the model list decides which are valid.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Effort {
    #[default]
    Default,
    Level(String),
}

impl Effort {
    pub fn is_default(&self) -> bool { matches!(self, Effort::Default) }
    pub fn level(&self) -> Option<&str> {
        match self { Effort::Default => None, Effort::Level(l) => Some(l) }
    }
}

/// `skip_serializing_if` for optional effort fields: Default is never written.
pub fn effort_is_unset(e: &Option<Effort>) -> bool {
    e.as_ref().map_or(true, Effort::is_default)
}

impl Serialize for Effort {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Effort::Default => s.serialize_none(),
            Effort::Level(l) => s.serialize_str(l),
        }
    }
}

impl<'de> Deserialize<'de> for Effort {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Effort;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an effort level string")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Effort, E> {
                Ok(Effort::Level(v.to_string()))
            }
            // A map is the retired thinking-budget shape; it carries no level.
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Effort, A::Error> {
                while m.next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?.is_some() {}
                Ok(Effort::Default)
            }
        }
        d.deserialize_any(V)
    }
}
```

`model_list.rs`:

```rust
//! The models the installed CLI offers and the effort levels each supports.
//! The one source of truth for which Model and Effort combinations are valid.

use crate::Effort;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    /// What to pass as `--model`.
    pub value: String,
    pub resolved_model: Option<String>,
    pub display_name: String,
    pub description: Option<String>,
    /// Empty when the model takes no effort level.
    pub effort_levels: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelListSource { Live, Cached, Curated }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<ModelOption>,
    pub source: ModelListSource,
    pub cli_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunnerConfigProblem {
    ModelNotAvailable,
    EffortNotSupported { level: String },
}

impl ModelList {
    pub fn find(&self, value: &str) -> Option<&ModelOption> {
        self.models.iter().find(|m| m.value == value)
    }

    /// A model is valid if the list has it; a level is valid if that model
    /// supports it. Default is valid on any listed model.
    pub fn check(&self, model: &str, effort: &Effort) -> Result<(), RunnerConfigProblem> {
        let m = self.find(model).ok_or(RunnerConfigProblem::ModelNotAvailable)?;
        match effort.level() {
            Some(l) if !m.effort_levels.iter().any(|x| x == l) => {
                Err(RunnerConfigProblem::EffortNotSupported { level: l.to_string() })
            }
            _ => Ok(()),
        }
    }
}

/// Where the model list comes from. The Runners ACL implements it against the
/// CLI; tests use a fixed list.
pub trait ModelSource: Send + Sync {
    fn fetch(&self) -> Result<ModelList, String>;
}
```

- [ ] **Step 4: Run** `cargo test --manifest-path src-tauri/Cargo.toml -p agent_bus_core` → PASS.
- [ ] **Step 5: Commit** `feat(core): Effort level type and model list with check`.

---

### Task 2: `Effort` replaces `EffortMode` end to end

One type swap across every consumer, so the workspace compiles at the end of the task.

**Files:**
- Modify: `src-tauri/agent_bus_core/src/runner.rs` (delete `EffortMode` + its tests), `src-tauri/agent_bus_core/src/contract_tests.rs`
- Modify: `src-tauri/pipeline/src/{model,resolve,draft,parse,store,validate,contract_tests,design_session,seed_template}.rs`
- Modify: `src-tauri/runners/src/{output,command,claude_cli,fake,anthropic_api}.rs`
- Modify: `src-tauri/llm_chat/src/{chat,command,claude_cli,fake,anthropic_api}.rs`
- Modify: `src-tauri/runtime/src/engine.rs`
- Modify: `src-tauri/app/src/{lib,pipeline_activator}.rs`

**Interfaces:**
- Consumes: `Effort`, `effort_is_unset`, `DEFAULT_TEAM_MODEL` (Task 1).
- Produces:
  - `RunnerConfig.effort: Effort` with `#[serde(default, skip_serializing_if = "Effort::is_default")]`.
  - `TeamRunnerConfig.effort: Option<Effort>` and `PipelineDefaults.default_effort: Option<Effort>` with `#[serde(default, skip_serializing_if = "agent_bus_core::effort_is_unset")]`.
  - `InvocationRequest.effort: Effort` (replaces `thinking_budget`).
  - `ChatRequest.effort: Effort` (replaces `thinking_budget`).
  - `LlmEngine::new(.., model: String, effort: Effort)` and `AgenticChatEngine::new(.., model: String, effort: Effort)` in `app/src/lib.rs`.

- [ ] **Step 1: Write the failing tests.**

`runners/src/command.rs` — change `req()` to `effort: Effort::Level("high".into())` and replace the budget assertion in `builds_the_spec_command_line` with:

```rust
let effort_i = args.iter().position(|a| a == "--effort").unwrap();
assert_eq!(args[effort_i + 1], "high");
assert!(!args.iter().any(|a| a == "--max-thinking-tokens"));
```

plus:

```rust
#[test]
fn default_effort_passes_no_effort_flag() {
    let mut r = req();
    r.effort = Effort::Default;
    let args = build_args(&r);
    assert!(!args.iter().any(|a| a == "--effort"));
    assert!(!args.iter().any(|a| a == "--max-thinking-tokens"));
    assert_eq!(args.last().unwrap(), "Investigate topic X");
}
```

`llm_chat/src/command.rs` — the same two assertions on `build_chat_args` (`--effort high`; Default gives no flag; no `--max-thinking-tokens`; resume still precedes the message).

`runners/src/anthropic_api.rs` and `llm_chat/src/anthropic_api.rs` — replace the thinking tests with:

```rust
#[test]
fn never_sends_a_thinking_field() {
    for effort in [Effort::Default, Effort::Level("max".into())] {
        let mut r = req();
        r.effort = effort;
        assert!(build_request_body(&r).get("thinking").is_none());
    }
}
```

(the chat variant calls its own body builder with no tools).

`pipeline/src/parse.rs` — `MINIMAL` uses `effort: high`; `parses_a_full_pipeline` asserts `runner.effort == Some(Effort::Level("high".into()))`; add:

```rust
#[test]
fn every_legacy_effort_map_loads_as_default() {
    for legacy in [
        "effort: { mode: off }",
        "effort: { mode: standard }",
        "effort:\n        mode: extended-high",
        "effort: { mode: custom, budget_tokens: 16000 }",
    ] {
        let yaml = format!(
            "id: x\nname: X\nteams:\n  - id: t\n    name: T\n    prompt: t.md\n    runner:\n      kind: claude-cli\n      model: m\n      {legacy}\n"
        );
        let p = parse_pipeline(&yaml).unwrap();
        assert_eq!(p.teams[0].runner.as_ref().unwrap().effort, Some(Effort::Default), "{legacy}");
    }
}

#[test]
fn a_numeric_effort_is_a_parse_error() {
    let yaml = "id: x\nname: X\nteams:\n  - id: t\n    name: T\n    prompt: t.md\n    runner: { kind: claude-cli, model: m, effort: 8192 }\n";
    assert!(parse_pipeline(yaml).is_err());
}

#[test]
fn a_legacy_file_resaves_without_an_effort_key() {
    let yaml = "id: x\nname: X\nteams:\n  - id: t\n    name: T\n    prompt: t.md\n    runner: { kind: claude-cli, model: m, effort: { mode: standard } }\n";
    let p = parse_pipeline(yaml).unwrap();
    let out = serde_yaml::to_string(&p).unwrap();
    assert!(!out.contains("effort"), "{out}");
    assert!(!out.contains("mode"), "{out}");
}
```

The V2 fixtures' `effort: { mode: standard }` stay as-is (they double as legacy coverage).

`pipeline/src/contract_tests.rs` — `full_runner()` uses `effort: Effort::Level("high".into())`; `runner_config_key_set_matches_ts` asserts `v["effort"] == "high"`; `runner_config_omits_api_key_env_when_none` uses `Effort::Default` and asserts the key set is `["kind", "model"]` (Default omits `effort`); `pipeline_defaults_key_appears_when_present` uses `default_effort: Some(Effort::Level("low".into()))`. `agent_bus_core/src/contract_tests.rs` replaces `effort_mode_matches_ts_discriminated_union` with:

```rust
/// Locks `src/ipc/pipeline.ts` `type Effort = string`: a level is a bare string.
#[test]
fn effort_level_matches_ts_string() {
    assert_eq!(serde_json::to_value(Effort::Level("max".into())).unwrap(), json!("max"));
}
```

`pipeline/src/resolve.rs` — the fallback test asserts `er.effort == Effort::Default`; inherited/defaults tests use `Effort::Level("high".into())`.

`pipeline/src/draft.rs`:

```rust
#[test]
fn new_team_defaults_to_the_default_alias_and_default_effort() {
    let t = DraftTeam::new("research", "Research");
    assert_eq!(t.runner.model, agent_bus_core::DEFAULT_TEAM_MODEL);
    assert_eq!(t.runner.effort, agent_bus_core::Effort::Default);
}
```

(replacing `new_team_defaults_to_the_shared_default_model` and the effort line of `draft_team_new_has_sane_defaults`).

`pipeline/src/seed_template.rs`:

```rust
#[test]
fn every_seed_team_uses_the_default_alias_and_default_effort() {
    let d = seed_template("ddd-spec-plan-impl").unwrap();
    assert!(!d.teams.is_empty());
    for t in &d.teams {
        assert_eq!(t.runner.model, "default", "{}", t.id);
        assert_eq!(t.runner.effort, agent_bus_core::Effort::Default, "{}", t.id);
    }
}
```

`pipeline/src/design_session.rs` — next to the existing `DEFAULT_MODEL` assertion (line ~600), assert `runner.received.lock().unwrap()[0].effort == Effort::Level("high".into())` on both the prose and structured paths.

- [ ] **Step 2: Run** `cargo test --manifest-path src-tauri/Cargo.toml --workspace` → compile errors on `EffortMode` / `thinking_budget`.

- [ ] **Step 3: Implement.**
  - Delete `EffortMode` and its two tests from `runner.rs`; update the `DEFAULT_MODEL` doc: "The model Design Session and God chat turns use. Teams default to `DEFAULT_TEAM_MODEL`."
  - `pipeline/src/model.rs`: field types and attributes as in **Interfaces**; delete `fn default_effort`; `effective_runner` uses `tr.effort.clone().expect(..)`; fix the test fixtures (`Effort::Level("high".into())` where they used `ExtendedHigh`, `Effort::Default` for `Standard`).
  - `pipeline/src/resolve.rs`: `d.default_effort.clone()` and `effort: team.effort.clone().or(de).or(Some(Effort::Default))`; doc line "Base fallbacks (claude-cli / Default effort)".
  - `pipeline/src/draft.rs`: `DraftTeam::new` uses `model: agent_bus_core::DEFAULT_TEAM_MODEL.into(), effort: Effort::Default`; doc "(claude-cli, the `default` model alias, Default effort, 1/1 workers …)".
  - `pipeline/src/design_session.rs` (both `ChatRequest`s): `effort: Effort::Level("high".into())`.
  - `runners/src/output.rs`: `pub effort: Effort` with doc "`--effort` level, or Default for no flag."
  - `runners/src/command.rs`:

    ```rust
    args.push("--model".into());
    args.push(req.model.clone());
    if let Some(level) = req.effort.level() {
        args.push("--effort".into());
        args.push(level.to_string());
    }
    ```

    The same shape in `llm_chat/src/command.rs` after `--model`.
  - `runners/src/anthropic_api.rs` / `llm_chat/src/anthropic_api.rs`: delete the `thinking` block; comment "The API runner sends no thinking field: effort levels are a CLI concept and this runner is not maintained beyond compiling."
  - `runtime/src/engine.rs`: `effort: effective.effort.clone()`; test fixture `effort: Some(Effort::Default)`.
  - `app/src/lib.rs`: `LlmEngine`/`AgenticChatEngine` hold `effort: Effort` and pass `effort: self.effort.clone()`; the god chat passes `agent_bus_core::Effort::Level("high".into())`; tests replace `8192` with `Effort::Level("high".into())`. `pipeline_activator.rs` test fixture uses `Effort::Default`.
  - All remaining `thinking_budget: N` struct literals in tests become `effort: Effort::Default` (or `Level("high")` where the test asserted 8192).

- [ ] **Step 4: Run** `cargo test --manifest-path src-tauri/Cargo.toml --workspace` → PASS. `grep -rn "EffortMode\|thinking_budget\|max-thinking-tokens" src-tauri --include=*.rs` → only `runners/src/probe.rs` (deleted in Task 5) may still mention `thinking_budget`; it must compile with `effort: Effort::Default`.
- [ ] **Step 5: Commit** `feat: Effort levels replace thinking-token budgets`.

---

### Task 3: Runners — `initialize` model query, fixture, curated list

**Files:**
- Create: `src-tauri/runners/src/control_request.rs` (the shared spawn)
- Create: `src-tauri/runners/src/model_query.rs`
- Create: `src-tauri/runners/src/curated_models.rs`
- Create: `src-tauri/runners/src/fixtures/initialize-sample.jsonl`
- Modify: `src-tauri/runners/src/usage_query.rs` (use the shared spawn), `src-tauri/runners/src/lib.rs`

**Interfaces:**
- Consumes: `ModelList`, `ModelOption`, `ModelListSource`, `ModelSource` (Task 1).
- Produces:
  - `control_request::CONTROL_ARGS: &[&str]` (the verbatim args).
  - `control_request::run(bin: &str, request_line: &str, timeout: Duration) -> Result<String, ControlError>` — returns stdout up to and including the first `control_response` line (or to EOF), killing the process group once that line arrives. `enum ControlError { Spawn(String), TimedOut }`.
  - `model_query::INITIALIZE_REQUEST`, `parse_initialize(stdout: &str) -> Result<Vec<ModelOption>, String>`, `parse_cli_version(stdout: &str) -> Option<String>`, `cli_version(bin: &str, timeout: Duration) -> Option<String>`, `ClaudeCliModelSource::{new, with_bin(bin, timeout)}` implementing `ModelSource` (source `Live`, `cli_version` from `claude --version`).
  - `curated_models::curated() -> ModelList` (source `Curated`, `cli_version: None`, 12 entries).

- [ ] **Step 1: Capture the fixture** (free; no model call):

```bash
echo '{"type":"control_request","request_id":"m1","request":{"subtype":"initialize"}}' | claude -p --input-format stream-json --output-format stream-json --verbose --no-session-persistence --setting-sources= --strict-mcp-config > "$SCRATCH/init.jsonl"
```

Write it to `src-tauri/runners/src/fixtures/initialize-sample.jsonl` as one line, with `account.email` → `"user@example.com"`, `account.organization` → `"example-org"`, `commands` → `[]`, `agents` → `[]`, `user_output_styles_dir` → `"/home/user/.claude/output-styles"`, `pid` → `12345`. `models` is kept byte-for-byte. Check: `grep -ci "splose\|healey\|/Users/" fixture` → 0.

- [ ] **Step 2: Write the failing tests** in `model_query.rs`:

```rust
const SAMPLE: &str = include_str!("fixtures/initialize-sample.jsonl");

#[test]
fn fixture_gives_twelve_models_with_their_levels() {
    let models = parse_initialize(SAMPLE).unwrap();
    assert_eq!(models.len(), 12);
    let by = |v: &str| models.iter().find(|m| m.value == v).unwrap().clone();
    assert_eq!(by("default").resolved_model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(by("default").effort_levels, ["low", "medium", "high", "xhigh", "max"]);
    assert!(by("haiku").effort_levels.is_empty());
    assert_eq!(by("claude-opus-4-6").effort_levels, ["low", "medium", "high", "max"]);
    assert_eq!(by("opus").display_name, "Opus 5.5");
    assert!(by("sonnet").description.is_some());
}

#[test]
fn fixture_is_redacted() {
    assert!(SAMPLE.contains("\"email\":\"user@example.com\""));
    assert!(!SAMPLE.contains("/Users/"));
}

#[test]
fn an_error_response_is_rejected() {
    let s = r#"{"type":"control_response","response":{"subtype":"error","request_id":"m1","error":"Unsupported control request subtype: initialize"}}"#;
    assert_eq!(parse_initialize(s).unwrap_err(), "Unsupported control request subtype: initialize");
}

#[test]
fn a_response_without_models_is_rejected() {
    let s = r#"{"type":"control_response","response":{"subtype":"success","request_id":"m1","response":{"commands":[]}}}"#;
    assert_eq!(parse_initialize(s).unwrap_err(), "initialize response has no models");
}

#[test]
fn no_control_response_is_an_error() {
    assert_eq!(parse_initialize("").unwrap_err(), "no initialize response from claude");
}

#[test]
fn version_line_is_parsed() {
    assert_eq!(parse_cli_version("2.1.292 (Claude Code)\n").as_deref(), Some("2.1.292"));
    assert_eq!(parse_cli_version(""), None);
}

#[test]
fn curated_list_matches_the_fixture() {
    let live = parse_initialize(SAMPLE).unwrap();
    let curated = crate::curated_models::curated();
    assert_eq!(curated.source, agent_bus_core::ModelListSource::Curated);
    let values = |ms: &[ModelOption]| ms.iter().map(|m| (m.value.clone(), m.effort_levels.clone())).collect::<Vec<_>>();
    assert_eq!(values(&curated.models), values(&live));
}
```

Unix fake-binary tests (reusing `usage_query`'s `script` helper pattern):

```rust
#[cfg(unix)]
#[test]
fn cli_source_reads_models_and_version_from_the_binary() {
    // The fake answers `--version` and the initialize request.
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/initialize-sample.jsonl");
    let (dir, bin) = script(&format!(
        "if [ \"$1\" = \"--version\" ]; then echo '9.9.9 (Claude Code)'; exit 0; fi\ncat > /dev/null\ncat '{}'",
        fixture.display()
    ));
    let list = ClaudeCliModelSource::with_bin(bin, Duration::from_secs(5)).fetch().unwrap();
    assert_eq!(list.source, agent_bus_core::ModelListSource::Live);
    assert_eq!(list.cli_version.as_deref(), Some("9.9.9"));
    assert_eq!(list.models.len(), 12);
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn cli_source_returns_on_the_control_response_without_waiting_for_exit() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/initialize-sample.jsonl");
    let (dir, bin) = script(&format!(
        "if [ \"$1\" = \"--version\" ]; then echo '9.9.9'; exit 0; fi\ncat '{}'\nsleep 30",
        fixture.display()
    ));
    let started = std::time::Instant::now();
    let list = ClaudeCliModelSource::with_bin(bin, Duration::from_secs(10)).fetch().unwrap();
    assert_eq!(list.models.len(), 12);
    assert!(started.elapsed() < Duration::from_secs(3), "took {:?}", started.elapsed());
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn cli_source_times_out_a_hung_binary() {
    let (dir, bin) = script("if [ \"$1\" = \"--version\" ]; then echo '9.9.9'; exit 0; fi\nsleep 30");
    let err = ClaudeCliModelSource::with_bin(bin, Duration::from_millis(300)).fetch().unwrap_err();
    assert_eq!(err, "model list query timed out");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn cli_source_sends_the_initialize_request_with_the_control_args() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/initialize-sample.jsonl");
    let (dir, bin) = script("");
    let record = dir.join("record");
    std::fs::write(&bin, format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '9.9.9'; exit 0; fi\nfor a in \"$@\"; do echo \"arg:$a\"; done > '{r}'\nhead -n 1 >> '{r}'\ncat '{f}'\n",
        r = record.display(), f = fixture.display()
    )).unwrap();
    ClaudeCliModelSource::with_bin(bin, Duration::from_secs(5)).fetch().unwrap();
    let rec = std::fs::read_to_string(&record).unwrap();
    let args: Vec<&str> = rec.lines().filter_map(|l| l.strip_prefix("arg:")).collect();
    assert_eq!(args, crate::control_request::CONTROL_ARGS);
    assert!(rec.contains(r#"{"type":"control_request","request_id":"m1","request":{"subtype":"initialize"}}"#));
    let _ = std::fs::remove_dir_all(dir);
}
```

Every existing `usage_query` test stays and must still pass unchanged (they pin the shared spawn: args, temp-dir cwd, timeout, group kill, large stdout, grandchild holding stdout, bounded reap).

- [ ] **Step 3: Run** `cargo test --manifest-path src-tauri/Cargo.toml -p runners` → compile errors.

- [ ] **Step 4: Implement.**
  - `control_request.rs`: move `ARGS` (as `pub const CONTROL_ARGS`), `kill_group` and the spawn/read/reap body out of `ClaudeCliUtilizationSource::fetch`. The reader thread reads with `BufRead::lines()` and sends each line over the channel; the caller loop appends lines and, on a line whose JSON `type` is `control_response`, kills the group, reaps (bounded), and returns. `Disconnected` (EOF) returns what was read. Deadline expiry kills, reaps and returns `ControlError::TimedOut`.
  - `usage_query.rs`: `fetch` becomes `control_request::run(&self.bin, GET_USAGE_REQUEST, self.timeout)` mapping `TimedOut` → `"usage query timed out"` and `Spawn(e)` → `format!("could not start claude: {e}")`, then `parse_get_usage`.
  - `model_query.rs`: `INITIALIZE_REQUEST` (verbatim + `\n`), `parse_initialize` (first `control_response` line; `subtype == "error"` → its `error` text; `response.response.models` not an array → `"initialize response has no models"`; each entry: `value` and `displayName` required strings (skip an entry missing either), `resolvedModel`/`description` optional, `supportedEffortLevels` optional string array → empty), `parse_cli_version` (first whitespace token of the first line), `cli_version(bin, timeout)` (spawn `bin --version` with stdout piped and stdin null, poll `try_wait` until the deadline, kill on expiry → `None`), `ClaudeCliModelSource::fetch` (`cli_version` then `control_request::run`; `TimedOut` → `"model list query timed out"`).
  - `curated_models.rs`: the 12 fixture entries as Rust literals (value, resolved model, display name, description, levels), doc: "The built-in fallback when the CLI cannot be queried and nothing is cached. Mirrors a real `initialize` capture; the live list always wins."
  - `lib.rs`: `pub mod control_request; pub mod curated_models; pub mod model_query;`.

- [ ] **Step 5: Run** `cargo test --manifest-path src-tauri/Cargo.toml -p runners` → PASS.
- [ ] **Step 6: Commit** `feat(runners): read the model list from initialize`.

---

### Task 4: Audit records the effort used

**Files:**
- Create: `src-tauri/app/migrations/018_invocation_effort.sql`
- Modify: `src-tauri/app/src/lib.rs` (both migration lists)
- Modify: `src-tauri/runtime/src/invocation_audit.rs`, `src-tauri/runtime/src/engine.rs`, test pools in `src-tauri/runtime/src/api.rs` and any other test that runs migration 007 then `record_start`

**Interfaces:**
- Produces: `InvocationAuditStore::record_start(task_id, team_id, model, effort: Option<&str>, attempts, started_at)`; `InvocationAudit.effort: Option<String>`.

- [ ] **Step 1: Write the failing test** in `invocation_audit.rs` (its `fresh_pool` also runs `018_invocation_effort.sql`):

```rust
#[tokio::test]
async fn start_records_the_effort_level_and_null_for_default() {
    let store = InvocationAuditStore::new(fresh_pool().await);
    let a = store.record_start("T-1", "research", "opus", Some("low"), 1, 1000).await.unwrap();
    let b = store.record_start("T-1", "research", "opus", None, 1, 1001).await.unwrap();
    assert_eq!(store.get(&a).await.unwrap().unwrap().effort.as_deref(), Some("low"));
    assert_eq!(store.get(&b).await.unwrap().unwrap().effort, None);
}
```

And in `app/src/lib.rs` tests:

```rust
#[tokio::test]
async fn migration_018_adds_a_nullable_effort_column() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let cols: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as("PRAGMA table_info(invocation_audit)").fetch_all(&pool).await.unwrap();
    let effort = cols.iter().find(|c| c.1 == "effort").expect("effort column");
    assert_eq!(effort.3, 0, "effort must be nullable");
}
```

- [ ] **Step 2: Run** → FAIL (arity / missing column).
- [ ] **Step 3: Implement.** Migration file:

```sql
-- 018_invocation_effort.sql — the effort level each invocation was started
-- with, taken from the argv the app built. NULL = Default (no --effort flag).
-- The stream never reports the effort used, so this is its only record.
ALTER TABLE invocation_audit ADD COLUMN effort TEXT;
```

Register `(18, include_str!(..))` in `run_migrations` and a `Migration { version: 18, description: "invocation effort level", .. }` in the plugin list. `record_start` binds `effort`; `SELECT` and `AuditRow` gain `effort` (last column); `row_to_audit` maps it. `engine.rs` passes `req.effort.level()`. Every test pool that runs 007 and then writes through `record_start` also runs 018.
- [ ] **Step 4: Run** `cargo test --manifest-path src-tauri/Cargo.toml --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(runtime): record the effort level on each invocation`.

---

### Task 5: Pre-flight check + app model list (cache, commands, event), probe removed

**Files:**
- Create: `src-tauri/runtime/src/preflight.rs`
- Create: `src-tauri/app/src/model_list_cache.rs`
- Modify: `src-tauri/runtime/src/{lib,api}.rs`
- Modify: `src-tauri/app/src/{lib,events}.rs`
- Delete: `src-tauri/runners/src/probe.rs` (+ `pub mod probe;`), the `test_model` command and its handler entry

**Interfaces:**
- Consumes: `ModelList::check`, `RunnerConfigProblem`, `ModelSource`, `ClaudeCliModelSource`, `curated()`.
- Produces:
  - `runtime::preflight::PreflightFailed { pub problems: Vec<(TeamId, RunnerConfigProblem)> }` with `fn describe(&self, pipeline: &Pipeline) -> String`.
  - `runtime::preflight::check_pipeline(pipeline: &Pipeline, list: &ModelList) -> Result<(), PreflightFailed>`.
  - `RuntimeState::with_model_list(self, Arc<std::sync::RwLock<ModelList>>) -> Self`; `start_run_inner` and `start_or_resume_run_inner` run the check first when a list is wired.
  - `app::model_list_cache::{load(path) -> Option<ModelList>, save(path, &ModelList) -> io::Result<()>, boot_list(cached: Option<ModelList>) -> ModelList}`.
  - Event `MODEL_LIST_UPDATED = "model-list-updated"`; commands `model_list() -> ModelList`, `refresh_model_list() -> ModelList`.

- [ ] **Step 1: Write the failing tests.** `preflight.rs`:

```rust
fn team(id: &str, name: &str, model: &str, effort: Effort) -> Team { /* runner: Some(TeamRunnerConfig { kind: Some(ClaudeCli), model: Some(model), effort: Some(effort), api_key_env: None }) */ }

#[test]
fn a_valid_pipeline_passes() {
    let p = pipeline(vec![team("a", "A", "opus", Effort::Level("high".into()))]);
    assert!(check_pipeline(&p, &curated()).is_ok());
}

#[test]
fn every_offending_team_is_named_with_its_reason() {
    let p = pipeline(vec![
        team("a", "Research", "claude-gone", Effort::Default),
        team("b", "Review", "claude-opus-4-6", Effort::Level("xhigh".into())),
        team("c", "Fine", "haiku", Effort::Default),
    ]);
    let err = check_pipeline(&p, &curated()).unwrap_err();
    assert_eq!(err.problems.len(), 2);
    let msg = err.describe(&p);
    assert!(msg.contains("Research: model 'claude-gone' is not available"), "{msg}");
    assert!(msg.contains("Review: effort 'xhigh' is not supported by claude-opus-4-6"), "{msg}");
}

#[test]
fn anthropic_api_teams_are_not_checked() {
    let mut t = team("a", "A", "claude-3-api-id", Effort::Default);
    t.runner.as_mut().unwrap().kind = Some(RunnerKind::AnthropicApi);
    assert!(check_pipeline(&pipeline(vec![t]), &curated()).is_ok());
}
```

`runtime/src/api.rs`:

```rust
#[tokio::test]
async fn start_run_with_an_unsupported_combination_aborts_before_creating_a_run() {
    let state = state_with_two_team_pipeline().await; // teams get runner haiku + Level("high")
    let list = Arc::new(std::sync::RwLock::new(runners::curated_models::curated()));
    let state = state.with_model_list(list);
    let err = start_or_resume_run_inner(&state).await.err().expect("must abort");
    assert!(err.contains("effort 'high' is not supported by haiku"), "{err}");
    assert!(state.runs.latest_active_for_project("proj").await.unwrap().is_none(), "no run created");
    assert!(state.tasks.list_all().await.unwrap().is_empty(), "no task claimed");
}
```

(If `runtime` does not depend on `runners`, build the list inline from `ModelOption`s instead; `list_all` is whatever the TaskStore's list method is named.)

`model_list_cache.rs`:

```rust
#[test]
fn save_then_load_marks_the_list_cached() {
    let dir = tempdir();
    let path = dir.join("model-list.json");
    let mut live = runners::curated_models::curated();
    live.source = ModelListSource::Live;
    live.cli_version = Some("2.1.292".into());
    save(&path, &live).unwrap();
    let back = load(&path).unwrap();
    assert_eq!(back.source, ModelListSource::Cached);
    assert_eq!(back.cli_version.as_deref(), Some("2.1.292"));
    assert_eq!(back.models, live.models);
}

#[test]
fn a_missing_or_corrupt_cache_is_ignored() {
    let dir = tempdir();
    assert!(load(&dir.join("absent.json")).is_none());
    std::fs::write(dir.join("bad.json"), "{\"models\": [tru").unwrap();
    assert!(load(&dir.join("bad.json")).is_none());
    std::fs::write(dir.join("old.json"), "{\"models\": 3}").unwrap();
    assert!(load(&dir.join("old.json")).is_none());
}

#[test]
fn boot_list_prefers_the_cache_then_the_curated_list() {
    let mut cached = runners::curated_models::curated();
    cached.source = ModelListSource::Cached;
    cached.models.truncate(2);
    assert_eq!(boot_list(Some(cached.clone())), cached);
    assert_eq!(boot_list(None).source, ModelListSource::Curated);
}

#[test]
fn an_empty_cached_list_falls_back_to_curated() {
    let empty = ModelList { models: vec![], source: ModelListSource::Cached, cli_version: None };
    assert_eq!(boot_list(Some(empty)).source, ModelListSource::Curated);
}
```

`events.rs` guard list gains `MODEL_LIST_UPDATED`.

- [ ] **Step 2: Run** → compile errors.
- [ ] **Step 3: Implement.**
  - `preflight.rs`: iterate `pipeline.teams`; skip a team whose `runner` is `None`, whose kind is not `ClaudeCli`, or whose model is `None` (validation already rejects those); collect `(TeamId(team.id), problem)`. `describe` → `"cannot start the run: " + problems.join("; ")`, each `"{name}: model '{m}' is not available"` or `"{name}: effort '{l}' is not supported by {m}"`, name = team name or id when empty.
  - `RuntimeState`: `model_list: Option<Arc<RwLock<ModelList>>>` (set by `with_model_list`); `fn preflight(&self) -> Result<(), String>` reads the list (poisoned lock → skip) and maps `PreflightFailed` through `describe`. Called first in `start_or_resume_run_inner` and `start_run_inner`.
  - `model_list_cache.rs`: `load` = read + `serde_json::from_str::<ModelList>`, set `source = Cached`; `save` writes a temp file then renames; `boot_list` = non-empty cache or `curated()`.
  - `app/src/lib.rs` setup: `let model_list = Arc::new(RwLock::new(boot_list(load(&data_dir.join("model-list.json")))));` → `RuntimeState::new(..).with_model_list(model_list.clone())`; `handle.manage(ModelListState { list, source: Arc<dyn ModelSource>, cache_path })`; spawn `refresh(&state, &handle)` at boot. `refresh` runs `source.fetch()` in `spawn_blocking`; on `Ok` with a non-empty list it stores the list, saves the cache (log a failed save) and emits `MODEL_LIST_UPDATED`; on `Err` it logs `app: model list query failed: {e}` and keeps the current list. Both commands return the current list.
  - Delete `probe.rs`, the `test_model` command and handler entry.
- [ ] **Step 4: Run** `cargo test --manifest-path src-tauri/Cargo.toml --workspace` → PASS; `grep -rn "probe::\|test_model" src-tauri --include=*.rs` → none.
- [ ] **Step 5: Commit** `feat: pre-flight model/effort check and the app model list`.

---

### Task 6: Frontend IPC + draft — model list, `Effort` type, snap rule

**Files:**
- Modify: `src/ipc/models.ts`, `src/ipc/models.test.ts`, `src/ipc/pipeline.ts`, `src/ipc/events.ts`, `src/wizard/draft.ts`, `src/wizard/draft.test.ts`
- Create: `src/hooks/useModelList.ts`
- Delete: `src/ipc/runner.ts`

**Interfaces:**
- Produces (`src/ipc/models.ts`):
  - `type ModelListSource = "live" | "cached" | "curated"`; `interface ModelOption { value: string; resolved_model: string | null; display_name: string; description: string | null; effort_levels: string[] }`; `interface ModelList { models: ModelOption[]; source: ModelListSource; cli_version: string | null }`.
  - `DEFAULT_TEAM_MODEL = "default"`.
  - `getModelList(): Promise<ModelList>`, `refreshModelList(): Promise<ModelList>`, `onModelListUpdated(cb: (l: ModelList) => void): Promise<() => void>` (listens, then re-reads via `getModelList`).
  - `findModel(list, value): ModelOption | undefined`; `modelLabel(m): string` (`"opus → claude-opus-5-5"` when `resolved_model` differs from `value`, else `value`); `sourceLabel(s): "live" | "cached" | "built-in"`; `supportsEffort(list, model, level): boolean`.
- `src/ipc/pipeline.ts`: `export type Effort = string;` `RunnerConfig.effort?: Effort` (absent = Default); same for `TeamRunnerConfig.effort` and `PipelineDefaults.default_effort`.
- `src/wizard/draft.ts`: `setTeamModel(d, id, model, list: ModelList | null): { draft: DraftPipeline; snapped: string | null }` (the dropped level, or null); `setTeamEffort(d, id, effort: Effort | undefined)`.
- `useModelList(): { list: ModelList | null; refreshing: boolean; refresh: () => Promise<void> }`.

- [ ] **Step 1: Write the failing tests.** `models.test.ts` (replace the CLAUDE_MODELS tests):

```ts
const LIST: ModelList = {
  source: "live",
  cli_version: "2.1.292",
  models: [
    { value: "opus", resolved_model: "claude-opus-5-5", display_name: "Opus 5.5", description: null, effort_levels: ["low", "high", "xhigh"] },
    { value: "claude-opus-4-6", resolved_model: "claude-opus-4-6", display_name: "Opus 4.6", description: null, effort_levels: ["low", "high"] },
    { value: "haiku", resolved_model: "claude-haiku-4-5-20251001", display_name: "Haiku 4.5", description: null, effort_levels: [] },
  ],
};

it("labels an alias with what it resolves to and a pinned id by itself", () => {
  expect(modelLabel(LIST.models[0])).toBe("opus → claude-opus-5-5");
  expect(modelLabel(LIST.models[1])).toBe("claude-opus-4-6");
});
it("shows the curated source as built-in", () => {
  expect(sourceLabel("curated")).toBe("built-in");
  expect(sourceLabel("live")).toBe("live");
});
it("supportsEffort reads the model's levels", () => {
  expect(supportsEffort(LIST, "opus", "xhigh")).toBe(true);
  expect(supportsEffort(LIST, "claude-opus-4-6", "xhigh")).toBe(false);
  expect(supportsEffort(LIST, "missing", "low")).toBe(false);
});
it("getModelList / refreshModelList call their commands", async () => {
  invokeMock.mockResolvedValueOnce(LIST);
  expect(await getModelList()).toEqual(LIST);
  expect(invokeMock).toHaveBeenCalledWith("model_list");
  invokeMock.mockResolvedValueOnce(LIST);
  await refreshModelList();
  expect(invokeMock).toHaveBeenCalledWith("refresh_model_list");
});
it("new teams default to the default alias", () => {
  expect(DEFAULT_TEAM_MODEL).toBe("default");
});
```

(`invokeMock` via `vi.mock("@tauri-apps/api/core", …)` as in the other IPC tests.)

`draft.test.ts`:

```ts
it("addTeam defaults to the default alias with Default effort", () => {
  const d = addTeam(emptyDraft(), "research", "Research");
  expect(d.teams[0].runner.model).toBe("default");
  expect(d.teams[0].runner.effort).toBeUndefined();
});
it("switching to a model without the saved level snaps effort to Default", () => {
  const base = setTeamEffort(setTeamModel(addTeam(emptyDraft(), "r", "R"), "r", "opus", LIST).draft, "r", "xhigh");
  const { draft, snapped } = setTeamModel(base, "r", "claude-opus-4-6", LIST);
  expect(draft.teams[0].runner.model).toBe("claude-opus-4-6");
  expect(draft.teams[0].runner.effort).toBeUndefined();
  expect(snapped).toBe("xhigh");
});
it("switching to a model that supports the level keeps it", () => {
  const base = setTeamEffort(addTeam(emptyDraft(), "r", "R"), "r", "high");
  const { draft, snapped } = setTeamModel(base, "r", "claude-opus-4-6", LIST);
  expect(draft.teams[0].runner.effort).toBe("high");
  expect(snapped).toBeNull();
});
it("a model without effort support snaps any level", () => {
  const base = setTeamEffort(addTeam(emptyDraft(), "r", "R"), "r", "low");
  expect(setTeamModel(base, "r", "haiku", LIST).snapped).toBe("low");
});
it("without a list nothing snaps", () => {
  const base = setTeamEffort(addTeam(emptyDraft(), "r", "R"), "r", "low");
  expect(setTeamModel(base, "r", "haiku", null).snapped).toBeNull();
});
it("setTeamEffort(undefined) clears the level", () => {
  const d = setTeamEffort(setTeamEffort(addTeam(emptyDraft(), "r", "R"), "r", "low"), "r", undefined);
  expect("effort" in d.teams[0].runner).toBe(false);
});
```

- [ ] **Step 2: Run** `npx vitest run src/ipc/models.test.ts src/wizard/draft.test.ts` → FAIL.
- [ ] **Step 3: Implement** the interfaces above. `defaultTeam` uses `runner: { kind: "claude-cli", model: DEFAULT_TEAM_MODEL, api_key_env: null }`. `setTeamEffort(…, undefined)` deletes the `effort` key (so it is never sent as `null`, which the backend rejects). `events.ts` gains `modelListUpdated: "model-list-updated"`. `useModelList` loads on mount, subscribes with `onModelListUpdated` (unsubscribing safely on unmount like `useRuntimeEvents`), and `refresh` sets `refreshing` around `refreshModelList`. Delete `CLAUDE_MODELS`, `isKnownModel`, `ClaudeModel`, the TS `DEFAULT_MODEL`, `src/ipc/runner.ts`.
- [ ] **Step 4: Run** the two test files → PASS (the drawer/PipelineView compile errors are fixed in Task 7).
- [ ] **Step 5: Commit** `feat(ui): model list IPC and the effort snap rule`.

---

### Task 7: Pickers in the drawer, PipelineView label, run-start error

**Files:**
- Modify: `src/wizard/canvas/NodeDrawer.tsx`, `src/wizard/canvas/NodeDrawer.test.tsx`
- Modify: `src/components/PipelineView.tsx`, `src/components/PipelineView.test.tsx`
- Modify: `src/App.tsx`, `src/App.test.tsx`

**Interfaces:**
- Consumes: everything Task 6 produces.

- [ ] **Step 1: Write the failing tests.** `NodeDrawer.test.tsx` mocks `../../ipc/models` keeping the pure helpers and stubbing `getModelList` (resolves `LIST` from Task 6 plus a `default` entry with all five levels), `refreshModelList`, `onModelListUpdated` (resolves a no-op). Replace the G6 describe block with:

```tsx
describe("NodeDrawer — model and effort pickers", () => {
  it("lists the CLI's models with alias rows showing their resolved model", async () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "opus → claude-opus-5-5" });
    expect(screen.getByRole("option", { name: "claude-opus-4-6" })).toBeInTheDocument();
    expect(screen.queryByLabelText(/model override/)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/test model/)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/budget for/)).not.toBeInTheDocument();
  });

  it("offers Default plus the selected model's levels", async () => {
    render(<NodeDrawer draft={withModel("claude-opus-4-6")} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "claude-opus-4-6" });
    const effort = screen.getByLabelText("effort for research");
    expect(within(effort).getAllByRole("option").map((o) => o.textContent)).toEqual(["Default", "low", "high"]);
  });

  it("a model without effort support offers only Default", async () => {
    render(<NodeDrawer draft={withModel("haiku")} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "haiku → claude-haiku-4-5-20251001" });
    const effort = screen.getByLabelText("effort for research");
    expect(within(effort).getAllByRole("option").map((o) => o.textContent)).toEqual(["Default"]);
  });

  it("switching to a model without the saved level snaps to Default and says so", async () => {
    function Host() {
      const [d, setD] = useState(() => setTeamEffort(withModel("opus"), "research", "xhigh"));
      return <NodeDrawer draft={d} selectedId="research" onChange={setD} onClose={() => {}} />;
    }
    render(<Host />);
    await screen.findByRole("option", { name: "claude-opus-4-6" });
    fireEvent.change(screen.getByLabelText("model for research"), { target: { value: "claude-opus-4-6" } });
    expect((screen.getByLabelText("effort for research") as HTMLSelectElement).value).toBe("");
    expect(screen.getByRole("status")).toHaveTextContent(/xhigh.*claude-opus-4-6.*Default/);
  });

  it("a saved model missing from the list shows as not available", async () => {
    render(<NodeDrawer draft={withModel("claude-gone")} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    const select = await screen.findByLabelText("model for research");
    await screen.findByRole("option", { name: "claude-gone — not available" });
    expect((select as HTMLSelectElement).value).toBe("claude-gone");
  });

  it("the refresh button refetches and shows the list's source", async () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    expect(await screen.findByText("live")).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("refresh model list"));
    await waitFor(() => expect(refreshMock).toHaveBeenCalled());
  });
});
```

`PipelineView.test.tsx`: the fixture team runs `effort: "high"` and renders `claude-opus-4-7 · high effort`; a second team without `effort` renders `default effort`.

`App.test.tsx`: when `start_run` rejects with `"cannot start the run: Review: effort 'xhigh' is not supported by claude-opus-4-6"`, clicking Start shows that text in a `role="alert"`.

- [ ] **Step 2: Run** `npx vitest run src/wizard/canvas src/components/PipelineView.test.tsx src/App.test.tsx` → FAIL.
- [ ] **Step 3: Implement.**
  - `NodeDrawer.tsx`: delete `EFFORT_PRESETS`, `effortFromSelect`, `ModelField`, the test-result helpers and the budget field; add `RunnerPickers({ draft, id, onChange })` using `useModelList()`:
    - Model `<select aria-label="model for {id}">`; when the saved value is not in the list, a first `<option value={value}>{value} — not available</option>`; then `list.models` as `<option value={m.value}>{modelLabel(m)}</option>`.
    - On change: `const { draft: next, snapped } = setTeamModel(draft, id, value, list); onChange(next); setNote(snapped ? \`${snapped} isn't supported by ${value}; effort reset to Default.\` : null);` — note rendered in `<span role="status">`.
    - Effort `<select aria-label="effort for {id}">`: `<option value="">Default</option>` + the selected model's levels; if the saved level is not among them, an extra `<option value={level}>{level} — not supported</option>` so the select shows the truth. Change → `setTeamEffort(draft, id, v || undefined)` and clears the note.
    - A row with `<Button size="sm" aria-label="refresh model list">` (label "Refresh" / "Refreshing…") and the source label (`sourceLabel(list.source)`) in muted text; while `list` is null the selects render the saved value only.
    - `HELP.runner`: "Which Claude runs this team and how hard it reasons. Models and effort levels come from your installed Claude CLI. An API-key env var name is only used by the anthropic-api runner."
  - `PipelineView.tsx`: drop `effortLabel`/`EffortMode`; render `{kind} · {model} · {effort ? `${effort} effort` : "default effort"} · workers …`.
  - `App.tsx`: `const [startError, setStartError] = useState<string | null>(null)`; clear it on press; `catch (e) { setStartError(String(e)); }`; render `<div role="alert">` with the message near the Start control, dismissed on the next press.
- [ ] **Step 4: Run** `npx vitest run` and `npx tsc --noEmit` → PASS / clean.
- [ ] **Step 5: Commit** `feat(ui): model and effort pickers`.

---

### Task 8: Whole-branch verification

- [ ] `cargo test --manifest-path src-tauri/Cargo.toml --workspace` → all pass; record counts.
- [ ] `npx vitest run` and `npx tsc --noEmit` → all pass / clean; record counts.
- [ ] `grep -rn "EffortMode\|max-thinking-tokens\|thinking_budget\|CLAUDE_MODELS\|isKnownModel\|testModel\|test_model" src src-tauri --include=*.rs --include=*.ts --include=*.tsx` → no hits.
- [ ] Fixture redaction check: `grep -ci "splose\|healey\|/Users/" src-tauri/runners/src/fixtures/initialize-sample.jsonl` → 0.
- [ ] DOMAIN.md: the Runners glossary already defines Model, Effort and Model list; update only the stale "effort (thinking budget)" expert vocabulary line and the "effort preset" wording in DraftPipeline if they contradict the build.
- [ ] Manual live check (spec): a one-team pipeline at `--effort low`, audit row reads `low`. This spends plan usage; skip it if no operator is present and say so in the PR.
