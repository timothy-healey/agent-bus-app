//! The llm_chat boundary types — the Runtime/domain vocabulary that crosses the
//! ACL. Nothing here mentions stream-json, CLI flags, or session ids: those are
//! sealed in command.rs / stream_json.rs / session.rs.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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
}

/// The assistant's reply to one turn. Domain-shaped: text + usage only. There is
/// deliberately NO session_id here (F3 — session continuity is internal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    pub text: String,
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
            model: "m".into(), thinking_budget: 0,
        };
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let reply = fake.chat_stream(&req, &sink).await.unwrap();
        assert_eq!(reply.text, "hi");
        assert!(seen.lock().unwrap().is_empty());
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
