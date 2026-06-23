//! Pipeline router — the pure transition function. Reads the Pipeline model
//! (Authoring<->Runtime shared kernel) to decide where a settled task goes.
//! No I/O: the worker loop calls this then persists the result.

use crate::task::{TaskState, MAX_ATTEMPTS};
use agent_bus_core::Verdict;
use pipeline::model::{NodeKind, Pipeline};

/// The router's decision for a settled task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routed {
    pub next_stage: String,
    pub next_state: TaskState,
    /// Whether attempts should be bumped (true only on a revise that routes back).
    pub bump_attempts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    UnknownStage(String),
    /// The stage is a team with no route for this verdict and it's not a
    /// terminal "done" — the task escalates to needs_human.
    NoRoute,
}

fn kind_of(p: &Pipeline, id: &str) -> Option<NodeKind> {
    p.node_ids().into_iter().find(|(n, _)| n == id).map(|(_, k)| k)
}

/// Resolve the target node of a route into a (stage, state). A gate target
/// parks the task gated; an escalation target sets needs_human; a team target
/// re-queues at that team.
fn target_to_outcome(p: &Pipeline, target: &str) -> Result<(String, TaskState), RouteError> {
    match kind_of(p, target) {
        Some(NodeKind::Team) => Ok((target.to_string(), TaskState::Queued)),
        Some(NodeKind::Gate) => Ok((target.to_string(), TaskState::Gated)),
        Some(NodeKind::Escalation) => Ok((target.to_string(), TaskState::NeedsHuman)),
        // A fork target queues the task at the fork id; the POOL reads the
        // NodeKind::Fork and expands it into lane siblings (route stays pure).
        Some(NodeKind::Fork) => Ok((target.to_string(), TaskState::Queued)),
        // A join target is the barrier sentinel (Decision D1/F3): the pool/store
        // resolve it. route() stays total and never returns many.
        Some(NodeKind::Join) => Ok((target.to_string(), TaskState::Joining)),
        None => {
            // The bundled template uses a literal "done" sink for terminal
            // teams that may not be a declared node; treat it as terminal.
            if target == "done" {
                Ok((target.to_string(), TaskState::Done))
            } else {
                Err(RouteError::UnknownStage(target.to_string()))
            }
        }
    }
}

