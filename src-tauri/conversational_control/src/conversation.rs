//! The Conversation aggregate (singleton per project). Enforces the context-map
//! invariants: turns strictly alternate user/assistant; total history tokens ≤
//! history_budget_tokens (truncate oldest *pair* when exceeded); a session has a
//! session_id + started_at + last_message_at; summary_of_prior_sessions holds
//! the 24h digest of earlier sessions.

use crate::turn::{Role, Turn};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_HISTORY_BUDGET_TOKENS: usize = 8_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConversationError {
    #[error("turns must alternate user/assistant; got two {0:?} in a row")]
    NonAlternating(Role),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub project_id: String,
    pub session_id: String,
    pub started_at: i64,
    pub last_message_at: i64,
    pub turns: Vec<Turn>,
    pub summary_of_prior_sessions: Option<String>,
    pub history_budget_tokens: usize,
}

impl Conversation {
    /// A fresh, empty conversation for a project.
    pub fn new(project_id: impl Into<String>, session_id: impl Into<String>, now: i64) -> Self {
        Conversation {
            project_id: project_id.into(),
            session_id: session_id.into(),
            started_at: now,
            last_message_at: now,
            turns: vec![],
            summary_of_prior_sessions: None,
            history_budget_tokens: DEFAULT_HISTORY_BUDGET_TOKENS,
        }
    }

    /// Append a turn, enforcing alternation. The first turn must be a user turn;
    /// thereafter roles must alternate. Updates last_message_at and truncates to
    /// the history budget. Returns the index of the appended turn.
    pub fn append(&mut self, turn: Turn) -> Result<usize, ConversationError> {
        if let Some(last) = self.turns.last() {
            if last.role == turn.role {
                return Err(ConversationError::NonAlternating(turn.role));
            }
        } else if turn.role != Role::User {
            return Err(ConversationError::NonAlternating(turn.role));
        }
        self.last_message_at = turn.at;
        self.turns.push(turn);
        self.truncate_to_budget();
        Ok(self.turns.len() - 1)
    }

    /// Total estimated tokens across all turns.
    pub fn estimated_tokens(&self) -> usize {
        self.turns.iter().map(|t| t.estimated_tokens()).sum()
    }

    /// Drop oldest user/assistant *pairs* until under budget. Removing whole
    /// pairs preserves alternation (the head stays a user turn). Never drops the
    /// final pair (always keep the latest exchange visible).
    fn truncate_to_budget(&mut self) {
        while self.estimated_tokens() > self.history_budget_tokens && self.turns.len() > 2 {
            // Drop the two oldest turns (a user/assistant pair).
            self.turns.drain(0..2);
        }
    }

    /// True if there is a dangling assistant tool-call with no result — the
    /// aggregate must not be at idle in this state (used by the command flow to
    /// assert it resolved every call before returning).
    pub fn has_unresolved_tool_call(&self) -> bool {
        self.turns
            .iter()
            .flat_map(|t| t.tool_calls.iter())
            .any(|tc| tc.result.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 { 1_000 }

    #[test]
    fn first_turn_must_be_user() {
        let mut c = Conversation::new("p", "s", now());
        let err = c.append(Turn::assistant("hi", vec![], now())).unwrap_err();
        assert_eq!(err, ConversationError::NonAlternating(Role::Assistant));
    }

    #[test]
    fn turns_must_alternate() {
        let mut c = Conversation::new("p", "s", now());
        c.append(Turn::user("a", now())).unwrap();
        let err = c.append(Turn::user("b", now())).unwrap_err();
        assert_eq!(err, ConversationError::NonAlternating(Role::User));
    }

    #[test]
    fn alternating_turns_append_and_bump_last_message_at() {
        let mut c = Conversation::new("p", "s", now());
        c.append(Turn::user("a", 10)).unwrap();
        c.append(Turn::assistant("b", vec![], 20)).unwrap();
        assert_eq!(c.turns.len(), 2);
        assert_eq!(c.last_message_at, 20);
    }

    #[test]
    fn truncation_drops_oldest_pair_when_over_budget() {
        let mut c = Conversation::new("p", "s", now());
        c.history_budget_tokens = 20; // tiny budget forces truncation
        for i in 0..8 {
            let role_user = i % 2 == 0;
            let big = "x".repeat(200); // ~50 tokens each
            let t = if role_user { Turn::user(big, i) } else { Turn::assistant(big, vec![], i) };
            c.append(t).unwrap();
        }
        // Stays alternating (head is still a user turn) and bounded.
        assert_eq!(c.turns.first().unwrap().role, Role::User);
        assert!(c.turns.len() >= 2);
        assert!(c.estimated_tokens() <= 20 || c.turns.len() == 2);
    }

    #[test]
    fn unresolved_tool_call_is_detectable() {
        use crate::turn::ToolCall;
        use agent_bus_core::ToolCallRequest;
        let mut c = Conversation::new("p", "s", now());
        c.append(Turn::user("go", 0)).unwrap();
        c.append(Turn::assistant(
            "working",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "inject_topic".into(), args: serde_json::json!({}) },
                result: None,
            }],
            1,
        )).unwrap();
        assert!(c.has_unresolved_tool_call());
    }
}
