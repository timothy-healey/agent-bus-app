//! Pipeline aggregate invariants (context-map.md → Pipeline Authoring).
//! validate() is the gate every write must pass before a Pipeline is saved.

use crate::model::{NodeKind, Pipeline, SCHEMA_VERSION};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PipelineValidationError {
    #[error("unsupported schema_version {found} (this build supports 1 and {max})")]
    UnsupportedSchemaVersion { found: u32, max: u32 },
    #[error("fork/join nodes require schema_version: 2")]
    ForkRequiresV2,
    #[error("fork '{fork}' lane '{lane}' does not resolve to a team")]
    ForkLaneNotTeam { fork: String, lane: String },
    #[error("join '{join}' waits_for '{team}' does not resolve to a team")]
    JoinWaitsForNotTeam { join: String, team: String },
    #[error("fork '{0}' must have at least 2 lanes")]
    ForkTooFewLanes(String),
    #[error("join '{join}' downstream points at unknown node '{target}'")]
    UnresolvedJoinDownstream { join: String, target: String },
    #[error("lane entered at '{entry}' is not linear (encountered non-team '{node}' before join '{join}')")]
    LaneNotLinear { entry: String, node: String, join: String },
    #[error("fork '{fork}' lanes do not match a join's waits_for")]
    ForkJoinMismatch { fork: String },
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
    if p.schema_version != 1 && p.schema_version != SCHEMA_VERSION {
        return Err(PipelineValidationError::UnsupportedSchemaVersion {
            found: p.schema_version,
            max: SCHEMA_VERSION,
        });
    }
    if p.schema_version < 2 && (!p.forks.is_empty() || !p.joins.is_empty()) {
        return Err(PipelineValidationError::ForkRequiresV2);
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
        p.forks.iter().map(|f| (f.id.as_str(), NodeKind::Fork)).collect::<Vec<_>>(),
        p.joins.iter().map(|j| (j.id.as_str(), NodeKind::Join)).collect::<Vec<_>>(),
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

    let is_team = |id: &str| kinds.get(id) == Some(&NodeKind::Team);
    for fork in &p.forks {
        if fork.lanes.len() < 2 {
            return Err(PipelineValidationError::ForkTooFewLanes(fork.id.clone()));
        }
        for lane in &fork.lanes {
            if !is_team(lane) {
                return Err(PipelineValidationError::ForkLaneNotTeam { fork: fork.id.clone(), lane: lane.clone() });
            }
            inbound.insert(lane.as_str());
        }
    }
    for join in &p.joins {
        for team in &join.waits_for {
            if !is_team(team) {
                return Err(PipelineValidationError::JoinWaitsForNotTeam { join: join.id.clone(), team: team.clone() });
            }
        }
        if !kinds.contains_key(join.downstream.as_str()) {
            return Err(PipelineValidationError::UnresolvedJoinDownstream { join: join.id.clone(), target: join.downstream.clone() });
        }
        inbound.insert(join.downstream.as_str());
        inbound.insert(join.id.as_str());
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
            forks: vec![],
            joins: vec![],
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
        assert!(matches!(
            validate(&p),
            Err(PipelineValidationError::UnsupportedSchemaVersion { found: 99, .. })
        ));
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

    use crate::model::{Fork, Join};

    fn lane_team(id: &str, approve: &str) -> Team {
        let mut t = team(id, Some(approve));
        t.outputs.on_revise = None;
        t.outputs.on_reject = Some("needs-human".into());
        t
    }

    fn valid_v2_pipeline() -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 2,
            teams: vec![
                team("entry", Some("fork-1")),
                lane_team("lane-a", "join-1"),
                lane_team("lane-b", "join-1"),
                team("after", Some("needs-human")),
            ],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into() }],
        }
    }

    #[test]
    fn schema_version_one_is_still_accepted() {
        assert_eq!(validate(&valid_pipeline()), Ok(()));
    }

    #[test]
    fn schema_version_two_is_accepted() {
        assert_eq!(validate(&valid_v2_pipeline()), Ok(()));
    }

    #[test]
    fn an_unsupported_version_is_still_rejected() {
        let mut p = valid_v2_pipeline();
        p.schema_version = 99;
        assert!(matches!(validate(&p), Err(PipelineValidationError::UnsupportedSchemaVersion { found: 99, .. })));
    }

    #[test]
    fn v1_with_a_fork_is_rejected() {
        let mut p = valid_v2_pipeline();
        p.schema_version = 1;
        assert_eq!(validate(&p), Err(PipelineValidationError::ForkRequiresV2));
    }

    #[test]
    fn fork_lane_must_resolve_to_a_team() {
        let mut p = valid_v2_pipeline();
        p.forks[0].lanes[0] = "ghost".into();
        assert_eq!(validate(&p), Err(PipelineValidationError::ForkLaneNotTeam { fork: "fork-1".into(), lane: "ghost".into() }));
    }

    #[test]
    fn join_waits_for_must_resolve_to_a_team() {
        let mut p = valid_v2_pipeline();
        p.joins[0].waits_for[1] = "ghost".into();
        assert_eq!(validate(&p), Err(PipelineValidationError::JoinWaitsForNotTeam { join: "join-1".into(), team: "ghost".into() }));
    }

    #[test]
    fn fork_with_one_lane_is_rejected() {
        let mut p = valid_v2_pipeline();
        p.forks[0].lanes = vec!["lane-a".into()];
        p.joins[0].waits_for = vec!["lane-a".into()];
        assert_eq!(validate(&p), Err(PipelineValidationError::ForkTooFewLanes("fork-1".into())));
    }

    #[test]
    fn join_downstream_must_resolve() {
        let mut p = valid_v2_pipeline();
        p.joins[0].downstream = "ghost".into();
        assert_eq!(validate(&p), Err(PipelineValidationError::UnresolvedJoinDownstream { join: "join-1".into(), target: "ghost".into() }));
    }

    #[test]
    fn fork_target_team_is_reachable_via_fork_lane() {
        let p = valid_v2_pipeline();
        assert_eq!(validate(&p), Ok(()));
    }
}