/// Route a team-stage settlement. `attempts` is the task's current attempts
/// count (pre-bump). Gates are not routed here — the operator's verdict on a
/// gate is applied by the api commands, which then call route() on the gate's
/// downstream as a fresh approve.
pub fn route(
    pipeline: &Pipeline,
    stage: &str,
    verdict: Verdict,
    attempts: u32,
) -> Result<Routed, RouteError> {
    // Gate stage: an approve forwards to downstream; revise/reject escalate.
    if let Some(gate) = pipeline.gates.iter().find(|g| g.id == stage) {
        return match verdict {
            Verdict::Approve => {
                let (next_stage, next_state) = target_to_outcome(pipeline, &gate.downstream)?;
                Ok(Routed { next_stage, next_state, bump_attempts: false })
            }
            Verdict::Revise | Verdict::Reject => {
                // Operator-driven; handled by revise_gate/reject_gate. As a pure
                // fallback, escalate.
                Ok(Routed {
                    next_stage: stage.to_string(),
                    next_state: TaskState::NeedsHuman,
                    bump_attempts: false,
                })
            }
        };
    }

    let team = pipeline
        .teams
        .iter()
        .find(|t| t.id == stage)
        .ok_or_else(|| RouteError::UnknownStage(stage.to_string()))?;

    match verdict {
        Verdict::Approve => match team.outputs.on_approve.as_deref() {
            Some(target) => {
                let (next_stage, next_state) = target_to_outcome(pipeline, target)?;
                Ok(Routed { next_stage, next_state, bump_attempts: false })
            }
            // No on_approve => terminal team (e.g. "done"): task is done.
            None => Ok(Routed { next_stage: stage.to_string(), next_state: TaskState::Done, bump_attempts: false }),
        },
        Verdict::Revise => {
            // At the cap, escalate instead of routing back.
            if attempts >= MAX_ATTEMPTS {
                let esc = team.outputs.on_reject.as_deref().unwrap_or("needs-human");
                let (next_stage, next_state) = target_to_outcome(pipeline, esc)
                    .unwrap_or((esc.to_string(), TaskState::NeedsHuman));
                return Ok(Routed { next_stage, next_state, bump_attempts: false });
            }
            match team.outputs.on_revise.as_deref() {
                Some(target) => {
                    let (next_stage, next_state) = target_to_outcome(pipeline, target)?;
                    Ok(Routed { next_stage, next_state, bump_attempts: true })
                }
                None => Ok(Routed { next_stage: stage.to_string(), next_state: TaskState::NeedsHuman, bump_attempts: false }),
            }
        }
        Verdict::Reject => {
            let target = team.outputs.on_reject.as_deref().unwrap_or("needs-human");
            let (next_stage, next_state) = target_to_outcome(pipeline, target)
                .unwrap_or((target.to_string(), TaskState::NeedsHuman));
            Ok(Routed { next_stage, next_state, bump_attempts: false })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pipeline::model::{Escalation, Gate, Routes, Scope, Team, TeamRunnerConfig, Workers};
    use agent_bus_core::{EffortMode, RunnerKind};

    fn team(id: &str, approve: Option<&str>, revise: Option<&str>, reject: Option<&str>) -> Team {
        Team {
            id: id.into(),
            name: id.into(),
            prompt: format!("prompts/{id}.md"),
            runner: Some(TeamRunnerConfig { kind: Some(RunnerKind::ClaudeCli), model: Some("m".into()), effort: Some(EffortMode::Standard), api_key_env: None }),
            scope: Scope::default(),
            outputs: Routes { on_approve: approve.map(String::from), on_revise: revise.map(String::from), on_reject: reject.map(String::from) },
            workers: Workers::default(),
        }
    }

    fn pipe() -> Pipeline {
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 1,
            defaults: None,
            teams: vec![
                team("research", Some("gate-1"), None, Some("needs-human")),
                team("writers", Some("done"), Some("research"), Some("needs-human")),
            ],
            gates: vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "writers".into() }],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![],
            joins: vec![],
        }
    }

    fn pipe_v2() -> Pipeline {
        use pipeline::model::{Fork, Join};
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 2,
            defaults: None,
            teams: vec![
                team("entry", Some("fork-1"), None, Some("needs-human")),
                team("lane-a", Some("join-1"), None, Some("needs-human")),
                team("lane-b", Some("join-1"), None, Some("needs-human")),
                team("after", Some("done"), None, Some("needs-human")),
            ],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: None }],
        }
    }

    #[test]
    fn approve_into_fork_queues_at_the_fork() {
        let r = route(&pipe_v2(), "entry", Verdict::Approve, 1).unwrap();
        assert_eq!(r.next_stage, "fork-1");
        assert_eq!(r.next_state, TaskState::Queued);
        assert!(!r.bump_attempts);
    }

    #[test]
    fn approve_into_join_yields_the_joining_barrier_outcome() {
        let r = route(&pipe_v2(), "lane-a", Verdict::Approve, 1).unwrap();
        assert_eq!(r.next_stage, "join-1");
        assert_eq!(r.next_state, TaskState::Joining);
        assert!(!r.bump_attempts);
    }

    #[test]
    fn approve_into_gate_parks_gated() {
        let r = route(&pipe(), "research", Verdict::Approve, 1).unwrap();
        assert_eq!(r.next_stage, "gate-1");
        assert_eq!(r.next_state, TaskState::Gated);
        assert!(!r.bump_attempts);
    }

    #[test]
    fn gate_approve_forwards_to_downstream_team() {
        let r = route(&pipe(), "gate-1", Verdict::Approve, 1).unwrap();
        assert_eq!(r.next_stage, "writers");
        assert_eq!(r.next_state, TaskState::Queued);
    }

    #[test]
    fn revise_routes_back_and_bumps_attempts() {
        let r = route(&pipe(), "writers", Verdict::Revise, 1).unwrap();
        assert_eq!(r.next_stage, "research");
        assert_eq!(r.next_state, TaskState::Queued);
        assert!(r.bump_attempts);
    }

    #[test]
    fn revise_at_cap_escalates_without_bump() {
        let r = route(&pipe(), "writers", Verdict::Revise, MAX_ATTEMPTS).unwrap();
        assert_eq!(r.next_stage, "needs-human");
        assert_eq!(r.next_state, TaskState::NeedsHuman);
        assert!(!r.bump_attempts);
    }

    #[test]
    fn reject_escalates() {
        let r = route(&pipe(), "research", Verdict::Reject, 1).unwrap();
        assert_eq!(r.next_stage, "needs-human");
        assert_eq!(r.next_state, TaskState::NeedsHuman);
    }

    #[test]
    fn approve_into_literal_done_is_terminal() {
        let r = route(&pipe(), "writers", Verdict::Approve, 1).unwrap();
        assert_eq!(r.next_state, TaskState::Done);
    }

    #[test]
    fn unknown_stage_is_an_error() {
        assert_eq!(route(&pipe(), "ghost", Verdict::Approve, 1), Err(RouteError::UnknownStage("ghost".into())));
    }
}
