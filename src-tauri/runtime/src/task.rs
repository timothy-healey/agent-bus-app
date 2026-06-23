//! The Task aggregate (context-map.md → Runtime). Root identity task_id. This
//! module owns the state machine + invariants; TaskStore (task_store.rs)
//! persists it. Two aggregates in Runtime (Task + WorkerPool) joined by
//! reference — a Task carries no Worker, only its own claim status via `state`.

use agent_bus_core::{TaskId, Verdict};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_ATTEMPTS: u32 = 3;

/// The spec's seven task states. Serialises to the exact strings stored in the
/// `tasks.state` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Gated,
    Revising,
    NeedsHuman,
    Done,
    Braked,
    /// Transient router→pool signal: an approve hit a join; the pool resolves the
    /// barrier. Never rests on a persisted task row (Decision D1/D7). Serialises
    /// to "joining" for completeness only.
    Joining,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Queued => "queued",
            TaskState::Running => "running",
            TaskState::Gated => "gated",
            TaskState::Revising => "revising",
            TaskState::NeedsHuman => "needs_human",
            TaskState::Done => "done",
            TaskState::Braked => "braked",
            TaskState::Joining => "joining",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => TaskState::Queued,
            "running" => TaskState::Running,
            "gated" => TaskState::Gated,
            "revising" => TaskState::Revising,
            "needs_human" => TaskState::NeedsHuman,
            "done" => TaskState::Done,
            "braked" => TaskState::Braked,
            "joining" => TaskState::Joining,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: String,
    pub pipeline: String,
    pub topic: String,
    pub target_repo: Option<String>,
    pub target_scope: Option<String>,
    pub current_stage: String,
    pub state: TaskState,
    pub attempts: u32,
    pub parent_artifact: Option<String>,
    pub review_artifact: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub lane: Option<String>,
    #[serde(default)]
    pub join_target: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TaskTransitionError {
    #[error("a done task is immutable")]
    DoneIsImmutable,
    #[error("illegal transition from {from:?} to {to:?}")]
    Illegal { from: TaskState, to: TaskState },
    #[error("attempts cap ({MAX_ATTEMPTS}) reached")]
    AttemptsCapReached,
}

impl Task {
    /// Construct a freshly injected task: queued at the pipeline's entry stage,
    /// attempts = 1.
    pub fn injected(
        project_id: String,
        pipeline: String,
        entry_stage: String,
        topic: String,
        target_repo: Option<String>,
        now_unix: i64,
    ) -> Self {
        Self {
            id: TaskId(format!("T-{}", uuid::Uuid::new_v4())),
            project_id,
            pipeline,
            topic,
            target_repo,
            target_scope: None,
            current_stage: entry_stage,
            state: TaskState::Queued,
            attempts: 1,
            parent_artifact: None,
            review_artifact: None,
            created_at: now_unix,
            updated_at: now_unix,
            group_id: None,
            lane: None,
            join_target: None,
        }
    }

    /// Construct a lane sibling task for a fork expansion (Decision D3). Inherits
    /// the parent's lineage (project/pipeline/topic/repo/scope/parent_artifact),
    /// mints a fresh id, and is queued at the lane's entry team carrying its
    /// group/lane/join membership.
    pub fn forked(
        parent: &Task,
        lane_entry_team: &str,
        group_id: &str,
        join_target: &str,
        now_unix: i64,
    ) -> Self {
        Self {
            id: TaskId(format!("T-{}", uuid::Uuid::new_v4())),
            project_id: parent.project_id.clone(),
            pipeline: parent.pipeline.clone(),
            topic: parent.topic.clone(),
            target_repo: parent.target_repo.clone(),
            target_scope: parent.target_scope.clone(),
            current_stage: lane_entry_team.to_string(),
            state: TaskState::Queued,
            attempts: 1,
            parent_artifact: parent.parent_artifact.clone(),
            review_artifact: None,
            created_at: now_unix,
            updated_at: now_unix,
            group_id: Some(group_id.to_string()),
            lane: Some(lane_entry_team.to_string()),
            join_target: Some(join_target.to_string()),
        }
    }

    /// Whether moving to `to` is allowed from the current state. Encodes the
    /// context-map.md transitions: queued -> running -> (settled implied) ->
    /// queued|gated|done|needs_human; gated -> queued|needs_human; braked is
    /// reachable from queued/running and returns to queued.
    pub fn can_transition_to(&self, to: TaskState) -> bool {
        use TaskState::*;
        if self.state == Done {
            return false;
        }
        match (self.state, to) {
            (Queued, Running) | (Queued, Braked) => true,
            (Running, Queued) | (Running, Gated) | (Running, Done) | (Running, NeedsHuman)
            | (Running, Revising) | (Running, Braked) => true,
            (Gated, Queued) | (Gated, Revising) | (Gated, NeedsHuman) => true,
            (Revising, Queued) | (Revising, Running) => true,
            (NeedsHuman, Queued) => true, // operator can re-queue an escalated task
            (Braked, Queued) => true,
            _ => false,
        }
    }

