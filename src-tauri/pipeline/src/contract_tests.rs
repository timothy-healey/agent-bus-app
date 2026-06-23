//! Serde contract regression tests for the Pipeline Authoring IPC return types:
//! `Pipeline` and its node/team/gate value types, plus the wizard's draft types.
//! Lock the
//! serialized JSON key set against the TS interfaces. Several fields use
//! `#[serde(skip_serializing_if = "Option::is_none")]` (`RunnerConfig.api_key_env`,
//! all of `Routes`), so each instance is FULLY populated (every Option = Some) to
//! force every key to appear and match the TS interface. Additive only.

#![cfg(test)]

use crate::model::{Escalation, Gate, Pipeline, Routes, RunnerConfig, Scope, Team, Workers};
use crate::draft::{DraftPipeline, DraftTeam, Slice, SliceTeam, TeamsSlice};
use agent_bus_core::{EffortMode, RunnerKind};
use serde_json::Value;
use std::collections::BTreeSet;

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn full_runner() -> RunnerConfig {
    RunnerConfig {
        kind: RunnerKind::AnthropicApi,
        model: "claude-opus-4-7".into(),
        effort: EffortMode::ExtendedHigh,
        // Some so the skip_serializing_if field appears (TS: api_key_env?).
        api_key_env: Some("ANTHROPIC_API_KEY".into()),
    }
}

fn full_team() -> Team {
    Team {
        id: "research".into(),
        name: "Research".into(),
        prompt: "prompts/research.md".into(),
        runner: full_runner(),
        scope: Scope { reads: vec!["src".into()], writes: vec!["artifacts".into()], tools: vec!["bash".into()] },
        // All routes Some so the skip_serializing_if fields appear (TS: on_*?).
        outputs: Routes {
            on_approve: Some("gate-1".into()),
            on_revise: Some("research".into()),
            on_reject: Some("needs-human".into()),
        },
        workers: Workers { default: 1, max: 3 },
    }
}

/// Locks `src/ipc/pipeline.ts:57-65` `interface Pipeline`:
/// { id, name, description, schema_version, teams, gates, escalations }.
#[test]
fn pipeline_key_set_matches_ts() {
    let p = Pipeline {
        id: "p".into(),
        name: "P".into(),
        description: "d".into(),
        schema_version: 1,
        teams: vec![full_team()],
        gates: vec![Gate { id: "gate-1".into(), label: "Gate".into(), downstream: "research".into() }],
        escalations: vec![Escalation { id: "needs-human".into(), triggers: vec!["timeout".into()] }],
        forks: vec![],
        joins: vec![],
    };
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(
        keys(&v),
        set(&["id", "name", "description", "schema_version", "teams", "gates", "escalations", "forks", "joins"]),
    );
    assert!(v["teams"].is_array());
    assert!(v["schema_version"].is_number());
}

/// Locks `src/ipc/pipeline.ts:36-44` `interface Team`:
/// { id, name, prompt, runner, scope, outputs, workers }.
#[test]
fn team_key_set_matches_ts() {
    let v = serde_json::to_value(full_team()).unwrap();
    assert_eq!(keys(&v), set(&["id", "name", "prompt", "runner", "scope", "outputs", "workers"]));
}

/// Locks `src/ipc/pipeline.ts:12-17` `interface RunnerConfig`:
/// { kind, model, effort, api_key_env? }. With api_key_env = Some it must appear.
#[test]
fn runner_config_key_set_matches_ts() {
    let v = serde_json::to_value(full_runner()).unwrap();
    assert_eq!(keys(&v), set(&["kind", "model", "effort", "api_key_env"]));
    assert_eq!(v["kind"], Value::String("anthropic-api".into()));
    // effort is the internally-tagged EffortMode object.
    assert_eq!(v["effort"]["mode"], Value::String("extended-high".into()));
}

