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
    /// small per-tool-call constant). Folds in each embedded tool-call's args
    /// and result payload so fat agentic composite turns are counted accurately
    /// (C3). Deterministic so budget tests are stable.
    pub fn estimated_tokens(&self) -> usize {
        let text = self.text.chars().count() / 4;
        let tools: usize = self.tool_calls.iter().map(tool_call_tokens).sum();
        text + tools + 4
    }
}

/// Coarse token estimate for one embedded tool-call: the tool name, the
/// serialized args, and the serialized result payload (the Ok value or the Err
/// message), plus a small structural constant for the call's JSON envelope.
/// Byte length is used for serialized JSON (an upper bound on char count), which
/// biases the estimate conservatively — the safe direction for a budget cap.
fn tool_call_tokens(tc: &ToolCall) -> usize {
    let name = tc.request.tool_name.chars().count() / 4;
    let args = serde_json::to_string(&tc.request.args).map_or(0, |s| s.len()) / 4;
    let result = match &tc.result {
        Some(ToolCallResult::Ok { result }) => {
            serde_json::to_string(result).map_or(0, |s| s.len()) / 4
        }
        Some(ToolCallResult::Err { error }) => error.chars().count() / 4,
        None => 0,
    };
    name + args + result + 8
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

    #[test]
    fn estimated_tokens_counts_tool_call_args() {
        let small = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        let fat_args = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest {
                    tool_name: "t".into(),
                    args: json!({ "blob": "x".repeat(400) }),
                },
                result: None,
            }],
            0,
        );
        // ~400 chars of args => ~100 extra tokens; must clearly exceed the small call.
        assert!(
            fat_args.estimated_tokens() > small.estimated_tokens() + 50,
            "fat args ({}) should dwarf empty args ({})",
            fat_args.estimated_tokens(),
            small.estimated_tokens()
        );
    }

    #[test]
    fn estimated_tokens_counts_tool_call_result() {
        let no_result = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        let fat_result = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: Some(ToolCallResult::Ok { result: json!({ "out": "y".repeat(400) }) }),
            }],
            0,
        );
        assert!(
            fat_result.estimated_tokens() > no_result.estimated_tokens() + 50,
            "fat result ({}) should dwarf no result ({})",
            fat_result.estimated_tokens(),
            no_result.estimated_tokens()
        );
    }

    #[test]
    fn estimated_tokens_counts_err_result_message() {
        let no_result = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        let fat_err = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: Some(ToolCallResult::Err { error: "e".repeat(400) }),
            }],
            0,
        );
        assert!(
            fat_err.estimated_tokens() > no_result.estimated_tokens() + 50,
            "fat err ({}) should dwarf no result ({})",
            fat_err.estimated_tokens(),
            no_result.estimated_tokens()
        );
    }

    #[test]
    fn estimated_tokens_unresolved_call_charges_only_structural_constant() {
        // A None result must not add result-size tokens; only the per-call
        // constant + name + (empty) args. Guards against the estimate ballooning
        // on dispatched-but-unresolved calls.
        let t = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "approve_gate".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        // name (12 chars /4 = 3) + args "{}" (2 bytes /4 = 0) + 8 constant + 4 turn base = 15.
        assert_eq!(t.estimated_tokens(), 15);
    }
}
