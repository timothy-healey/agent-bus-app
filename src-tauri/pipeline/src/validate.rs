//! Pipeline aggregate invariants (context-map.md → Pipeline Authoring).
//! validate() is the gate every write must pass before a Pipeline is saved.

use crate::model::{NodeKind, Pipeline, SCHEMA_VERSION};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PipelineValidationError {
    #[error("unsupported schema_version {found} (this build supports 1..={max})")]
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
    #[error("pipeline has no source team (every team has an inbound route; exactly one entry/source is required)")]
    NoSource,
    #[error("pipeline has multiple source teams {0:?} (exactly one entry/source is required)")]
    MultipleSources(Vec<String>),
    #[error("team '{0}' is not reachable from the source via routes")]
    StoreUnreachable(String),
    #[error("pipeline has no teams")]
    NoTeams,
    #[error("team '{0}' has no resolvable runner (no model from the team or pipeline defaults)")]
    TeamHasNoRunner(String),
    #[error("join '{join}' quorum {quorum} out of range (must be 1..={lanes})")]
    QuorumOutOfRange { join: String, quorum: u32, lanes: u32 },
    #[error("fork nesting exceeds the maximum depth of {max} (fork '{fork}' is at depth {depth})")]
    NestingTooDeep { fork: String, depth: u32, max: u32 },
    #[error("team '{team}' store.capacity must be >= 1")]
    StoreCapacityZero { team: String },
}

/// Max fork nesting depth (DD-P1-5). A top-level fork is depth 1; a fork reached
/// from inside another fork's lane is depth 2; etc.
const MAX_NESTING_DEPTH: u32 = 3;

/// Walk a fork lane from its entry forward toward the join. P1: a lane may
/// contain gates and nested forks (no longer strictly linear). A team hop
/// follows on_approve; a gate hop follows the gate's downstream; a fork hop
/// recurses — each nested lane must reach the nested fork's paired join, whose
/// downstream continues the walk. Reaching `join_id` is success. `depth` tracks
/// fork nesting for the bound. Bounded by node count to terminate.
fn check_lane_reachable(
    p: &Pipeline,
    kinds: &HashMap<&str, NodeKind>,
    entry: &str,
    join_id: &str,
    depth: u32,
) -> Result<(), PipelineValidationError> {
    let bound = p.teams.len() + p.gates.len() + p.forks.len() + p.joins.len() + 1;
    let mut current = entry.to_string();
    for _ in 0..=bound {
        if current == join_id {
            return Ok(());
        }
        match kinds.get(current.as_str()) {
            Some(NodeKind::Team) => {
                let team = p.teams.iter().find(|t| t.id == current).unwrap();
                match team.outputs.on_approve.as_deref() {
                    Some(next) => current = next.to_string(),
                    None => return Err(PipelineValidationError::LaneNotLinear {
                        entry: entry.to_string(), node: current.clone(), join: join_id.to_string() }),
                }
            }
            Some(NodeKind::Gate) => {
                let gate = p.gates.iter().find(|g| g.id == current).unwrap();
                current = gate.downstream.clone();
            }
            Some(NodeKind::Fork) => {
                if depth + 1 > MAX_NESTING_DEPTH {
                    return Err(PipelineValidationError::NestingTooDeep {
                        fork: current.clone(), depth: depth + 1, max: MAX_NESTING_DEPTH });
                }
                let fork = p.forks.iter().find(|f| f.id == current).unwrap();
                // Pair the nested fork with the join whose lanes are all reachable
                // from its lanes (one deeper level). Continue from that downstream.
                // A NestingTooDeep encountered while resolving a nested lane must
                // surface rather than be masked as a bare mismatch — probe the
                // lanes against the candidate join and propagate that error.
                let nested_join = p.joins.iter().find(|j| {
                    fork.lanes.iter().all(|lane| check_lane_reachable(p, kinds, lane, &j.id, depth + 1).is_ok())
                });
                match nested_join {
                    Some(j) => current = j.downstream.clone(),
                    None => {
                        for j in &p.joins {
                            for lane in &fork.lanes {
                                if let Err(e @ PipelineValidationError::NestingTooDeep { .. }) =
                                    check_lane_reachable(p, kinds, lane, &j.id, depth + 1)
                                {
                                    return Err(e);
                                }
                            }
                        }
                        return Err(PipelineValidationError::ForkJoinMismatch { fork: current.clone() });
                    }
                }
            }
            // An escalation, join (other than the target), or unknown node ends a
            // lane that never reaches its join.
            _ => return Err(PipelineValidationError::LaneNotLinear {
                entry: entry.to_string(), node: current.clone(), join: join_id.to_string() }),
        }
    }
    Err(PipelineValidationError::LaneNotLinear {
        entry: entry.to_string(), node: current, join: join_id.to_string() })
}