    /// Apply a transition, enforcing the immutability + legality invariants.
    pub fn transition_to(&mut self, to: TaskState, now_unix: i64) -> Result<(), TaskTransitionError> {
        if self.state == TaskState::Done {
            return Err(TaskTransitionError::DoneIsImmutable);
        }
        if !self.can_transition_to(to) {
            return Err(TaskTransitionError::Illegal { from: self.state, to });
        }
        self.state = to;
        self.updated_at = now_unix;
        Ok(())
    }

    /// Bump attempts on a revise, enforcing the cap. Returns Err when already at
    /// the cap (the caller should escalate to needs_human instead).
    pub fn bump_attempts(&mut self) -> Result<u32, TaskTransitionError> {
        if self.attempts >= MAX_ATTEMPTS {
            return Err(TaskTransitionError::AttemptsCapReached);
        }
        self.attempts += 1;
        Ok(self.attempts)
    }

    /// Map a verdict to the next state for a *team* stage (gates are handled by
    /// the operator, not here). Approve -> Queued (router moves the stage);
    /// Revise -> Revising (router routes back, attempts bumped); Reject ->
    /// NeedsHuman.
    pub fn next_state_for_verdict(verdict: Verdict) -> TaskState {
        match verdict {
            Verdict::Approve => TaskState::Queued,
            Verdict::Revise => TaskState::Revising,
            Verdict::Reject => TaskState::NeedsHuman,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Task {
        Task::injected("p".into(), "pipe".into(), "research".into(), "topic".into(), Some("/repo".into()), 100)
    }

    #[test]
    fn injected_task_is_queued_at_entry_with_one_attempt() {
        let task = t();
        assert_eq!(task.state, TaskState::Queued);
        assert_eq!(task.current_stage, "research");
        assert_eq!(task.attempts, 1);
        assert!(task.id.0.starts_with("T-"));
    }

    #[test]
    fn state_string_round_trips() {
        for s in [TaskState::Queued, TaskState::Running, TaskState::Gated, TaskState::Revising,
                  TaskState::NeedsHuman, TaskState::Done, TaskState::Braked] {
            assert_eq!(TaskState::parse(s.as_str()), Some(s));
        }
        assert_eq!(TaskState::parse("bogus"), None);
    }

    #[test]
    fn legal_claim_transition_queued_to_running() {
        let mut task = t();
        assert!(task.transition_to(TaskState::Running, 200).is_ok());
        assert_eq!(task.state, TaskState::Running);
        assert_eq!(task.updated_at, 200);
    }

    #[test]
    fn illegal_transition_is_rejected() {
        let mut task = t(); // queued
        let err = task.transition_to(TaskState::Gated, 200).unwrap_err();
        assert!(matches!(err, TaskTransitionError::Illegal { .. }));
    }

    #[test]
    fn done_is_immutable() {
        let mut task = t();
        task.transition_to(TaskState::Running, 1).unwrap();
        task.transition_to(TaskState::Done, 2).unwrap();
        let err = task.transition_to(TaskState::Queued, 3).unwrap_err();
        assert_eq!(err, TaskTransitionError::DoneIsImmutable);
    }

    #[test]
    fn attempts_cap_is_enforced() {
        let mut task = t(); // attempts = 1
        assert_eq!(task.bump_attempts().unwrap(), 2);
        assert_eq!(task.bump_attempts().unwrap(), 3);
        assert_eq!(task.bump_attempts().unwrap_err(), TaskTransitionError::AttemptsCapReached);
    }

    #[test]
    fn injected_task_has_no_lane_fields() {
        let task = t();
        assert_eq!(task.group_id, None);
        assert_eq!(task.lane, None);
        assert_eq!(task.join_target, None);
    }

    #[test]
    fn forked_sibling_inherits_lineage_and_carries_lane_fields() {
        let parent = t();
        let sib = Task::forked(&parent, "lane-a", "G-1", "join-1", 500);
        assert_ne!(sib.id, parent.id);
        assert!(sib.id.0.starts_with("T-"));
        assert_eq!(sib.project_id, parent.project_id);
        assert_eq!(sib.pipeline, parent.pipeline);
        assert_eq!(sib.topic, parent.topic);
        assert_eq!(sib.current_stage, "lane-a");
        assert_eq!(sib.state, TaskState::Queued);
        assert_eq!(sib.attempts, 1);
        assert_eq!(sib.group_id.as_deref(), Some("G-1"));
        assert_eq!(sib.lane.as_deref(), Some("lane-a"));
        assert_eq!(sib.join_target.as_deref(), Some("join-1"));
    }

    #[test]
    fn joining_state_string_round_trips() {
        assert_eq!(TaskState::parse("joining"), Some(TaskState::Joining));
        assert_eq!(TaskState::Joining.as_str(), "joining");
    }

    #[test]
    fn verdict_maps_to_next_state() {
        assert_eq!(Task::next_state_for_verdict(Verdict::Approve), TaskState::Queued);
        assert_eq!(Task::next_state_for_verdict(Verdict::Revise), TaskState::Revising);
        assert_eq!(Task::next_state_for_verdict(Verdict::Reject), TaskState::NeedsHuman);
    }
}
