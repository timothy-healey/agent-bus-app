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
        assert_eq!(team.runner.kind, RunnerKind::ClaudeCli);
        assert_eq!(team.runner.effort, EffortMode::ExtendedHigh);
        assert_eq!(team.outputs.on_approve.as_deref(), Some("gate-1-spec"));
        assert_eq!(team.outputs.on_revise, None);
        assert_eq!(team.workers.max, 3);
        assert_eq!(team.scope.reads.len(), 2);
    }

    #[test]
    fn schema_version_defaults_to_one_when_absent() {
        let yaml = "id: x\nname: X\nteams: []\n";
        let p = parse_pipeline(yaml).unwrap();
        assert_eq!(p.schema_version, 1);
    }

    #[test]
    fn malformed_yaml_is_a_parse_error() {
        let err = parse_pipeline("id: x\n  bad: : indent").unwrap_err();
        assert!(matches!(err, PipelineParseError::Yaml(_)));
    }

    #[test]
    fn missing_required_team_field_is_a_parse_error() {
        // team missing `runner` — serde reports a missing-field error.
        let yaml = "id: x\nname: X\nteams:\n  - id: t\n    name: T\n    prompt: p.md\n    scope: {}\n    outputs: {}\n";
        assert!(parse_pipeline(yaml).is_err());
    }
}