/// Validate a Pipeline against the aggregate invariants. Returns Ok(()) when
/// the graph is well-formed.
pub fn validate(p: &Pipeline) -> Result<(), PipelineValidationError> {
    // schema_version supported (1..=SCHEMA_VERSION; older versions still load)
    if p.schema_version < 1 || p.schema_version > SCHEMA_VERSION {
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

    // Store capacity is a WIP limit: a zero-capacity store can never admit work.
    for t in &p.teams {
        if t.store.capacity == 0 {
            return Err(PipelineValidationError::StoreCapacityZero { team: t.id.clone() });
        }
    }

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
        if let Some(q) = join.quorum {
            let lanes = join.waits_for.len() as u32;
            if q < 1 || q > lanes {
                return Err(PipelineValidationError::QuorumOutOfRange { join: join.id.clone(), quorum: q, lanes });
            }
        }
        inbound.insert(join.downstream.as_str());
        inbound.insert(join.id.as_str());
    }

    for fork in &p.forks {
        let paired = p.joins.iter().find(|j| {
            fork.lanes.iter().all(|lane| check_lane_reachable(p, &kinds, lane, &j.id, 1).is_ok())
        });
        match paired {
            Some(_join) => {}
            None => {
                // Surface the most specific lane error (depth/linearity) instead
                // of a bare mismatch when possible.
                if let Some(j) = p.joins.first() {
                    for lane in &fork.lanes {
                        check_lane_reachable(p, &kinds, lane, &j.id, 1)?;
                    }
                }
                return Err(PipelineValidationError::ForkJoinMismatch { fork: fork.id.clone() });
            }
        }
    }

    // reachability: every team except the first declared (the entry team) must
    // receive at least one inbound edge. The first team is the entry point and
    // is reachable by definition (inject drops topics into it).
    for team in p.teams.iter().skip(1) {
        if !inbound.contains(team.id.as_str()) {
            return Err(PipelineValidationError::UnreachableTeam(team.id.clone()));
        }
    }

    // Assembly-line designation (④a, deferred from chunk ①): exactly one team
    // has no *forward* inbound edge — that team is the source/generator stage (no
    // input store; it produces work-items by scanning, loop-until-dry). Forward
    // edges are the assembly-line flow: team on_approve, gate downstream, fork
    // lane, join downstream. on_revise/on_reject are FEEDBACK/escalation edges
    // (a reviewer sending an item *back* to its writer), not upstream supply —
    // they must not disqualify a team from being the source (the writer a
    // reviewer revises-back-to is still the entry). Zero forward-sources means a
    // forward cycle with no entry (NoSource); more than one means ambiguous entry
    // (MultipleSources). v1 supports a single source; mid-pipeline generators are
    // a future extension.
    let forward_inbound = forward_inbound_teams(p);
    let sources: Vec<String> = p
        .teams
        .iter()
        .filter(|t| !forward_inbound.contains(t.id.as_str()))
        .map(|t| t.id.clone())
        .collect();
    match sources.len() {
        0 => return Err(PipelineValidationError::NoSource),
        1 => {}
        _ => return Err(PipelineValidationError::MultipleSources(sources)),
    }
    let source = &sources[0];

    // Store reachability: every non-source team's input store must be fed — i.e.
    // every team is reachable from the source by following forward edges. This is
    // stronger than the per-edge inbound check above: it rejects a team fed only
    // by an island disconnected from the source. Walk the forward graph from the
    // source and assert every team is visited.
    let reachable = forward_reachable_from(p, &kinds, source);
    for team in &p.teams {
        if !reachable.contains(team.id.as_str()) {
            return Err(PipelineValidationError::StoreUnreachable(team.id.clone()));
        }
    }

    Ok(())
}

