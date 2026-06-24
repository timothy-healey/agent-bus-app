//! Default resolution (R5). Pipeline Authoring overlays each team's partial
//! `TeamRunnerConfig` on `Pipeline.defaults`, producing a Pipeline whose every
//! team holds a fully-specified runner. Runtime consumes the resolved pipeline
//! and never learns about defaults (DOMAIN.md: no resolution logic leaks into
//! Runtime). Base fallbacks (claude-cli / standard effort) apply only when
//! neither the team nor the pipeline default supplies a value; `model` has no
//! base fallback (an unspecified model is a validation error, not a guess).

use crate::model::{Pipeline, PipelineDefaults, TeamRunnerConfig};
use agent_bus_core::{EffortMode, RunnerKind};

/// Overlay one team's authored runner over the pipeline defaults.
/// Precedence: team field > pipeline default > base fallback. `model` may end
/// up None (no base fallback) — validation rejects that.
fn resolve_one(team: &TeamRunnerConfig, defaults: Option<&PipelineDefaults>) -> TeamRunnerConfig {
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
            role: crate::model::Role::default(),
            store: crate::model::Store::default(),
        }
    }

    fn pipeline_with(defaults: Option<PipelineDefaults>, teams: Vec<Team>) -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(),
            schema_version: 2, defaults, teams,
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
