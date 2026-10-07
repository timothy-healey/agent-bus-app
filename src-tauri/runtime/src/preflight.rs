//! Pre-flight check at run start: every `claude-cli` team's effective model and
//! effort must be in the current model list, its plugins must resolve, its
//! scope paths must become absolute, and a Remote git grant needs a model with
//! auto mode. Otherwise the run does not start. The CLI never validates
//! `--effort` (an unsupported level is silently ignored), so this is where an
//! invalid combination is caught.

use agent_bus_core::{ModelList, RunnerConfigProblem, RunnerKind, TeamId, ToolGrant};
use pipeline::model::Pipeline;
use std::path::Path;
use workspace::paths::{resolve_absolute, PathVars};

/// What the scope checks need beyond the pipeline and the model list.
#[derive(Clone, Copy)]
pub struct PreflightEnv<'a> {
    pub project_root: &'a Path,
    /// The project's target repo, which `${target_repo}` and relative scope
    /// paths resolve against.
    pub target_repo: Option<&'a Path>,
    /// Whether a declared plugin resolves. `None` skips the plugin check.
    pub plugin_exists: Option<&'a dyn Fn(&str) -> bool>,
}

impl<'a> PreflightEnv<'a> {
    /// No scope context: only model, effort and Remote git are checked against
    /// a repo-less project root.
    pub fn bare(project_root: &'a Path) -> Self {
        Self { project_root, target_repo: None, plugin_exists: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightFailed {
    pub problems: Vec<(TeamId, RunnerConfigProblem)>,
}

impl PreflightFailed {
    /// One message naming each offending team (by name, else id) and why.
    pub fn describe(&self, pipeline: &Pipeline) -> String {
        self.describe_with(pipeline, true)
    }

    /// As `describe`; without a target repo, an unresolvable path that needs
    /// one says so.
    pub fn describe_with(&self, pipeline: &Pipeline, has_target_repo: bool) -> String {
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
                    RunnerConfigProblem::PluginNotFound { name: plugin } => {
                        format!("{name}: plugin '{plugin}' is not installed")
                    }
                    RunnerConfigProblem::PathUnresolvable { pattern } => {
                        let needs_repo = pattern.contains("${target_repo}")
                            || !(pattern.starts_with('/') || pattern.starts_with("${"));
                        if !has_target_repo && needs_repo {
                            format!("{name}: scope path '{pattern}' needs a target repo; set one in Settings")
                        } else {
                            format!("{name}: scope path '{pattern}' cannot be resolved")
                        }
                    }
                    RunnerConfigProblem::RemoteGitWithoutAutoMode => {
                        format!("{name}: Remote git needs a model with auto mode, and {model} has none")
                    }
                }
            })
            .collect();
        format!("cannot start the run: {}", parts.join("; "))
    }
}

