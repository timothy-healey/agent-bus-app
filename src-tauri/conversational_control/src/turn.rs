//! Turn / Role / ToolCall value types — the serialised content of a
//! Conversation's `history_json`. A Turn is one message by one role; a
//! ToolCall is a request+result pair embedded in an assistant turn.

use agent_bus_core::{ToolCallRequest, ToolCallResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A dispatched app-tool: the request Claude (or the parser) made, and the
/// result the supplier returned. `result: None` means dispatched-but-unresolved
/// (an invariant the aggregate forbids at idle — see Task 3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub request: ToolCallRequest,
    pub result: Option<ToolCallResult>,
}

/// One turn in the conversation. Assistant turns may carry tool-calls; user
/// turns never do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub at: i64,
}

impl Turn {
    pub fn user(text: impl Into<String>, at: i64) -> Self {
        Turn { role: Role::User, text: text.into(), tool_calls: vec![], at }
    }
    pub fn assistant(text: impl Into<String>, tool_calls: Vec<ToolCall>, at: i64) -> Self {
        Turn { role: Role::Assistant, text: text.into(), tool_calls, at }
    }
    /// Rough token estimate for history-budget math (D: ~4 chars/token, plus a
    /// small per-tool-call constant). Deterministic so budget tests are stable.
    pub fn estimated_tokens(&self) -> usize {
        let text = self.text.chars().count() / 4;
        let tools: usize = self
            .tool_calls
            .iter()
            .map(|tc| tc.request.tool_name.chars().count() / 4 + 8)
            .sum();
        text + tools + 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn turn_round_trips_with_embedded_tool_call() {
        let tc = ToolCall {
            request: ToolCallRequest { tool_name: "inject_topic".into(), args: json!({"topic": "x"}) },
            result: Some(ToolCallResult::Ok { result: json!({"task_id": "T-1"}) }),
        };
        let t = Turn::assistant("done", vec![tc], 100);
        let s = serde_json::to_string(&t).unwrap();
        let back: Turn = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
        assert_eq!(back.role, Role::Assistant);
    }

    #[test]
    fn role_serialises_lowercase() {
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::to_string(&Role::Assistant).unwrap(), "\"assistant\"");
    }

    #[test]
    fn estimated_tokens_grows_with_text_and_tool_calls() {
        let bare = Turn::user("hello there", 0);
        let withtool = Turn::assistant(
            "hello there",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "approve_gate".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        assert!(withtool.estimated_tokens() > bare.estimated_tokens());
    }
}
