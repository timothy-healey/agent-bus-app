//! Pre-flight check at run start: every `claude-cli` team's effective model and
//! effort must be in the current model list, or the run does not start. The
//! CLI never validates `--effort` (an unsupported level is silently ignored),
//! so this is where an invalid combination is caught.

use agent_bus_core::{ModelList, RunnerConfigProblem, RunnerKind, TeamId};
use pipeline::model::Pipeline;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightFailed {
    pub problems: Vec<(TeamId, RunnerConfigProblem)>,
}

impl PreflightFailed {
    /// One message naming each offending team (by name, else id) and why.
    pub fn describe(&self, pipeline: &Pipeline) -> String {
        let parts: Vec<String> = self
            .problems
            .iter()
            .map(|(id, problem)| {
                let team = pipeline.teams.iter().find(|t| t.id == id.0);
                let name = team.map(|t| t.name.as_str()).filter(|n| !n.is_empty()).unwrap_or(&id.0);
                let model = team
                    .and_then(|t| t.runner.as_ref())
                    .and_then(|r| r.model.as_deref())
                    .unwrap_or("?");
                match problem {
                    RunnerConfigProblem::ModelNotAvailable => format!("{name}: model '{model}' is not available"),
                    RunnerConfigProblem::EffortNotSupported { level } => {
                        format!("{name}: effort '{level}' is not supported by {model}")
                    }
                }
            })
            .collect();
        format!("cannot start the run: {}", parts.join("; "))
    }
}

/// Check every `claude-cli` team against the list. A team with no resolved
/// runner or model is skipped: pipeline validation already rejects those.
pub fn check_pipeline(pipeline: &Pipeline, list: &ModelList) -> Result<(), PreflightFailed> {
    let problems: Vec<(TeamId, RunnerConfigProblem)> = pipeline
        .teams
        .iter()
        .filter_map(|t| {
            let r = t.runner.as_ref()?;
            if r.kind != Some(RunnerKind::ClaudeCli) {
                return None;
            }
            let model = r.model.as_deref()?;
            let effort = r.effort.clone().unwrap_or_default();
            list.check(model, &effort).err().map(|p| (TeamId(t.id.clone()), p))
        })
        .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(PreflightFailed { problems })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{Effort, RunnerKind};
    use pipeline::model::{Team, TeamRunnerConfig};
    use runners::curated_models::curated;

    fn team(id: &str, name: &str, model: &str, effort: Effort) -> Team {
        Team {
            id: id.into(),
            name: name.into(),
            prompt: format!("{id}.md"),
            scope: Default::default(),
            runner: Some(TeamRunnerConfig {
                kind: Some(RunnerKind::ClaudeCli),
                model: Some(model.into()),
                effort: Some(effort),
                api_key_env: None,
            }),
            outputs: Default::default(),
            workers: Default::default(),
            role: Default::default(),
            store: Default::default(),
        }
    }

    fn pipeline(teams: Vec<Team>) -> Pipeline {
        Pipeline {
            id: "p".into(),
            name: "P".into(),
            description: String::new(),
            schema_version: 3,
            defaults: None,
            teams,
            gates: vec![],
            escalations: vec![],
            forks: vec![],
            joins: vec![],
        }
    }

    #[test]
    fn a_valid_pipeline_passes() {
        let p = pipeline(vec![
            team("a", "A", "opus", Effort::Level("high".into())),
            team("b", "B", "haiku", Effort::Default),
        ]);
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
        assert_eq!(err.problems[0].0, TeamId("a".into()));
        let msg = err.describe(&p);
        assert!(msg.starts_with("cannot start the run: "), "{msg}");
        assert!(msg.contains("Research: model 'claude-gone' is not available"), "{msg}");
        assert!(msg.contains("Review: effort 'xhigh' is not supported by claude-opus-4-6"), "{msg}");
        assert!(!msg.contains("Fine"), "{msg}");
    }

    #[test]
    fn a_team_without_a_name_is_named_by_its_id() {
        let p = pipeline(vec![team("research", "", "claude-gone", Effort::Default)]);
        let msg = check_pipeline(&p, &curated()).unwrap_err().describe(&p);
        assert!(msg.contains("research: model 'claude-gone' is not available"), "{msg}");
    }

    #[test]
    fn anthropic_api_teams_are_not_checked() {
        let mut t = team("a", "A", "claude-api-only-id", Effort::Default);
        t.runner.as_mut().unwrap().kind = Some(RunnerKind::AnthropicApi);
        assert!(check_pipeline(&pipeline(vec![t]), &curated()).is_ok());
    }

    #[test]
    fn an_unresolved_team_is_skipped() {
        let mut t = team("a", "A", "opus", Effort::Default);
        t.runner = None;
        assert!(check_pipeline(&pipeline(vec![t]), &curated()).is_ok());
    }
}
