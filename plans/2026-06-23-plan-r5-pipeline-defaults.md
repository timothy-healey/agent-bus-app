# R5 — Pipeline-level runner/model defaults with team overrides — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a `Pipeline` declare `defaults` (default runner kind / model / effort) that teams inherit when they omit their own runner config, cutting config repetition, with a team's own config overriding the pipeline default.

**Architecture:** Additive, no schema-version bump. `Team.runner` becomes optional (`Option<RunnerConfig>`) and gains a partial-override sibling. Pipeline Authoring owns a **resolver** (`resolve.rs`) that materialises each team's *effective* `RunnerConfig` by overlaying team config on the pipeline default. Runtime never sees the defaults: the store resolves at load-time so Runtime keeps consuming an already-resolved effective `RunnerConfig`. Validation requires every team to end up with an effective runner.

**Tech Stack:** Rust (Cargo workspace under `src-tauri/`), serde / serde_yaml, thiserror, React+TS frontend (vitest).

---

## Decisions

- **DD1 — Shape of the default block.** `Pipeline.defaults: Option<PipelineDefaults>` where `PipelineDefaults { default_runner: Option<RunnerKind>, default_model: Option<String>, default_effort: Option<EffortMode> }`. **Recommended/chosen:** every field optional so a pipeline can supply just a model, just a runner, etc. Additive `#[serde(default, skip_serializing_if = "Option::is_none")]` — v2 pipelines with no `defaults` key load unchanged.
- **DD2 — Shape of the team override.** `Team.runner: Option<RunnerConfig>` is too coarse (a team that only wants to override the model would have to repeat kind+effort). **Recommended/chosen:** keep `Team.runner: Option<RunnerConfig>` for the *fully-specified* case AND add `Team.runner` as the existing required-fields struct made partial via a new `TeamRunnerOverride { kind?, model?, effort?, api_key_env? }`. To keep the change small and the contract stable we choose the **partial-override** approach: replace `Team.runner: RunnerConfig` with `Team.runner: Option<TeamRunnerConfig>`, where `TeamRunnerConfig` has all-optional `kind`/`model`/`effort` plus the existing optional `api_key_env`. The resolver fills gaps from `Pipeline.defaults`, then from hard-coded base defaults. This is the **single** override mechanism — no second redundant full-struct field.
- **DD3 — Where resolution lives.** A resolver in Pipeline Authoring (`pipeline/src/resolve.rs`). Runtime must NOT learn about defaults (DOMAIN.md: Authoring↔Runtime shared kernel; no resolution logic leaks into Runtime). **Chosen:** `PipelineStore::load` returns an already-resolved `Pipeline` (every `team.runner` is `Some(fully-specified RunnerConfig)`). Runtime consumes `team.runner` — see DD4 for how Runtime reads it without an `Option` papercut.
- **DD4 — Runtime's read of `team.runner`.** Runtime currently reads `team.runner.model` / `team.runner.effort` directly. After resolution `team.runner` is always `Some`. **Chosen:** add `Team::effective_runner(&self) -> &RunnerConfig` (panics with a clear message if `None`, which can only happen on an unresolved pipeline — a programmer error, never reachable for a stored/loaded pipeline because the store resolves + validates). Runtime calls `team.effective_runner()`. This keeps Runtime ignorant of defaults while giving it an infallible `RunnerConfig`.
- **DD5 — Validation.** `validate()` runs on the **resolved** pipeline and requires every team to have `runner: Some(_)` with a non-empty model (new error `TeamHasNoRunner`). The store resolves *then* validates, so a team that supplies neither its own model nor a pipeline default model fails validation with a clear message.
- **DD6 — No schema bump.** Additive only; `SCHEMA_VERSION` stays 2. Existing v1 + v2 pipelines (incl. wizard-produced ones, which always emit a full per-team runner) load unchanged because a full `TeamRunnerConfig` resolves to itself.
- **DD7 — Wizard / DraftPipeline.** The wizard already authors a full per-team `RunnerConfig`. **Chosen:** `DraftTeam.runner` stays a full `RunnerConfig` (the wizard's advanced panel needs concrete values to edit); `to_pipeline()` wraps it as `Some(TeamRunnerConfig::from_full(runner))`. Optionally `DraftPipeline` gains a `defaults` field carried through to the emitted `Pipeline`, but the wizard UI authoring of defaults is out of scope for R5 (frontend defaults-editing is a later A1 editor concern) — we carry `defaults` through the draft model so nothing is lost, defaulting to `None`.
- **DD8 — Frontend types.** `src/ipc/pipeline.ts` `Team.runner` becomes `runner?: TeamRunnerConfig | null` and a new `PipelineDefaults` is added to `Pipeline`. Because the store resolves before the `pipeline_load` IPC returns, the **loaded** `Pipeline` the viewer receives always has a full runner — but the type must still allow the partial/None shape on the wire for round-trip safety. We type `runner?: RunnerConfig | null` on the loaded Team (post-resolution it is full) and add the optional `defaults`. The wizard `DraftTeam.runner` stays full `RunnerConfig`.

---

## File Structure

- `src-tauri/agent_bus_core/src/runner.rs` — unchanged (RunnerKind, EffortMode, RunnerConfig already here? No — RunnerConfig lives in pipeline). No change.
- `src-tauri/pipeline/src/model.rs` — Modify: add `PipelineDefaults`, change `Team.runner` to `Option<TeamRunnerConfig>`, add `TeamRunnerConfig`, add `Pipeline.defaults`, add `Team::effective_runner()`, helpers.
- `src-tauri/pipeline/src/resolve.rs` — Create: `resolve_defaults(&Pipeline) -> Pipeline` and the per-team overlay logic.
- `src-tauri/pipeline/src/validate.rs` — Modify: add `TeamHasNoRunner` error; require each team's runner resolves.
- `src-tauri/pipeline/src/store.rs` — Modify: `load()` resolves before validating/returning.
- `src-tauri/pipeline/src/draft.rs` — Modify: `to_pipeline()` wraps team runner as `Some(TeamRunnerConfig)`; carry optional `defaults`.
- `src-tauri/pipeline/src/lib.rs` — Modify: `pub mod resolve;` + re-export.
- `src-tauri/pipeline/src/contract_tests.rs` — Modify: update full_team()/key-set asserts for the new shape (additive: `defaults` key, runner still present).
- `src-tauri/runtime/src/pool.rs` — Modify: read `team.effective_runner()` instead of `team.runner.model`.
- Any other `team.runner` readers in the workspace — Modify to `effective_runner()`.
- `src/ipc/pipeline.ts` — Modify: `TeamRunnerConfig`, `PipelineDefaults`, optional `runner`/`defaults`.

---

## Task 0: Register R5 language in DOMAIN.md (vet F1/F2)

**Files:**
- Modify: `DOMAIN.md`

- [ ] **Step 1: Update the Team language entry (vet F2)**

In `DOMAIN.md` → `### Pipeline Authoring`, change the `**Team**` bullet to note inheritance:

```
- **Team** — a node in the graph with one prompt, one scope, one runner config (a team may inherit the pipeline-level default and override fields selectively — R5; the *resolved* team always has exactly one fully-specified runner config)
```

- [ ] **Step 2: Add the effective-runner term (vet F1)**

Add a new bullet under `### Pipeline Authoring`:

```
- **Pipeline defaults / effective runner config** — `Pipeline.defaults` (`default_runner` / `default_model` / `default_effort`) supply runner config that teams inherit when they omit their own (R5). The **effective runner config** is a team's runner after the pipeline defaults are overlaid; Pipeline Authoring resolves it at load (`resolve.rs`) so Runtime always consumes a fully-specified `RunnerConfig` and never learns about defaults.
```

- [ ] **Step 3: Commit**

```bash
git add DOMAIN.md
git commit -m "docs(r5): register pipeline-defaults + effective-runner language (vet F1/F2)"
```

---

## Task 1: Add `PipelineDefaults` + `TeamRunnerConfig` value types

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs`
- Test: `src-tauri/pipeline/src/model.rs` (inline `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to `model.rs` tests module:

```rust
    #[test]
    fn team_runner_config_round_trips_and_is_all_optional() {
        // every field absent => deserialises to all-None
        let empty: TeamRunnerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty, TeamRunnerConfig::default());

        let full = TeamRunnerConfig {
            kind: Some(RunnerKind::ClaudeCli),
            model: Some("m".into()),
            effort: Some(EffortMode::Standard),
            api_key_env: None,
        };
        let s = serde_json::to_string(&full).unwrap();
        let back: TeamRunnerConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(full, back);
    }

    #[test]
    fn pipeline_defaults_is_all_optional() {
        let d: PipelineDefaults = serde_yaml::from_str("{}").unwrap();
        assert_eq!(d, PipelineDefaults::default());
        let d2 = PipelineDefaults {
            default_runner: Some(RunnerKind::ClaudeCli),
            default_model: Some("claude-opus-4-8".into()),
            default_effort: Some(EffortMode::ExtendedHigh),
        };
        let s = serde_json::to_string(&d2).unwrap();
        let back: PipelineDefaults = serde_json::from_str(&s).unwrap();
        assert_eq!(d2, back);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline model:: 2>&1 | tail -20`
Expected: FAIL — `cannot find type TeamRunnerConfig` / `PipelineDefaults`.

- [ ] **Step 3: Add the types to `model.rs`**

After the `RunnerConfig` block (and `default_effort` fn), add:

```rust
/// Pipeline-level runner defaults (R5). Every field optional: a pipeline may
/// supply just a model, just a runner kind, etc. Teams that omit a field
/// inherit it (Pipeline Authoring resolves; see resolve.rs). Additive — a
/// pipeline with no `defaults` key loads unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PipelineDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_runner: Option<RunnerKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<EffortMode>,
}

/// A team's runner config as authored (R5): every field optional so a team can
/// override just the model and inherit kind+effort from `Pipeline.defaults`.
/// The resolver (resolve.rs) overlays this on the pipeline defaults to produce
/// a fully-specified `RunnerConfig`. A team that omits `runner` entirely
/// inherits the whole default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TeamRunnerConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<RunnerKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

impl TeamRunnerConfig {
    /// Wrap a fully-specified RunnerConfig as a (complete) override — used by
    /// the wizard's `to_pipeline()` where the team runner is always full.
    pub fn from_full(r: RunnerConfig) -> Self {
        Self {
            kind: Some(r.kind),
            model: Some(r.model),
            effort: Some(r.effort),
            api_key_env: r.api_key_env,
        }
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline model:: 2>&1 | tail -20`
Expected: PASS (the two new tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/model.rs
git commit -m "feat(r5): add PipelineDefaults + TeamRunnerConfig value types"
```

---

## Task 2: Make `Team.runner` optional + add `Pipeline.defaults` + `effective_runner()`

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs`
- Test: `src-tauri/pipeline/src/model.rs` tests

This task changes `Team.runner` from `RunnerConfig` to `Option<TeamRunnerConfig>` and adds `Pipeline.defaults`. It will break compilation across the workspace; later tasks fix each consumer. Keep the model crate compiling by updating model.rs's own tests/helpers in this task.

- [ ] **Step 1: Write the failing test**

Add to `model.rs` tests:

```rust
    #[test]
    fn effective_runner_returns_the_resolved_config() {
        let mut t = sample_team("research");
        // sample_team sets a Some(full) runner; effective_runner returns it
        let r = t.effective_runner();
        assert_eq!(r.kind, RunnerKind::ClaudeCli);
        assert_eq!(r.model, "claude-opus-4-7");
        // None => effective_runner panics (unresolved pipeline = programmer error)
        t.runner = None;
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.effective_runner())).is_err());
    }

    #[test]
    fn pipeline_defaults_field_defaults_to_none_when_absent() {
        let json = r#"{"id":"p","name":"P","schema_version":2,"teams":[]}"#;
        let p: Pipeline = serde_json::from_str(json).unwrap();
        assert!(p.defaults.is_none());
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline 2>&1 | tail -25`
Expected: FAIL — compile errors (`runner` type mismatch) + missing `effective_runner` / `defaults`.

- [ ] **Step 3: Edit the model**

In `Pipeline`, add after `description` field's existing fields (place near top, before `teams`):

```rust
    /// Pipeline-level runner defaults teams inherit (R5). None = no defaults;
    /// every team must then specify its own runner. Resolved at load
    /// (resolve.rs) so Runtime only ever sees fully-specified team runners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<PipelineDefaults>,
```

Change `Team.runner`:

```rust
    /// As authored: a partial override over `Pipeline.defaults`, or None to
    /// inherit the whole default. After `resolve::resolve_defaults` every team
    /// holds `Some(fully-specified)`; use `effective_runner()` to read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<TeamRunnerConfig>,
```

Add to `impl` block for `Team` (create one if absent, after the `Pipeline` impl):

```rust
impl Team {
    /// The team's fully-resolved runner. Only valid after the pipeline has been
    /// resolved (resolve::resolve_defaults) — every field is then present.
    /// Panics if called on an unresolved team (a programmer error: the store
    /// resolves + validates before any consumer sees the pipeline).
    pub fn effective_runner(&self) -> RunnerConfig {
        let tr = self
            .runner
            .as_ref()
            .expect("team runner not resolved (call resolve::resolve_defaults first)");
        RunnerConfig {
            kind: tr.kind.expect("resolved runner missing kind"),
            model: tr.model.clone().expect("resolved runner missing model"),
            effort: tr.effort.expect("resolved runner missing effort"),
            api_key_env: tr.api_key_env.clone(),
        }
    }
}
```

Update `model.rs`'s own `sample_team` helper to use the new shape:

```rust
            runner: Some(TeamRunnerConfig {
                kind: Some(RunnerKind::ClaudeCli),
                model: Some("claude-opus-4-7".into()),
                effort: Some(EffortMode::ExtendedHigh),
                api_key_env: None,
            }),
```

Update the two inline `Pipeline { ... }` literals in model.rs tests to add `defaults: None,`.

- [ ] **Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline model:: 2>&1 | tail -25`
Expected: PASS for model tests. (Other modules in the crate may still fail to compile — fixed in Tasks 3–6. If `cargo test -p pipeline` won't compile the whole crate, that's expected; proceed.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/model.rs
git commit -m "feat(r5): Team.runner optional + Pipeline.defaults + effective_runner()"
```

---

## Task 3: The resolver (`resolve.rs`)

**Files:**
- Create: `src-tauri/pipeline/src/resolve.rs`
- Modify: `src-tauri/pipeline/src/lib.rs`
- Test: `src-tauri/pipeline/src/resolve.rs` inline tests

- [ ] **Step 1: Write the failing tests (create the file with tests + stub)**

Create `src-tauri/pipeline/src/resolve.rs`:

```rust
//! Default resolution (R5). Pipeline Authoring overlays each team's partial
//! `TeamRunnerConfig` on `Pipeline.defaults`, producing a Pipeline whose every
//! team holds a fully-specified runner. Runtime consumes the resolved pipeline
//! and never learns about defaults (DOMAIN.md: no resolution logic leaks into
//! Runtime). Base fallbacks (claude-cli / standard effort) apply only when
//! neither the team nor the pipeline default supplies a value; `model` has no
//! base fallback (an unspecified model is a validation error, not a guess).

use crate::model::{Pipeline, RunnerConfig, TeamRunnerConfig};
use agent_bus_core::{EffortMode, RunnerKind};

/// Overlay one team's authored runner over the pipeline defaults.
/// Precedence: team field > pipeline default > base fallback. `model` may end
/// up None (no base fallback) — validation rejects that.
fn resolve_one(team: &TeamRunnerConfig, defaults: Option<&crate::model::PipelineDefaults>) -> TeamRunnerConfig {
    let (dk, dm, de) = match defaults {
        Some(d) => (d.default_runner, d.default_model.clone(), d.default_effort),
        None => (None, None, None),
    };
    TeamRunnerConfig {
        kind: team.kind.or(dk).or(Some(RunnerKind::ClaudeCli)),
        model: team.model.clone().or(dm),
        effort: team.effort.or(de).or(Some(EffortMode::Standard)),
        api_key_env: team.api_key_env.clone(),
    }
}

/// Resolve every team's runner against the pipeline defaults, returning a new
/// Pipeline. The returned pipeline's `defaults` is preserved (for round-trip /
/// re-save) but each team now carries a fully-overlaid `Some(runner)`.
pub fn resolve_defaults(pipeline: &Pipeline) -> Pipeline {
    let mut out = pipeline.clone();
    let defaults = pipeline.defaults.as_ref();
    for team in &mut out.teams {
        let authored = team.runner.clone().unwrap_or_default();
        team.runner = Some(resolve_one(&authored, defaults));
    }
    out
}

/// True when every team's runner is fully specified (kind+model+effort all
/// Some). Used by validation post-resolution.
pub fn is_fully_resolved(r: &TeamRunnerConfig) -> bool {
    r.kind.is_some() && r.model.is_some() && r.effort.is_some()
}

/// Convenience: read a resolved team runner as a concrete RunnerConfig.
/// (Mirrors Team::effective_runner; kept for resolver-internal tests.)
#[allow(dead_code)]
fn as_full(r: &TeamRunnerConfig) -> Option<RunnerConfig> {
    Some(RunnerConfig {
        kind: r.kind?,
        model: r.model.clone()?,
        effort: r.effort?,
        api_key_env: r.api_key_env.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pipeline, PipelineDefaults, Routes, Scope, Team, TeamRunnerConfig, Workers};
    use agent_bus_core::{EffortMode, RunnerKind};

    fn team_with(id: &str, runner: Option<TeamRunnerConfig>) -> Team {
        Team {
            id: id.into(),
            name: id.into(),
            prompt: format!("prompts/{id}.md"),
            runner,
            scope: Scope::default(),
            outputs: Routes::default(),
            workers: Workers::default(),
        }
    }

    fn pipeline_with(defaults: Option<PipelineDefaults>, teams: Vec<Team>) -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(),
            defaults, schema_version: 2, teams,
            gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
        }
    }

    #[test]
    fn team_omitting_runner_inherits_the_full_default() {
        let p = pipeline_with(
            Some(PipelineDefaults {
                default_runner: Some(RunnerKind::ClaudeCli),
                default_model: Some("claude-opus-4-8".into()),
                default_effort: Some(EffortMode::ExtendedHigh),
            }),
            vec![team_with("research", None)],
        );
        let r = resolve_defaults(&p);
        let er = r.teams[0].effective_runner();
        assert_eq!(er.kind, RunnerKind::ClaudeCli);
        assert_eq!(er.model, "claude-opus-4-8");
        assert_eq!(er.effort, EffortMode::ExtendedHigh);
    }

    #[test]
    fn team_field_overrides_the_pipeline_default() {
        let p = pipeline_with(
            Some(PipelineDefaults {
                default_runner: Some(RunnerKind::ClaudeCli),
                default_model: Some("default-model".into()),
                default_effort: Some(EffortMode::Standard),
            }),
            vec![team_with("research", Some(TeamRunnerConfig {
                model: Some("override-model".into()),
                ..Default::default()
            }))],
        );
        let er = resolve_defaults(&p).teams[0].effective_runner();
        assert_eq!(er.model, "override-model"); // team wins
        assert_eq!(er.effort, EffortMode::Standard); // inherited
    }

    #[test]
    fn base_fallbacks_apply_when_no_default_and_no_team_field() {
        // model still required (no base) but kind+effort fall back
        let p = pipeline_with(None, vec![team_with("research", Some(TeamRunnerConfig {
            model: Some("m".into()),
            ..Default::default()
        }))]);
        let er = resolve_defaults(&p).teams[0].effective_runner();
        assert_eq!(er.kind, RunnerKind::ClaudeCli);
        assert_eq!(er.effort, EffortMode::Standard);
        assert_eq!(er.model, "m");
    }

    #[test]
    fn model_stays_none_when_neither_team_nor_default_supplies_it() {
        let p = pipeline_with(None, vec![team_with("research", None)]);
        let resolved = resolve_defaults(&p);
        assert!(resolved.teams[0].runner.as_ref().unwrap().model.is_none());
        assert!(!is_fully_resolved(resolved.teams[0].runner.as_ref().unwrap()));
    }

    #[test]
    fn resolve_is_idempotent_on_a_full_runner() {
        let full = TeamRunnerConfig::from_full(crate::model::RunnerConfig {
            kind: RunnerKind::AnthropicApi, model: "x".into(),
            effort: EffortMode::ExtendedLow, api_key_env: Some("K".into()),
        });
        let p = pipeline_with(None, vec![team_with("t", Some(full.clone()))]);
        let once = resolve_defaults(&p);
        let twice = resolve_defaults(&once);
        assert_eq!(once.teams[0].runner, twice.teams[0].runner);
    }
}
```

- [ ] **Step 2: Wire the module**

In `src-tauri/pipeline/src/lib.rs`, add `pub mod resolve;` after `pub mod draft;` and add `pub use resolve::*;` after the other re-exports.

- [ ] **Step 3: Run to verify it fails then passes**

Run: `cd src-tauri && cargo test -p pipeline resolve:: 2>&1 | tail -30`
Expected: compiles, all `resolve::` tests PASS. (If pipeline crate still has unrelated compile errors from Tasks 4–6 consumers within the same crate — there are none expected in this crate yet besides validate/draft/contract — proceed to those tasks; resolve tests should compile once model.rs Task 2 is in.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/pipeline/src/resolve.rs src-tauri/pipeline/src/lib.rs
git commit -m "feat(r5): default resolver overlays team runner on pipeline defaults"
```

---

## Task 4: Validation requires a resolvable runner

**Files:**
- Modify: `src-tauri/pipeline/src/validate.rs`
- Test: `src-tauri/pipeline/src/validate.rs` tests

- [ ] **Step 1: Write the failing test**

Add to validate.rs tests (the `team` helper there builds a full `RunnerConfig` — update it in Step 3). New test:

```rust
    #[test]
    fn a_team_with_no_resolvable_model_is_rejected() {
        use crate::model::TeamRunnerConfig;
        let mut p = valid_pipeline();
        // strip the model so it cannot resolve (no pipeline default either)
        p.teams[0].runner = Some(TeamRunnerConfig { model: None, ..Default::default() });
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::TeamHasNoRunner("research".into()))
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline validate:: 2>&1 | tail -20`
Expected: FAIL — no `TeamHasNoRunner` variant + helper compile errors.

- [ ] **Step 3: Edit validate.rs**

Add the error variant to `PipelineValidationError`:

```rust
    #[error("team '{0}' has no resolvable runner (no model from the team or pipeline defaults)")]
    TeamHasNoRunner(String),
```

In `validate()`, after the `NoTeams` check (before/around unique-id collection — order doesn't matter, put it right after the `if p.teams.is_empty()` block), add:

```rust
    // R5: every team must end up with a fully-specified runner. validate runs on
    // the RESOLVED pipeline (store::load resolves first), so a team whose runner
    // is None or missing kind/model/effort here means neither the team nor the
    // pipeline defaults supplied it.
    for team in &p.teams {
        let ok = team
            .runner
            .as_ref()
            .map(crate::resolve::is_fully_resolved)
            .unwrap_or(false);
        if !ok {
            return Err(PipelineValidationError::TeamHasNoRunner(team.id.clone()));
        }
    }
```

Update the validate.rs `team()` test helper to the new shape:

```rust
    fn team(id: &str, approve: Option<&str>) -> Team {
        Team {
            id: id.into(),
            name: id.into(),
            prompt: format!("prompts/{id}.md"),
            runner: Some(crate::model::TeamRunnerConfig {
                kind: Some(RunnerKind::ClaudeCli),
                model: Some("m".into()),
                effort: Some(EffortMode::Standard),
                api_key_env: None,
            }),
            scope: Scope::default(),
            outputs: Routes { on_approve: approve.map(String::from), on_revise: None, on_reject: None },
            workers: Workers::default(),
        }
    }
```

Update both `valid_pipeline()` and `valid_v2_pipeline()` literals in validate.rs to add `defaults: None,`. Remove the now-unused `RunnerConfig` import if the compiler warns (keep `EffortMode, RunnerKind`).

- [ ] **Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline validate:: 2>&1 | tail -25`
Expected: PASS (all validate tests incl. the new one).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/validate.rs
git commit -m "feat(r5): validation requires every team to resolve to a runner"
```

---

## Task 5: Store resolves at load; draft emits wrapped runner

**Files:**
- Modify: `src-tauri/pipeline/src/store.rs`
- Modify: `src-tauri/pipeline/src/draft.rs`
- Test: store.rs + draft.rs tests

- [ ] **Step 1: Write the failing tests**

Add to store.rs tests:

```rust
    #[test]
    fn load_resolves_pipeline_defaults_into_each_team() {
        use crate::model::{Escalation, Pipeline, PipelineDefaults, Routes, Scope, Team, TeamRunnerConfig, Workers};
        use agent_bus_core::{EffortMode, RunnerKind};
        let store = PipelineStore::new(temp_root());
        // author a pipeline whose team omits model; pipeline default supplies it
        let p = Pipeline {
            id: "demo".into(), name: "Demo".into(), description: String::new(),
            defaults: Some(PipelineDefaults {
                default_runner: Some(RunnerKind::ClaudeCli),
                default_model: Some("claude-opus-4-8".into()),
                default_effort: Some(EffortMode::ExtendedHigh),
            }),
            schema_version: 2,
            teams: vec![Team {
                id: "research".into(), name: "Research".into(), prompt: "prompts/research.md".into(),
                runner: None, // inherits the whole default
                scope: Scope::default(),
                outputs: Routes { on_approve: Some("needs-human".into()), on_revise: None, on_reject: None },
                workers: Workers::default(),
            }],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![], joins: vec![],
        };
        // write the *authored* YAML directly (save() validates the resolved form,
        // so author then write the raw yaml to disk)
        let dir = workspace::paths::pipelines_dir(store.project_root());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("demo.yaml"), serde_yaml::to_string(&p).unwrap()).unwrap();

        let loaded = store.load("demo").unwrap();
        let er = loaded.teams[0].effective_runner();
        assert_eq!(er.model, "claude-opus-4-8");
        assert_eq!(er.effort, EffortMode::ExtendedHigh);
    }
```

Add to draft.rs tests:

```rust
    #[test]
    fn to_pipeline_wraps_team_runner_as_some_full_override() {
        let p = complete_draft().to_pipeline();
        let tr = p.teams[0].runner.as_ref().unwrap();
        assert!(tr.kind.is_some() && tr.model.is_some() && tr.effort.is_some());
        // and it resolves + hard-validates
        let resolved = crate::resolve::resolve_defaults(&p);
        assert_eq!(crate::validate::validate(&resolved), Ok(()));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline store:: draft:: 2>&1 | tail -25`
Expected: FAIL — `load` returns unresolved (`effective_runner` panics) / `to_pipeline` still emits old shape (compile error on `runner: t.runner.clone()`).

- [ ] **Step 3: Edit store.rs `load`**

Change `load` to resolve before validate+return:

```rust
    pub fn load(&self, id: &str) -> Result<Pipeline, PipelineStoreError> {
        let path = self.yaml_path(id);
        if !path.exists() {
            return Err(PipelineStoreError::NotFound(id.to_string()));
        }
        let yaml = std::fs::read_to_string(&path)?;
        let pipeline = parse_pipeline(&yaml)?;
        let resolved = crate::resolve::resolve_defaults(&pipeline);
        validate(&resolved)?;
        Ok(resolved)
    }
```

Also update store.rs's `save_round_trips_through_yaml` test literal: change the team's `runner:` to `Some(TeamRunnerConfig { kind: Some(..), model: Some("m"), effort: Some(Standard), api_key_env: None })` and add `defaults: None,` to the `Pipeline` literal. (`save` calls `validate` directly — the authored pipeline must already be resolvable; a full runner is. For belt-and-braces, `save` may also resolve first; **Recommended:** make `save` resolve too so an author-side full pipeline always validates. Change `save`'s first line to `let resolved = crate::resolve::resolve_defaults(pipeline); validate(&resolved)?;` and write `pipeline` (the authored form) to disk — preserving the compact defaults on disk. Keep writing the *authored* `pipeline`, not `resolved`, so `defaults` + omitted team runners stay compact on disk.)

- [ ] **Step 4: Edit draft.rs `to_pipeline`**

In `to_pipeline`, change the team mapping `runner` field:

```rust
                    runner: Some(crate::model::TeamRunnerConfig::from_full(t.runner.clone())),
```

(Leave `DraftTeam.runner: RunnerConfig` unchanged — the wizard authors full values.) Add `defaults: None,` to the `Pipeline { ... }` literal in `to_pipeline`.

- [ ] **Step 5: Run to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline 2>&1 | tail -30`
Expected: the whole `pipeline` crate compiles; store + draft + earlier tests PASS. (contract_tests may still fail — Task 6.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/store.rs src-tauri/pipeline/src/draft.rs
git commit -m "feat(r5): store resolves defaults at load; draft emits wrapped runner"
```

---

## Task 6: Fix contract tests for the new shape

**Files:**
- Modify: `src-tauri/pipeline/src/contract_tests.rs`
- Test: same file

- [ ] **Step 1: Run to see the failures**

Run: `cd src-tauri && cargo test -p pipeline contract 2>&1 | tail -40`
Expected: FAIL — `full_team()` uses `runner: full_runner()` (type mismatch) and `pipeline_key_set_matches_ts` lacks `defaults`.

- [ ] **Step 2: Update `full_team()` + the key-set assertions**

Change `full_team()`'s `runner` field to the wrapped optional shape:

```rust
        runner: Some(crate::model::TeamRunnerConfig {
            kind: Some(RunnerKind::AnthropicApi),
            model: Some("claude-opus-4-7".into()),
            effort: Some(EffortMode::ExtendedHigh),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
        }),
```

In `pipeline_key_set_matches_ts`, add `defaults: None,` to the `Pipeline` literal. Because `defaults` uses `skip_serializing_if = "Option::is_none"`, a `None` defaults does NOT add a key — so the expected key set is unchanged. To lock the *presence* of the key when populated, add a second focused test:

```rust
    #[test]
    fn pipeline_defaults_key_appears_when_present() {
        use crate::model::PipelineDefaults;
        use agent_bus_core::{EffortMode, RunnerKind};
        let mut p = Pipeline {
            id: "p".into(), name: "P".into(), description: "d".into(), schema_version: 2,
            defaults: Some(PipelineDefaults {
                default_runner: Some(RunnerKind::ClaudeCli),
                default_model: Some("m".into()),
                default_effort: Some(EffortMode::Standard),
            }),
            teams: vec![full_team()], gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
        };
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.as_object().unwrap().contains_key("defaults"));
        let d = &v["defaults"];
        assert!(d.get("default_runner").is_some());
        assert!(d.get("default_model").is_some());
        assert!(d.get("default_effort").is_some());
        // and team.runner is still an object with the override keys
        let r = &v["teams"][0]["runner"];
        assert!(r.get("model").is_some());
        p.defaults = None; // silence unused-mut if needed
        let _ = p;
    }
```

The `team_key_set_matches_ts` assertion expects keys `{id,name,prompt,runner,scope,outputs,workers}` — `runner` is `Some(..)` so the key still appears; assertion unchanged.

- [ ] **Step 3: Run to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline 2>&1 | tail -20`
Expected: whole `pipeline` crate PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/pipeline/src/contract_tests.rs
git commit -m "test(r5): lock new defaults/optional-runner serde contract"
```

---

## Task 7: Update Runtime to read `effective_runner()`

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs` (lines ~101-102)
- Modify: any other workspace reader of `team.runner.*` (grep)
- Test: existing runtime tests must stay green; add one asserting effective runner is read.

- [ ] **Step 1: Find every reader**

Run: `cd src-tauri && grep -rn "\.runner\." runtime/src app/src 2>/dev/null`
Expected: `pool.rs:101 team.runner.model`, `pool.rs:102 team.runner.effort`. Note any others (app/src lib).

- [ ] **Step 2: Write/adjust the failing test**

In pool.rs tests (find the existing test that builds a `Team` and runs `process_one_claim`), the `Team` literals there set `runner: RunnerConfig{..}`. They will not compile. Update each test `Team` literal to the resolved shape:

```rust
        runner: Some(pipeline::TeamRunnerConfig {
            kind: Some(RunnerKind::ClaudeCli),
            model: Some("test-model".into()),
            effort: Some(EffortMode::Standard),
            api_key_env: None,
        }),
```

(Match the model string the existing assertions expect; keep them identical to current values — read the file first and preserve the exact `model`/`effort` used.)

Add an assertion in whichever test inspects the `InvocationRequest` that `req.model` equals the team's effective model (if such a test exists; if the runner is a fake capturing the request, assert on the captured `model`).

- [ ] **Step 3: Edit pool.rs `process_one_claim`**

Replace lines reading `team.runner`:

```rust
        let effective = team.effective_runner();
        let req = InvocationRequest {
            task_id: task.id.0.clone(),
            team_id: team.id.clone(),
            model: effective.model.clone(),
            thinking_budget: effective.effort.budget_tokens(),
            // ...rest unchanged...
```

(Add `use pipeline::...` if `RunnerConfig`/`TeamRunnerConfig` types are referenced in tests; `effective_runner()` returns `RunnerConfig` which pool already imports as `Team` comes from `pipeline`.)

- [ ] **Step 4: Fix any other readers found in Step 1**

For each other `team.runner.X` reader, replace with `team.effective_runner().X`. If a reader needs the whole config, bind `let r = team.effective_runner();` once.

- [ ] **Step 5: Run runtime tests**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | tail -25`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(r5): runtime reads effective_runner (defaults resolved upstream)"
```

---

## Task 8: Fix remaining workspace compile sites + app crate

**Files:**
- Modify: `src-tauri/app/src/lib.rs` and any crate that constructs a `Team`/`Pipeline` literal or reads `team.runner`.
- Test: workspace build.

- [ ] **Step 1: Build the whole workspace to find breakage**

Run: `cd src-tauri && cargo check --workspace 2>&1 | grep -E "error|-->" | head -60`
Expected: a list of `Team { runner: ... }` literal mismatches / `Pipeline { ... }` literals missing `defaults`.

- [ ] **Step 2: Fix each site**

For each error: a `Pipeline { ... }` literal needs `defaults: None,`; a `Team { ... }` literal needs `runner: Some(pipeline::TeamRunnerConfig { kind: Some(..), model: Some(..), effort: Some(..), api_key_env: None })` (or `runner: None` where appropriate for a test of inheritance). Any `team.runner.model` read → `team.effective_runner().model`. Preserve the exact model/effort values each test currently asserts on (read before edit).

- [ ] **Step 3: Re-check**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add -A src-tauri
git commit -m "fix(r5): update remaining Team/Pipeline literals for optional runner"
```

---

## Task 9: Frontend IPC types

**Files:**
- Modify: `src/ipc/pipeline.ts`
- Test: `bun vitest run` (no behavioural change expected; this is type-level)

- [ ] **Step 1: Edit the types**

Add after `RunnerConfig`:

```typescript
export interface TeamRunnerConfig {
  kind?: RunnerKind | null;
  model?: string | null;
  effort?: EffortMode | null;
  api_key_env?: string | null;
}

export interface PipelineDefaults {
  default_runner?: RunnerKind | null;
  default_model?: string | null;
  default_effort?: EffortMode | null;
}
```

Change `interface Team`'s `runner`:

```typescript
  runner?: RunnerConfig | null; // post-resolution (pipeline_load) this is the full RunnerConfig
```

Add to `interface Pipeline`:

```typescript
  defaults?: PipelineDefaults | null;
```

Leave `DraftTeam.runner: RunnerConfig` (wizard authors full values) unchanged.

- [ ] **Step 2: Run the frontend tests + typecheck**

Run: `PATH="/opt/homebrew/bin:$PATH" bun vitest run 2>&1 | tail -20`
Run: `PATH="/opt/homebrew/bin:$PATH" bun run build 2>&1 | tail -20`
Expected: vitest all PASS; build succeeds. If `tsc` flags a `t.runner.model` access in the wizard (DraftTeam keeps full runner, so no) — none expected. If the viewer (`PipelineView.tsx`) reads `team.runner.model` directly and now sees `runner?`, add a guard `team.runner?.model ?? ""`. Fix any such site minimally.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/pipeline.ts $(git diff --name-only -- 'src/**/*.tsx' 'src/**/*.ts')
git commit -m "feat(r5): frontend types for pipeline defaults + optional team runner"
```

---

## Task 10: Full verification

- [ ] **Step 1: Cargo**

Run: `cd src-tauri && cargo test --workspace 2>&1 | tail -30`
Expected: all green.

Run: `cd src-tauri && cargo clippy --workspace 2>&1 | tail -20`
Expected: no warnings (fix any: unused imports from the runner refactor are the likely ones).

- [ ] **Step 2: Frontend**

Run: `PATH="/opt/homebrew/bin:$PATH" bun vitest run 2>&1 | tail -15`
Run: `PATH="/opt/homebrew/bin:$PATH" bun run build 2>&1 | tail -10`
Expected: all green.

- [ ] **Step 3: Final commit (if clippy fixes needed)**

```bash
git add -A && git commit -m "chore(r5): clippy clean + final verification"
```

---

## Self-Review

- **Spec coverage:** `pipeline.default_runner`/`default_model`/default effort → `PipelineDefaults` (Task 1). Team inherits when omitting config → `TeamRunnerConfig` optional + resolver (Tasks 1–3). Team config overrides → `resolve_one` precedence (Task 3). Resolver computes effective RunnerConfig → `resolve_defaults` + `effective_runner` (Tasks 2–3). Validation requires effective runner → `TeamHasNoRunner` (Task 4). Existing v2 pipelines load unchanged → additive `#[serde(default)]`, no SCHEMA_VERSION bump (DD6). Runtime consumes resolved config, no leak → store resolves at load; Runtime calls `effective_runner` (Tasks 5, 7). Wizard pipelines still validate/run → `to_pipeline` wraps full runner (Task 5), runtime/app literals fixed (Tasks 7–8).
- **Placeholder scan:** none — every code step shows the code; "grep then fix each site" tasks (8) are mechanical with the exact replacement shown.
- **Type consistency:** `TeamRunnerConfig` fields `{kind,model,effort,api_key_env}` consistent across model/resolve/validate/draft/contract/runtime. `effective_runner() -> RunnerConfig` used identically in pool.rs and tests. `resolve_defaults(&Pipeline)->Pipeline` signature consistent in store + tests.
