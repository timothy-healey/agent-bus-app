//! YAML → Pipeline deserialisation. Validation (invariants) is separate
//! (validate.rs); this module only turns bytes into the typed model.

use crate::model::Pipeline;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipelineParseError {
    #[error("invalid pipeline YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// Parse a pipeline definition from a YAML string.
pub fn parse_pipeline(yaml: &str) -> Result<Pipeline, PipelineParseError> {
    let pipeline: Pipeline = serde_yaml::from_str(yaml)?;
    Ok(pipeline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{EffortMode, RunnerKind};

    const MINIMAL: &str = r#"
id: demo
name: Demo Pipeline
description: A two-node demo.
schema_version: 1
teams:
  - id: research
    name: Research
    prompt: prompts/research.md
    runner:
      kind: claude-cli
      model: claude-opus-4-7
      effort:
        mode: extended-high
    scope:
      reads: ["${target_repo}", "${project}/artifacts/analyses"]
      writes: ["${project}/artifacts/analyses"]
      tools: [Read, Write, Edit, Glob, Grep]
    outputs:
      on_approve: gate-1-spec
    workers:
      default: 1
      max: 3
gates:
  - id: gate-1-spec
    label: Gate 1 — Spec Approval
    downstream: needs-human
escalations:
  - id: needs-human
    triggers: ["attempts >= 3", "verdict == reject"]
"#;

    #[test]
    fn parses_a_full_pipeline() {
        let p = parse_pipeline(MINIMAL).unwrap();
        assert_eq!(p.id, "demo");
        assert_eq!(p.schema_version, 1);
        assert_eq!(p.teams.len(), 1);
        assert_eq!(p.gates.len(), 1);
        assert_eq!(p.escalations.len(), 1);

        let team = &p.teams[0];
        assert_eq!(team.id, "research");
        let runner = team.runner.as_ref().unwrap();
        assert_eq!(runner.kind, Some(RunnerKind::ClaudeCli));
        assert_eq!(runner.effort, Some(EffortMode::ExtendedHigh));
        assert_eq!(team.outputs.on_approve.as_deref(), Some("gate-1-spec"));
        assert_eq!(team.outputs.on_revise, None);
        assert_eq!(team.workers.max, 3);
        assert_eq!(team.scope.reads.len(), 2);
    }

    #[test]
    fn schema_version_defaults_to_current_when_absent() {
        let yaml = "id: x\nname: X\nteams: []\n";
        let p = parse_pipeline(yaml).unwrap();
        assert_eq!(p.schema_version, crate::model::SCHEMA_VERSION);
    }

    const V2_FORKJOIN: &str = r#"
id: parallel-demo
name: Parallel Demo
schema_version: 2
teams:
  - id: entry
    name: Entry
    prompt: prompts/entry.md
    runner: { kind: claude-cli, model: m, effort: { mode: standard } }
    scope: {}
    outputs: { on_approve: fork-1 }
  - id: reviewer-a
    name: Reviewer A
    prompt: prompts/a.md
    runner: { kind: claude-cli, model: m, effort: { mode: standard } }
    scope: {}
    outputs: { on_approve: join-1 }
  - id: reviewer-b
    name: Reviewer B
    prompt: prompts/b.md
    runner: { kind: claude-cli, model: m, effort: { mode: standard } }
    scope: {}
    outputs: { on_approve: join-1 }
forks:
  - id: fork-1
    lanes: [reviewer-a, reviewer-b]
joins:
  - id: join-1
    waits_for: [reviewer-a, reviewer-b]
    downstream: needs-human
escalations:
  - id: needs-human
"#;

    #[test]
    fn parses_a_v2_fork_join_pipeline() {
        let p = parse_pipeline(V2_FORKJOIN).unwrap();
        assert_eq!(p.schema_version, 2);
        assert_eq!(p.forks.len(), 1);
        assert_eq!(p.forks[0].lanes, vec!["reviewer-a", "reviewer-b"]);
        assert_eq!(p.joins.len(), 1);
        assert_eq!(p.joins[0].waits_for, vec!["reviewer-a", "reviewer-b"]);
        assert_eq!(p.joins[0].downstream, "needs-human");
    }

    #[test]
    fn malformed_yaml_is_a_parse_error() {
        let err = parse_pipeline("id: x\n  bad: : indent").unwrap_err();
        assert!(matches!(err, PipelineParseError::Yaml(_)));
    }

    #[test]
    fn missing_required_team_field_is_a_parse_error() {
        // team missing `prompt` (still required) — serde reports a missing-field
        // error. (`runner` is now optional/R5, so omitting it is NOT an error;
        // the team inherits the pipeline default.)
        let yaml = "id: x\nname: X\nteams:\n  - id: t\n    name: T\n    scope: {}\n    outputs: {}\n";
        assert!(parse_pipeline(yaml).is_err());
    }

    #[test]
    fn team_omitting_runner_parses_for_inheritance() {
        // R5: a team may omit its runner entirely (inherits pipeline defaults).
        let yaml = "id: x\nname: X\nteams:\n  - id: t\n    name: T\n    prompt: p.md\n    scope: {}\n    outputs: {}\n";
        let p = parse_pipeline(yaml).unwrap();
        assert!(p.teams[0].runner.is_none());
    }
}
