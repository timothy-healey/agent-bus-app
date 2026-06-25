//! FakeChatRunner — a ChatRunner test double. Returns seeded ChatReplies in
//! order (clamping to the last when exhausted) and records every ChatRequest it
//! received, so tests can assert what was asked without a live `claude`.

use crate::chat::{
    ChatError, ChatReply, ChatRequest, ChatRunner, ChatToolDef, DeltaSink, StructuredReply,
};
use async_trait::async_trait;
use std::sync::Mutex;

/// A ChatRunner test double. Either returns seeded replies in order (clamping to
/// the last when exhausted) or always errors with a seeded ChatError. Records
/// every request for assertions. Optionally seeds per-call scripted prose deltas
/// that `chat_stream` forwards before returning that call's reply.
pub struct FakeChatRunner {
    replies: Vec<ChatReply>,
    /// Per-call scripted prose deltas to forward via chat_stream before returning
    /// that call's reply. A call with no entry (index >= len) forwards nothing.
    /// Indexed by the same cursor as `replies`.
    deltas: Vec<Vec<String>>,
    error: Option<ChatError>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<ChatRequest>>,
}

impl FakeChatRunner {
    /// Seed with replies returned in order (no streamed deltas).
    pub fn new(replies: Vec<ChatReply>) -> Self {
        Self { replies, deltas: vec![], error: None, cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }

    /// Seed with replies AND, per call, the ordered prose deltas chat_stream
    /// forwards before returning that call's reply.
    pub fn with_deltas(replies: Vec<ChatReply>, deltas: Vec<Vec<String>>) -> Self {
        Self { replies, deltas, error: None, cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }

    /// Seed with an error that every `chat` call returns.
    pub fn failing(error: ChatError) -> Self {
        Self { replies: vec![], deltas: vec![], error: Some(error), cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }

    /// Reply at index `idx` (clamped), or empty when unseeded.
    fn reply_at(&self, idx: usize) -> ChatReply {
        self.replies
            .get(idx.min(self.replies.len().saturating_sub(1)))
            .cloned()
            .unwrap_or(ChatReply { text: String::new(), usage: Default::default() })
    }

    /// Re-create the seeded error class (ChatError is not Clone).
    fn clone_error(&self) -> Option<ChatError> {
        self.error.as_ref().map(|err| match err {
            ChatError::RateLimited(m) => ChatError::RateLimited(m.clone()),
            ChatError::Spawn(m) => ChatError::Spawn(m.clone()),
            ChatError::NoResult => ChatError::NoResult,
            ChatError::Other(m) => ChatError::Other(m.clone()),
            ChatError::Unsupported(m) => ChatError::Unsupported(m.clone()),
        })
    }

    /// Advance the cursor, returning the index this call should use.
    fn next_idx(&self) -> usize {
        let mut c = self.cursor.lock().unwrap();
        let i = *c;
        *c += 1;
        i
    }
}

#[async_trait]
impl ChatRunner for FakeChatRunner {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError> {
        self.received.lock().unwrap().push(req.clone());
        if let Some(err) = self.clone_error() {
            return Err(err);
        }
        let idx = self.next_idx();
        Ok(self.reply_at(idx))
    }

    async fn chat_stream(&self, req: &ChatRequest, sink: &DeltaSink) -> Result<ChatReply, ChatError> {
        self.received.lock().unwrap().push(req.clone());
        if let Some(err) = self.clone_error() {
            return Err(err);
        }
        let idx = self.next_idx();
        // Only forward when this call has a scripted delta entry.
        if let Some(call_deltas) = self.deltas.get(idx) {
            for d in call_deltas {
                sink(d);
            }
        }
        Ok(self.reply_at(idx))
    }
}

/// A structured-capable ChatRunner test double (the anthropic-api path stand-in).
/// `supports_structured()` is true; `chat_structured` returns seeded
/// `StructuredReply`s in order (clamping to the last) and records the
/// `(ChatRequest, tool names, forced tool)` of every call so tests can assert the
/// structured seam was driven. `chat`/`chat_stream` return seeded plain replies.
pub struct FakeStructuredChatRunner {
    structured: Vec<StructuredReply>,
    replies: Vec<ChatReply>,
    error: Option<ChatError>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<ChatRequest>>,
    /// Per structured call: (tool names offered, forced tool name).
    pub structured_calls: Mutex<Vec<(Vec<String>, Option<String>)>>,
}

impl FakeStructuredChatRunner {
    /// Seed the structured replies returned by `chat_structured` in order.
    pub fn new(structured: Vec<StructuredReply>) -> Self {
        Self {
            structured,
            replies: vec![],
            error: None,
            cursor: Mutex::new(0),
            received: Mutex::new(vec![]),
            structured_calls: Mutex::new(vec![]),
        }
    }

    /// Seed structured replies AND plain `chat` replies.
    pub fn with_chat(structured: Vec<StructuredReply>, replies: Vec<ChatReply>) -> Self {
        Self {
            structured,
            replies,
            error: None,
            cursor: Mutex::new(0),
            received: Mutex::new(vec![]),
            structured_calls: Mutex::new(vec![]),
        }
    }

    /// Seed an error every `chat_structured` call returns.
    pub fn failing(error: ChatError) -> Self {
        Self {
            structured: vec![],
            replies: vec![],
            error: Some(error),
            cursor: Mutex::new(0),
            received: Mutex::new(vec![]),
            structured_calls: Mutex::new(vec![]),
        }
    }

    fn next_idx(&self) -> usize {
        let mut c = self.cursor.lock().unwrap();
        let i = *c;
        *c += 1;
        i
    }

    fn clone_error(&self) -> Option<ChatError> {
        self.error.as_ref().map(|err| match err {
            ChatError::RateLimited(m) => ChatError::RateLimited(m.clone()),
            ChatError::Spawn(m) => ChatError::Spawn(m.clone()),
            ChatError::NoResult => ChatError::NoResult,
            ChatError::Other(m) => ChatError::Other(m.clone()),
            ChatError::Unsupported(m) => ChatError::Unsupported(m.clone()),
        })
    }
}

#[async_trait]
impl ChatRunner for FakeStructuredChatRunner {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError> {
        self.received.lock().unwrap().push(req.clone());
        if let Some(err) = self.clone_error() {
            return Err(err);
        }
        let idx = self.next_idx();
        Ok(self
            .replies
            .get(idx.min(self.replies.len().saturating_sub(1)))
            .cloned()
            .unwrap_or(ChatReply { text: String::new(), usage: Default::default() }))
    }

    fn supports_structured(&self) -> bool {
        true
    }

    async fn chat_structured(
        &self,
        req: &ChatRequest,
        tools: &[ChatToolDef],
        force: Option<&str>,
    ) -> Result<StructuredReply, ChatError> {
        self.received.lock().unwrap().push(req.clone());
        self.structured_calls
            .lock()
            .unwrap()
            .push((tools.iter().map(|t| t.name.clone()).collect(), force.map(|s| s.to_string())));
        if let Some(err) = self.clone_error() {
            return Err(err);
        }
        let idx = self.next_idx();
        self.structured
            .get(idx.min(self.structured.len().saturating_sub(1)))
            .cloned()
            .ok_or(ChatError::NoResult)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatRunner, ChatRequest, ChatReply, ChatUsage};

    fn req(msg: &str) -> ChatRequest {
        ChatRequest {
            dialogue_id: "proj-1".into(),
            system_prompt: "sys".into(),
            user_message: msg.into(),
            model: "m".into(),
            thinking_budget: 0,
        }
    }

    #[tokio::test]
    async fn returns_seeded_replies_in_order_and_records_requests() {
        let fake = FakeChatRunner::new(vec![
            ChatReply { text: "first".into(), usage: ChatUsage::default() },
            ChatReply { text: "second".into(), usage: ChatUsage::default() },
        ]);
        let a = fake.chat(&req("one")).await.unwrap();
        let b = fake.chat(&req("two")).await.unwrap();
        assert_eq!(a.text, "first");
        assert_eq!(b.text, "second");
        let received = fake.received.lock().unwrap();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].user_message, "one");
        assert_eq!(received[1].user_message, "two");
    }

    #[tokio::test]
    async fn clamps_to_the_last_reply_when_exhausted() {
        let fake = FakeChatRunner::new(vec![ChatReply { text: "only".into(), usage: ChatUsage::default() }]);
        let _ = fake.chat(&req("a")).await.unwrap();
        let again = fake.chat(&req("b")).await.unwrap();
        assert_eq!(again.text, "only");
    }

    #[tokio::test]
    async fn scripted_deltas_are_forwarded_then_reply_returned() {
        let fake = FakeChatRunner::with_deltas(
            vec![ChatReply { text: "Hello world".into(), usage: ChatUsage::default() }],
            vec![vec!["Hello ".into(), "world".into()]],
        );
        let seen = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: crate::chat::DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let reply = fake.chat_stream(&req("hi"), &sink).await.unwrap();
        assert_eq!(reply.text, "Hello world");
        assert_eq!(*seen.lock().unwrap(), vec!["Hello ".to_string(), "world".to_string()]);
    }

    #[tokio::test]
    async fn deltas_default_to_empty_for_new_constructor() {
        let fake = FakeChatRunner::new(vec![ChatReply { text: "x".into(), usage: ChatUsage::default() }]);
        let seen = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: crate::chat::DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let reply = fake.chat_stream(&req("hi"), &sink).await.unwrap();
        assert_eq!(reply.text, "x");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn can_be_seeded_to_error() {
        let fake = FakeChatRunner::failing(crate::chat::ChatError::RateLimited("429".into()));
        let err = fake.chat(&req("x")).await.unwrap_err();
        assert!(err.is_rate_limited());
    }
}
