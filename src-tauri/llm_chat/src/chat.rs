//! The llm_chat boundary types — the Runtime/domain vocabulary that crosses the
//! ACL. Nothing here mentions stream-json, CLI flags, or session ids: those are
//! sealed in command.rs / stream_json.rs / session.rs.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Token usage for one chat turn. Structurally identical to `runners::RunnerUsage`
/// (D2) but defined here so the ACL stays kernel-only — the composition root maps
/// it onto whatever Telemetry shape it needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

/// One chat turn request. `dialogue_id` is the caller's own stable id; the ACL
/// maps it to a claude session internally (F3 — no session_id crosses out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRequest {
    pub dialogue_id: String,
    pub system_prompt: String,
    pub user_message: String,
    pub model: String,
    pub thinking_budget: u32,
    /// The directory the child `claude` process runs in (LF26 parity with the
    /// worker runner). `None` = inherit the parent's cwd (the default, used by
    /// the terminal chat which has no work-item working dir).
    pub working_dir: Option<String>,
}

/// The assistant's reply to one turn. Domain-shaped: text + usage only. There is
/// deliberately NO session_id here (F3 — session continuity is internal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    pub text: String,
    pub usage: ChatUsage,
}

/// A tool the model may call to emit structured output (the kernel shape that
/// crosses the ACL). `input_schema` is a plain JSON Schema `Value` — the SAME
/// schema a consumer already uses for prose-fenced validation (DS-Schema's
/// `slice_schema`, T1's published `ToolSpec.input_schema`). NO anthropic idiom
/// here: the runner maps this onto whatever the provider's tool shape is,
/// sealed inside the concrete runner.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A structured (tool-call) reply: the tool the model chose plus its arguments
/// as a JSON `Value` already shaped to the tool's `input_schema`, plus usage.
/// Domain-shaped — no `tool_use`/`tool_result` envelope, no session id: those
/// stay sealed inside the concrete runner (the ACL seal).
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredReply {
    pub tool_name: String,
    pub args: Value,
    pub usage: ChatUsage,
}

/// Mirrors `runners::RunnerError`'s classes so chat consumers handle failure the
/// same way Runtime handles runner failure.
#[derive(Debug)]
pub enum ChatError {
    RateLimited(String),
    Spawn(String),
    NoResult,
    Other(String),
    /// The runner does not support the requested capability (e.g. structured
    /// output via native tool-use). Returned by the DEFAULT `chat_structured`
    /// impl so the CLI runner degrades and consumers fall back to the
    /// prose+parse+repair path. NOT a failure — it is a capability signal.
    Unsupported(String),
}

impl ChatError {
    /// True when the failure is a rate-limit — the one case the root treats
    /// specially (surface as an error turn; the root may set the brake).
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, ChatError::RateLimited(_))
    }
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChatError::RateLimited(m) => write!(f, "rate limited: {m}"),
            ChatError::Spawn(m) => write!(f, "spawn failed: {m}"),
            ChatError::NoResult => write!(f, "no result parsed from chat output"),
            ChatError::Other(m) => write!(f, "chat failed: {m}"),
            ChatError::Unsupported(m) => write!(f, "unsupported chat capability: {m}"),
        }
    }
}

impl std::error::Error for ChatError {}

/// A display-only prose sink. The streaming chat path forwards each assistant
/// text fragment here as it parses. Deliberately a plain `&str` callback: NO
/// stream-json idiom, session id, or event name crosses the ACL through it —
/// the composition root maps fragments to whatever UI event it likes (F3).
pub type DeltaSink = Box<dyn Fn(&str) + Send + Sync>;

/// The ACL seam. Chat consumers depend only on this trait; the concrete runner
/// is selected once at the composition root. Object-safe so it is held as
/// `Arc<dyn ChatRunner>`.
#[async_trait]
pub trait ChatRunner: Send + Sync {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError>;

