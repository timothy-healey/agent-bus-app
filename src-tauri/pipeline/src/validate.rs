//! Pipeline aggregate invariants (context-map.md → Pipeline Authoring).
//! validate() is the gate every write must pass before a Pipeline is saved.

use crate::model::{NodeKind, Pipeline, SCHEMA_VERSION};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PipelineValidationError {
    #[error("unsupported schema_version {found} (this build supports {supported})")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    #[error("duplicate node id: {0}")]
    DuplicateNodeId(String),
    #[error("route on node '{node}' ({route}) points at unknown node '{target}'")]
    UnresolvedRoute { node: String, route: String, target: String },
    #[error("node '{0}' routes to itself")]
    SelfReference(String),
    #[error("gate '{gate}' downstream points at unknown node '{target}'")]
    UnresolvedGateDownstream { gate: String, target: String },
    #[error("team '{0}' is unreachable (no route or gate-downstream points at it)")]
    UnreachableTeam(String),
    #[error("pipeline has no teams")]
    NoTeams,
}

/// Validate a Pipeline against the aggregate invariants. Returns Ok(()) when
/// the graph is well-formed.
pub fn validate(p: &Pipeline) -> Result<(), PipelineValidationError> {
    // schema_version supported
    if p.schema_version != SCHEMA_VERSION {
        return Err(PipelineValidationError::UnsupportedSchemaVersion {
            found: p.schema_version,
            supported: SCHEMA_VERSION,
        });
    }

    if p.teams.is_empty() {
        return Err(PipelineValidationError::NoTeams);
    }

    // unique node ids across all kinds
    let mut kinds: HashMap<&str, NodeKind> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for (id, kind) in [
        p.teams.iter().map(|t| (t.id.as_str(), NodeKind::Team)).collect::<Vec<_>>(),
        p.gates.iter().map(|g| (g.id.as_str(), NodeKind::Gate)).collect::<Vec<_>>(),
        p.escalations.iter().map(|e| (e.id.as_str(), NodeKind::Escalation)).collect::<Vec<_>>(),
    ]
    .concat()
    {
        if !seen.insert(id) {
            return Err(PipelineValidationError::DuplicateNodeId(id.to_string()));
        }
        kinds.insert(id, kind);
    }

    // collect every edge target, tracking which nodes receive an inbound edge
    let mut inbound: HashSet<&str> = HashSet::new();

    for team in &p.teams {
        for (route, target) in [
            ("on_approve", team.outputs.on_approve.as_deref()),
            ("on_revise", team.outputs.on_revise.as_deref()),
            ("on_reject", team.outputs.on_reject.as_deref()),
        ] {
            if let Some(target) = target {
                if target == team.id {
                    return Err(PipelineValidationError::SelfReference(team.id.clone()));
                }
                if !kinds.contains_key(target) {
                    return Err(PipelineValidationError::UnresolvedRoute {
                        node: team.id.clone(),
                        route: route.to_string(),
                        target: target.to_string(),
                    });
                }
                inbound.insert(target);
            }
        }
    }

    for gate in &p.gates {
        if gate.downstream == gate.id {
            return Err(PipelineValidationError::SelfReference(gate.id.clone()));
        }
        if !kinds.contains_key(gate.downstream.as_str()) {
            return Err(PipelineValidationError::UnresolvedGateDownstream {
                gate: gate.id.clone(),
                target: gate.downstream.clone(),
            });
        }
        inbound.insert(gate.downstream.as_str());
    }

    // reachability: every team except the first declared (the entry team) must
    // receive at least one inbound edge. The first team is the entry point and
    // is reachable by definition (inject drops topics into it).
    for team in p.teams.iter().skip(1) {
        if !inbound.contains(team.id.as_str()) {
            return Err(PipelineValidationError::UnreachableTeam(team.id.clone()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Escalation, Gate, Routes, RunnerConfig, Scope, Team, Workers};
    use agent_bus_core::{EffortMode, RunnerKind};

    fn team(id: &str, approve: Option<&str>) -> Team {
        Team {
            id: id.into(),
            name: id.into(),
            prompt: format!("prompts/{id}.md"),
            runner: RunnerConfig {
                kind: RunnerKind::ClaudeCli,
                model: "m".into(),
                effort: EffortMode::Standard,
                api_key_env: None,
            },
            scope: Scope::default(),
            outputs: Routes { on_approve: approve.map(String::from), on_revise: None, on_reject: None },
            workers: Workers::default(),
        }
    }

    fn valid_pipeline() -> Pipeline {
        Pipeline {
            id: "p".into(),
            name: "P".into(),
            description: String::new(),
            schema_version: 1,
            teams: vec![team("research", Some("gate-1")), team("writers", Some("needs-human"))],
            gates: vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "writers".into() }],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
        }
    }

    #[test]
    fn a_well_formed_pipeline_validates() {
        assert_eq!(validate(&valid_pipeline()), Ok(()));
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let mut p = valid_pipeline();
        p.schema_version = 99;
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::UnsupportedSchemaVersion { found: 99, supported: 1 })
        );
    }

    #[test]
    fn duplicate_node_id_is_rejected() {
        let mut p = valid_pipeline();
        p.teams[1].id = "research".into();
        assert_eq!(validate(&p), Err(PipelineValidationError::DuplicateNodeId("research".into())));
    }

    #[test]
    fn unresolved_route_is_rejected() {
        let mut p = valid_pipeline();
        p.teams[0].outputs.on_approve = Some("nowhere".into());
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::UnresolvedRoute {
                node: "research".into(),
                route: "on_approve".into(),
                target: "nowhere".into(),
            })
        );
    }

    #[test]
    fn self_reference_is_rejected() {
        let mut p = valid_pipeline();
        p.teams[0].outputs.on_revise = Some("research".into());
        assert_eq!(validate(&p), Err(PipelineValidationError::SelfReference("research".into())));
    }

    #[test]
    fn unreachable_team_is_rejected() {
        let mut p = valid_pipeline();
        // gate now points back at research, so 'writers' has no inbound edge
        p.gates[0].downstream = "research".into();
        // but research already routes to gate-1, and writers -> needs-human;
        // writers receives nothing inbound now.
        assert_eq!(validate(&p), Err(PipelineValidationError::UnreachableTeam("writers".into())));
    }

    #[test]
    fn empty_pipeline_is_rejected() {
        let mut p = valid_pipeline();
        p.teams.clear();
        assert_eq!(validate(&p), Err(PipelineValidationError::NoTeams));
    }
}