/// The set of team ids that receive a *forward* (assembly-line) inbound edge:
/// a team on_approve, a gate downstream, a fork lane, or a join downstream. The
/// source/generator stage is the team with NO forward inbound. on_revise and
/// on_reject are deliberately excluded — they are feedback/escalation edges, not
/// upstream supply (a reviewer revising back to its writer must not make the
/// writer look downstream-fed).
fn forward_inbound_teams(p: &Pipeline) -> HashSet<String> {
    let mut inbound: HashSet<String> = HashSet::new();
    for team in &p.teams {
        if let Some(next) = team.outputs.on_approve.as_deref() {
            inbound.insert(next.to_string());
        }
    }
    for gate in &p.gates {
        inbound.insert(gate.downstream.clone());
    }
    for fork in &p.forks {
        for lane in &fork.lanes {
            inbound.insert(lane.clone());
        }
    }
    for join in &p.joins {
        inbound.insert(join.downstream.clone());
    }
    inbound
}

/// The set of node ids reachable from `start` by following forward (assembly-
/// line) edges: team on_approve, gate downstreams, fork lanes, and join
/// downstreams. on_revise/on_reject are excluded (feedback/escalation, not
/// supply). Used for store reachability — a team in the result has its input
/// store fed from the source. Bounded BFS over the node set.
fn forward_reachable_from<'a>(
    p: &'a Pipeline,
    kinds: &HashMap<&'a str, NodeKind>,
    start: &str,
) -> HashSet<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = vec![start.to_string()];
    while let Some(node) = stack.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }
        let mut push = |target: &str| {
            if !seen.contains(target) {
                stack.push(target.to_string());
            }
        };
        match kinds.get(node.as_str()) {
            Some(NodeKind::Team) => {
                if let Some(t) = p.teams.iter().find(|t| t.id == node) {
                    // Forward flow only — on_approve. revise/reject are feedback.
                    if let Some(next) = t.outputs.on_approve.as_deref() {
                        push(next);
                    }
                }
            }
            Some(NodeKind::Gate) => {
                if let Some(g) = p.gates.iter().find(|g| g.id == node) {
                    push(&g.downstream);
                }
            }
            Some(NodeKind::Fork) => {
                if let Some(f) = p.forks.iter().find(|f| f.id == node) {
                    for lane in &f.lanes {
                        push(lane);
                    }
                }
            }
            Some(NodeKind::Join) => {
                if let Some(j) = p.joins.iter().find(|j| j.id == node) {
                    push(&j.downstream);
                }
            }
            // Escalations are terminal; unknown nodes are caught upstream.
            _ => {}
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Escalation, Gate, Role, Routes, Scope, Store, Team, TeamRunnerConfig, Workers};
    use agent_bus_core::{EffortMode, RunnerKind};

    fn team(id: &str, approve: Option<&str>) -> Team {
        Team {
            id: id.into(),
            name: id.into(),
            prompt: format!("prompts/{id}.md"),
            runner: Some(TeamRunnerConfig {
                kind: Some(RunnerKind::ClaudeCli),
                model: Some("m".into()),
                effort: Some(EffortMode::Standard),
                api_key_env: None,
            }),
            scope: Scope::default(),
            outputs: Routes { on_approve: approve.map(String::from), on_revise: None, on_reject: None },
            workers: Workers::default(),
            role: Role::default(),
            store: Store::default(),
        }
    }

    fn valid_pipeline() -> Pipeline {
        Pipeline {
            id: "p".into(),
            name: "P".into(),
            description: String::new(),
            schema_version: 1,
            defaults: None,
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
    fn rejects_zero_store_capacity() {
        let mut p = valid_pipeline();
        p.teams[0].store.capacity = 0;
        let err = validate(&p).unwrap_err();
        assert!(format!("{err:?}").contains("StoreCapacity"));
    }

    #[test]
    fn accepts_schema_version_three() {
        let mut p = valid_pipeline();
        p.schema_version = 3;
        assert!(validate(&p).is_ok());
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
    fn the_bundled_shape_has_exactly_one_source_and_validates() {
        // The valid v1 + v2 shapes each have a single source (research / entry)
        // and every team reachable from it.
        assert_eq!(validate(&valid_pipeline()), Ok(()));
        assert_eq!(validate(&valid_v2_pipeline()), Ok(()));
    }

    #[test]
    fn two_source_pipeline_is_rejected() {
        // Two teams with no FORWARD inbound: 'research' (entry) and a second
        // generator 'gen2'. gen2 is given a revise-back inbound so it still passes
        // the per-edge UnreachableTeam check, but it has no on_approve/gate/fork/
        // join feeding it -> it is a second source -> MultipleSources.
        let mut p = valid_pipeline();
        // gen2 flows forward into writers (so writers stays reachable from a
        // source); research also flows into gate-1 -> writers.
        let mut gen2 = team("gen2", Some("writers"));
        gen2.outputs.on_reject = Some("needs-human".into());
        p.teams.push(gen2);
        // someone revises back to gen2, giving it a (feedback) inbound edge.
        p.teams[1].outputs.on_revise = Some("gen2".into()); // writers.on_revise -> gen2
        let err = validate(&p).unwrap_err();
        assert!(
            matches!(err, PipelineValidationError::MultipleSources(_)),
            "expected MultipleSources, got {err:?}"
        );
    }

    #[test]
    fn no_source_pipeline_is_rejected() {
        // Every team has a FORWARD inbound edge (a 2-team forward cycle): no
        // entry/source.
        let mut p = valid_pipeline();
        p.gates.clear();
        p.teams = vec![team("a", Some("b")), team("b", Some("a"))];
        assert_eq!(validate(&p), Err(PipelineValidationError::NoSource));
    }

    #[test]
    fn a_team_unreachable_from_the_source_is_rejected() {
        // research (source) -> gate-1 -> writers. Add an island pair that flows
        // forward into each other (so each has a forward inbound -> neither is a
        // second source, and each has a per-edge inbound -> passes UnreachableTeam)
        // but is NOT reachable from the source.
        let mut p = valid_pipeline();
        p.teams.push(team("island-a", Some("island-b")));
        p.teams.push(team("island-b", Some("island-a")));
        let err = validate(&p).unwrap_err();
        assert!(
            matches!(err, PipelineValidationError::StoreUnreachable(_)),
            "expected StoreUnreachable, got {err:?}"
        );
    }

    #[test]
    fn empty_pipeline_is_rejected() {
        let mut p = valid_pipeline();
        p.teams.clear();
        assert_eq!(validate(&p), Err(PipelineValidationError::NoTeams));
    }

    #[test]
    fn a_team_with_no_resolvable_model_is_rejected() {
        let mut p = valid_pipeline();
        // strip the model so it cannot resolve (no pipeline default either)
        p.teams[0].runner = Some(TeamRunnerConfig { model: None, ..Default::default() });
        assert_eq!(
            validate(&p),
            Err(PipelineValidationError::TeamHasNoRunner("research".into()))
        );
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
            defaults: None,
            teams: vec![
                team("entry", Some("fork-1")),
                lane_team("lane-a", "join-1"),
                lane_team("lane-b", "join-1"),
                team("after", Some("needs-human")),
            ],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: None }],
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

    use crate::model::Gate as GateNode;

    #[test]
    fn a_gate_inside_a_lane_is_now_accepted() {
        // P1: a lane may contain a gate. lane-a -> gate-x -> join-1.
        let mut p = valid_v2_pipeline();
        p.teams[1].outputs.on_approve = Some("gate-x".into());
        p.gates.push(GateNode { id: "gate-x".into(), label: "X".into(), downstream: "join-1".into() });
        assert_eq!(validate(&p), Ok(()));
    }

    #[test]
    fn a_nested_fork_inside_a_lane_is_now_accepted() {
        // P1: lane-a is itself a fork. lane-a -> fork-2 {n1,n2} -> join-2 -> join-1.
        let mut p = valid_v2_pipeline();
        p.teams[1].outputs.on_approve = Some("fork-2".into());
        p.teams.push(lane_team("n1", "join-2"));
        p.teams.push(lane_team("n2", "join-2"));
        p.forks.push(Fork { id: "fork-2".into(), lanes: vec!["n1".into(), "n2".into()] });
        p.joins.push(Join { id: "join-2".into(), waits_for: vec!["n1".into(), "n2".into()], downstream: "join-1".into(), cancel_on_reject: false, quorum: None });
        assert_eq!(validate(&p), Ok(()));
    }

    #[test]
    fn nesting_deeper_than_the_bound_is_rejected() {
        // 4 levels of fork nesting exceeds the max depth (3).
        let mut p = valid_v2_pipeline();
        // lane-a -> fork-2 -> fork-3 -> fork-4 (each a single nested fork inside the prior lane)
        p.teams[1].outputs.on_approve = Some("fork-2".into());
        // fork-2 lanes
        p.teams.push(lane_team("b1", "fork-3"));
        p.teams.push(lane_team("b2", "join-2"));
        p.forks.push(Fork { id: "fork-2".into(), lanes: vec!["b1".into(), "b2".into()] });
        p.joins.push(Join { id: "join-2".into(), waits_for: vec!["b1".into(), "b2".into()], downstream: "join-1".into(), cancel_on_reject: false, quorum: None });
        // fork-3 (depth 3) lanes
        p.teams.push(lane_team("c1", "fork-4"));
        p.teams.push(lane_team("c2", "join-3"));
        p.forks.push(Fork { id: "fork-3".into(), lanes: vec!["c1".into(), "c2".into()] });
        p.joins.push(Join { id: "join-3".into(), waits_for: vec!["c1".into(), "c2".into()], downstream: "join-2".into(), cancel_on_reject: false, quorum: None });
        // fork-4 (depth 4 — too deep) lanes
        p.teams.push(lane_team("d1", "join-4"));
        p.teams.push(lane_team("d2", "join-4"));
        p.forks.push(Fork { id: "fork-4".into(), lanes: vec!["d1".into(), "d2".into()] });
        p.joins.push(Join { id: "join-4".into(), waits_for: vec!["d1".into(), "d2".into()], downstream: "join-3".into(), cancel_on_reject: false, quorum: None });
        assert!(matches!(validate(&p), Err(PipelineValidationError::NestingTooDeep { .. })));
    }

    #[test]
    fn quorum_within_bounds_is_accepted() {
        let mut p = valid_v2_pipeline();
        p.joins[0].quorum = Some(1);
        assert_eq!(validate(&p), Ok(()));
        p.joins[0].quorum = Some(2); // == number of lanes (all-must-approve equiv)
        assert_eq!(validate(&p), Ok(()));
    }

    #[test]
    fn quorum_zero_is_rejected() {
        let mut p = valid_v2_pipeline();
        p.joins[0].quorum = Some(0);
        assert_eq!(validate(&p), Err(PipelineValidationError::QuorumOutOfRange { join: "join-1".into(), quorum: 0, lanes: 2 }));
    }

    #[test]
    fn quorum_above_lane_count_is_rejected() {
        let mut p = valid_v2_pipeline();
        p.joins[0].quorum = Some(3); // only 2 lanes
        assert_eq!(validate(&p), Err(PipelineValidationError::QuorumOutOfRange { join: "join-1".into(), quorum: 3, lanes: 2 }));
    }

    #[test]
    fn a_multi_team_linear_lane_is_accepted() {
        let mut p = valid_v2_pipeline();
        p.teams[1].outputs.on_approve = Some("lane-a2".into());
        p.teams.push(lane_team("lane-a2", "join-1"));
        p.joins[0].waits_for = vec!["lane-a2".into(), "lane-b".into()];
        assert_eq!(validate(&p), Ok(()));
    }
}