    /// Streaming variant: identical contract to `chat` (same final `ChatReply`),
    /// but assistant prose fragments are forwarded to `sink` as they arrive for
    /// live display. The default delegates to `chat` (no deltas), so existing
    /// runners keep working; streaming runners override this.
    async fn chat_stream(&self, req: &ChatRequest, sink: &DeltaSink) -> Result<ChatReply, ChatError> {
        let _ = sink;
        self.chat(req).await
    }

    /// Whether this runner can emit native structured output (tool-use). The
    /// default is `false` so existing runners (the CLI) keep working and
    /// consumers fall back to the prose+parse+repair path; the API runner
    /// overrides it to `true`.
    fn supports_structured(&self) -> bool {
        false
    }

    /// Structured-output turn: ask the model to call one of `tools` and return
    /// its `{tool_name, args}` natively (schema-valid by construction — no
    /// prose-fenced-json parsing or repair loop). `force` optionally pins the
    /// model to a specific tool name (forced tool_choice). The DEFAULT impl
    /// returns `ChatError::Unsupported` so the CLI runner degrades; consumers
    /// gate on `supports_structured()` before calling. The provider tool-use
    /// idiom is sealed inside the concrete runner — only `ChatToolDef`/
    /// `StructuredReply` (Value-based) cross this seam (the ACL seal).
    async fn chat_structured(
        &self,
        req: &ChatRequest,
        tools: &[ChatToolDef],
        force: Option<&str>,
    ) -> Result<StructuredReply, ChatError> {
        let _ = (req, tools, force);
        Err(ChatError::Unsupported(
            "this chat runner does not support native structured output".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_usage_default_is_zeroed() {
        let u = ChatUsage::default();
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.model, "");
    }

    #[test]
    fn rate_limited_is_distinguishable() {
        assert!(ChatError::RateLimited("429".into()).is_rate_limited());
        assert!(!ChatError::Spawn("no binary".into()).is_rate_limited());
        assert!(!ChatError::NoResult.is_rate_limited());
    }

    #[tokio::test]
    async fn default_chat_stream_delegates_to_chat_with_no_deltas() {
        use crate::fake::FakeChatRunner;
        let fake = FakeChatRunner::new(vec![ChatReply { text: "hi".into(), usage: ChatUsage::default() }]);
        let req = ChatRequest {
            dialogue_id: "d".into(), system_prompt: "s".into(), user_message: "u".into(),
            model: "m".into(), thinking_budget: 0, working_dir: None,
        };
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let reply = fake.chat_stream(&req, &sink).await.unwrap();
        assert_eq!(reply.text, "hi");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn default_chat_structured_is_unsupported() {
        // The default ChatRunner does not support native tool-use; the CLI runner
        // inherits this so consumers fall back to the prose+parse+repair path.
        use crate::fake::FakeChatRunner;
        let fake = FakeChatRunner::new(vec![ChatReply { text: "x".into(), usage: ChatUsage::default() }]);
        assert!(!fake.supports_structured());
        let req = ChatRequest {
            dialogue_id: "d".into(), system_prompt: "s".into(), user_message: "u".into(),
            model: "m".into(), thinking_budget: 0, working_dir: None,
        };
        let tool = ChatToolDef {
            name: "emit".into(), description: "emit a slice".into(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        let err = fake.chat_structured(&req, &[tool], Some("emit")).await.unwrap_err();
        assert!(matches!(err, ChatError::Unsupported(_)));
    }

    #[test]
    fn structured_reply_carries_tool_args_usage_only() {
        // Compile-time proof StructuredReply is kernel-shaped (no tool_use envelope).
        let r = StructuredReply {
            tool_name: "emit".into(),
            args: serde_json::json!({"kind": "teams"}),
            usage: ChatUsage::default(),
        };
        assert_eq!(r.tool_name, "emit");
        assert_eq!(r.args["kind"], "teams");
    }

    #[test]
    fn chat_reply_carries_text_and_usage_only() {
        // Compile-time proof there is no session_id field (F3): constructing a
        // ChatReply requires exactly text + usage.
        let r = ChatReply { text: "hi".into(), usage: ChatUsage::default() };
        assert_eq!(r.text, "hi");
        assert_eq!(r.usage.output_tokens, 0);
    }
}