/// Check every `claude-cli` team against the list and its scope. A team with
/// no resolved runner or model is skipped: pipeline validation already
/// rejects those. A team may have several problems; each is reported.
pub fn check_pipeline(pipeline: &Pipeline, list: &ModelList, env: &PreflightEnv) -> Result<(), PreflightFailed> {
    let mut problems: Vec<(TeamId, RunnerConfigProblem)> = Vec::new();
    for t in &pipeline.teams {
        let Some(r) = t.runner.as_ref() else { continue };
        if r.kind != Some(RunnerKind::ClaudeCli) {
            continue;
        }
        let Some(model) = r.model.as_deref() else { continue };
        let id = || TeamId(t.id.clone());
        let effort = r.effort.clone().unwrap_or_default();
        if let Err(p) = list.check(model, &effort) {
            problems.push((id(), p));
        }
        if let Some(exists) = env.plugin_exists {
            for name in &t.scope.plugins {
                if !exists(name) {
                    problems.push((id(), RunnerConfigProblem::PluginNotFound { name: name.clone() }));
                }
            }
        }
        // `${task_id}` is per task; any value proves the pattern resolves.
        let mut vars = PathVars::new(env.project_root).with_task_id("T");
        if let Some(repo) = env.target_repo {
            vars = vars.with_target_repo(repo);
        }
        for pattern in t.scope.reads.iter().chain(t.scope.writes.iter()) {
            if resolve_absolute(pattern, &vars).is_err() {
                problems.push((id(), RunnerConfigProblem::PathUnresolvable { pattern: pattern.clone() }));
            }
        }
        if t.scope.grants.contains(&ToolGrant::RemoteGit) && !list.supports_auto_mode(model) {
            problems.push((id(), RunnerConfigProblem::RemoteGitWithoutAutoMode));
        }
    }
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

    fn env() -> PreflightEnv<'static> {
        PreflightEnv { project_root: Path::new("/p"), target_repo: Some(Path::new("/repo")), plugin_exists: None }
    }

    fn installed(name: &str) -> bool {
        name == "superpowers"
    }

    #[test]
    fn a_plugin_that_does_not_resolve_blocks_the_run_and_names_the_team() {
        let mut t = team("w", "Writers", "opus", Effort::Default);
        t.scope.plugins = vec!["superpowers".into(), "ghost".into()];
        let p = pipeline(vec![t]);
        let e = PreflightEnv { plugin_exists: Some(&installed), ..env() };
        let err = check_pipeline(&p, &curated(), &e).unwrap_err();
        assert_eq!(err.problems, vec![(TeamId("w".into()), RunnerConfigProblem::PluginNotFound { name: "ghost".into() })]);
        assert!(err.describe(&p).contains("Writers: plugin 'ghost' is not installed"), "{}", err.describe(&p));
    }

    #[test]
    fn an_unresolvable_scope_path_blocks_the_run() {
        let mut t = team("r", "Research", "opus", Effort::Default);
        t.scope.reads = vec!["${target_repo}".into(), "${nope}/x".into(), "docs".into()];
        let p = pipeline(vec![t]);
        // With a target repo, only the unknown variable fails.
        let err = check_pipeline(&p, &curated(), &env()).unwrap_err();
        assert_eq!(err.problems, vec![(TeamId("r".into()), RunnerConfigProblem::PathUnresolvable { pattern: "${nope}/x".into() })]);
        assert!(err.describe(&p).contains("Research: scope path '${nope}/x' cannot be resolved"));
        // Without one, `${target_repo}` and the relative path fail too, and the
        // message says what to do.
        let bare = PreflightEnv::bare(Path::new("/p"));
        let err = check_pipeline(&p, &curated(), &bare).unwrap_err();
        assert_eq!(err.problems.len(), 3);
        let msg = err.describe_with(&p, false);
        assert!(msg.contains("Research: scope path '${target_repo}' needs a target repo; set one in Settings"), "{msg}");
    }

    #[test]
    fn remote_git_on_a_model_without_auto_mode_blocks_the_run() {
        let mut t = team("i", "Implementers", "haiku", Effort::Default);
        t.scope.grants = vec![agent_bus_core::ToolGrant::Bash, agent_bus_core::ToolGrant::RemoteGit];
        let p = pipeline(vec![t.clone()]);
        let err = check_pipeline(&p, &curated(), &env()).unwrap_err();
        assert_eq!(err.problems, vec![(TeamId("i".into()), RunnerConfigProblem::RemoteGitWithoutAutoMode)]);
        assert!(err.describe(&p).contains("Implementers: Remote git needs a model with auto mode, and haiku has none"));
        t.runner.as_mut().unwrap().model = Some("sonnet".into());
        assert!(check_pipeline(&pipeline(vec![t]), &curated(), &env()).is_ok());
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
        assert!(check_pipeline(&p, &curated(), &env()).is_ok());
    }

    #[test]
    fn every_offending_team_is_named_with_its_reason() {
        let p = pipeline(vec![
            team("a", "Research", "claude-gone", Effort::Default),
            team("b", "Review", "claude-opus-4-6", Effort::Level("xhigh".into())),
            team("c", "Fine", "haiku", Effort::Default),
        ]);
        let err = check_pipeline(&p, &curated(), &env()).unwrap_err();
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
        let msg = check_pipeline(&p, &curated(), &env()).unwrap_err().describe(&p);
        assert!(msg.contains("research: model 'claude-gone' is not available"), "{msg}");
    }

    #[test]
    fn anthropic_api_teams_are_not_checked() {
        let mut t = team("a", "A", "claude-api-only-id", Effort::Default);
        t.runner.as_mut().unwrap().kind = Some(RunnerKind::AnthropicApi);
        assert!(check_pipeline(&pipeline(vec![t]), &curated(), &env()).is_ok());
    }

    #[test]
    fn an_unresolved_team_is_skipped() {
        let mut t = team("a", "A", "opus", Effort::Default);
        t.runner = None;
        assert!(check_pipeline(&pipeline(vec![t]), &curated(), &env()).is_ok());
    }
}
