//! The conversation "brain" behind a trait (D4/D6). v1's CommandEngine parses
//! the user line, dispatches any tool call, and composes an assistant reply —
//! NO model, NO binary. A future ClaudeCliEngine (v1.1) would reuse the Runners
//! ACL behind this same trait. Tests use FakeEngine.

use crate::catalog::ToolCatalog;
use crate::command::{parse_command, Parsed};
use crate::dispatch::ToolDispatcher;
use crate::turn::ToolCall;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// What the engine produces for one user input: the assistant's text plus any
/// tool-calls it made (already resolved with results).
#[derive(Debug, Clone, PartialEq)]
pub struct EngineReply {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

/// The seam. Given the user's input line (and the catalog), produce a reply.
#[async_trait]
pub trait ConversationEngine: Send + Sync {
    async fn respond(&self, input: &str, catalog: &ToolCatalog) -> EngineReply;
}

/// v1 engine: parse -> dispatch -> compose. Deterministic and binary-free.
pub struct CommandEngine {
    dispatcher: Arc<dyn ToolDispatcher>,
}

impl CommandEngine {
    pub fn new(dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait]
impl ConversationEngine for CommandEngine {
    async fn respond(&self, input: &str, catalog: &ToolCatalog) -> EngineReply {
        match parse_command(input, catalog) {
            Parsed::Tool(req) => {
                let result = self.dispatcher.dispatch(&req).await;
                let ok = matches!(result, agent_bus_core::ToolCallResult::Ok { .. });
                let text = if ok {
                    format!("Done: {}.", req.tool_name)
                } else {
                    format!("That didn't work: {}.", req.tool_name)
                };
                EngineReply {
                    text,
                    tool_calls: vec![ToolCall { request: req, result: Some(result) }],
                }
            }
            Parsed::Chat(msg) => EngineReply {
                // v1 has no model, so a plain question gets a help nudge listing
                // the v1 commands. v1.1's LLM engine replaces this branch.
                text: format!(
                    "v1 terminal is command-driven. Try /inject <topic>, /approve <task_id>, /brake [reason]. (You said: {msg})"
                ),
                tool_calls: vec![],
            },
            Parsed::Error(e) => EngineReply { text: e, tool_calls: vec![] },
        }
    }
}

/// Test double: returns seeded replies in order, recording inputs.
pub struct FakeEngine {
    replies: Vec<EngineReply>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<String>>,
}

impl FakeEngine {
    pub fn new(replies: Vec<EngineReply>) -> Self {
        Self { replies, cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }
}

#[async_trait]
impl ConversationEngine for FakeEngine {
    async fn respond(&self, input: &str, _catalog: &ToolCatalog) -> EngineReply {
        self.received.lock().unwrap().push(input.to_string());
        let mut c = self.cursor.lock().unwrap();
        let idx = (*c).min(self.replies.len().saturating_sub(1));
        *c += 1;
        self.replies.get(idx).cloned().unwrap_or(EngineReply { text: String::new(), tool_calls: vec![] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::FakeDispatcher;
    use agent_bus_core::{ToolCallResult, ToolSpec};
    use serde_json::json;

    fn catalog() -> ToolCatalog {
        ToolCatalog::new(vec![ToolSpec {
            name: "inject_topic".into(), description: "d".into(),
            input_schema: json!({"type":"object"}), supplier_context: "runtime".into(),
        }])
    }

    #[tokio::test]
    async fn command_engine_dispatches_a_slash_command_and_embeds_the_result() {
        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id": "T-9"}) }),
        );
        let eng = CommandEngine::new(disp.clone());
        let reply = eng.respond("/inject 03-scheduling", &catalog()).await;
        assert!(reply.text.contains("Done"));
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].request.tool_name, "inject_topic");
        assert!(matches!(reply.tool_calls[0].result, Some(ToolCallResult::Ok { .. })));
        // the dispatcher actually got the call
        assert_eq!(disp.received.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn command_engine_failed_dispatch_yields_no_ok_text() {
        let disp = Arc::new(FakeDispatcher::new()); // unseeded -> Err
        let eng = CommandEngine::new(disp);
        let reply = eng.respond("/inject x", &catalog()).await;
        assert!(reply.text.contains("didn't work"));
        assert!(matches!(reply.tool_calls[0].result, Some(ToolCallResult::Err { .. })));
    }

    #[tokio::test]
    async fn command_engine_plain_text_returns_help_no_tool_calls() {
        let eng = CommandEngine::new(Arc::new(FakeDispatcher::new()));
        let reply = eng.respond("how is T-042 going?", &catalog()).await;
        assert!(reply.tool_calls.is_empty());
        assert!(reply.text.contains("command-driven"));
    }

    #[tokio::test]
    async fn fake_engine_returns_seeded_replies_and_records() {
        let eng = FakeEngine::new(vec![EngineReply { text: "hi".into(), tool_calls: vec![] }]);
        let r = eng.respond("anything", &catalog()).await;
        assert_eq!(r.text, "hi");
        assert_eq!(eng.received.lock().unwrap()[0], "anything");
    }
}
