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
    fn verdict_maps_to_next_state() {
        assert_eq!(Task::next_state_for_verdict(Verdict::Approve), TaskState::Queued);
        assert_eq!(Task::next_state_for_verdict(Verdict::Revise), TaskState::Revising);
        assert_eq!(Task::next_state_for_verdict(Verdict::Reject), TaskState::NeedsHuman);
    }
}