/// `api_key_env` is optional in TS (`api_key_env?`): omitted entirely when None
/// (skip_serializing_if). Locks `src/ipc/pipeline.ts:16`.
#[test]
fn runner_config_omits_api_key_env_when_none() {
    let rc = RunnerConfig {
        kind: RunnerKind::ClaudeCli,
        model: "m".into(),
        effort: EffortMode::Standard,
        api_key_env: None,
    };
    let v = serde_json::to_value(&rc).unwrap();
    assert_eq!(keys(&v), set(&["kind", "model", "effort"]));
    assert!(!v.as_object().unwrap().contains_key("api_key_env"));
}

/// Locks `src/ipc/pipeline.ts:19-23` `interface Scope { reads; writes; tools }`.
#[test]
fn scope_key_set_matches_ts() {
    let v = serde_json::to_value(Scope { reads: vec![], writes: vec![], tools: vec![] }).unwrap();
    assert_eq!(keys(&v), set(&["reads", "writes", "tools"]));
}

/// Locks `src/ipc/pipeline.ts:25-29` `interface Routes`: all three keys optional
/// (`on_*?`). With all Some, the full key set appears.
#[test]
fn routes_key_set_matches_ts_when_all_present() {
    let v = serde_json::to_value(Routes {
        on_approve: Some("a".into()),
        on_revise: Some("b".into()),
        on_reject: Some("c".into()),
    })
    .unwrap();
    assert_eq!(keys(&v), set(&["on_approve", "on_revise", "on_reject"]));
}

/// Routes fields are optional in TS: omitted entirely when None.
/// Locks `src/ipc/pipeline.ts:26-28`.
#[test]
fn routes_omits_none_edges() {
    let v = serde_json::to_value(Routes::default()).unwrap();
    assert!(v.as_object().unwrap().is_empty());
}

/// Locks `src/ipc/pipeline.ts:31-34` `interface Workers { default; max }`.
#[test]
fn workers_key_set_matches_ts() {
    let v = serde_json::to_value(Workers { default: 1, max: 4 }).unwrap();
    assert_eq!(keys(&v), set(&["default", "max"]));
}

/// Locks `src/ipc/pipeline.ts:46-50` `interface Gate { id; label; downstream }`.
#[test]
fn gate_key_set_matches_ts() {
    let v = serde_json::to_value(Gate { id: "g".into(), label: "L".into(), downstream: "d".into() }).unwrap();
    assert_eq!(keys(&v), set(&["id", "label", "downstream"]));
}

/// Locks `src/ipc/pipeline.ts:52-55` `interface Escalation { id; triggers }`.
#[test]
fn escalation_key_set_matches_ts() {
    let v = serde_json::to_value(Escalation { id: "e".into(), triggers: vec!["t".into()] }).unwrap();
    assert_eq!(keys(&v), set(&["id", "triggers"]));
}

/// Locks the DraftPipeline key set the wizard IPC mirrors (src/ipc/pipeline.ts).
#[test]
fn draft_pipeline_key_set_matches_ts() {
    let mut d = DraftPipeline::empty();
    d.id = "p".into();
    d.name = "P".into();
    d.teams.push(DraftTeam::new("research", "Research"));
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(
        keys(&v),
        set(&["id", "name", "description", "schema_version", "teams", "forks", "joins", "escalations"]),
    );
}

/// Locks DraftTeam (carries prompt_body inline, not a path).
#[test]
fn draft_team_key_set_matches_ts() {
    let v = serde_json::to_value(DraftTeam::new("research", "Research")).unwrap();
    assert_eq!(keys(&v), set(&["id", "name", "prompt_body", "runner", "scope", "outputs", "workers"]));
}

/// Locks the internally-tagged Slice wire shape (kind discriminator).
#[test]
fn teams_slice_serialises_with_kind_tag() {
    let s = Slice::Teams(TeamsSlice { teams: vec![SliceTeam { id: "a".into(), name: "A".into() }] });
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["kind"], serde_json::Value::String("teams".into()));
    assert!(v["teams"].is_array());
}
