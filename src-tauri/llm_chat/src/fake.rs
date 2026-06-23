//! FakeChatRunner — a ChatRunner test double. Returns seeded ChatReplies in
//! order (clamping to the last when exhausted) and records every ChatRequest it
//! received, so tests can assert what was asked without a live `claude`.

use crate::chat::{ChatError, ChatReply, ChatRequest, ChatRunner};
use async_trait::async_trait;
use std::sync::Mutex;

/// A ChatRunner test double. Either returns seeded replies in order (clamping to
/// the last when exhausted) or always errors with a seeded ChatError. Records
/// every request for assertions.
pub struct FakeChatRunner {
    replies: Vec<ChatReply>,
    error: Option<ChatError>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<ChatRequest>>,
}

impl FakeChatRunner {
    /// Seed with replies returned in order.
    pub fn new(replies: Vec<ChatReply>) -> Self {
        Self { replies, error: None, cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }

    /// Seed with an error that every `chat` call returns.
    pub fn failing(error: ChatError) -> Self {
        Self { replies: vec![], error: Some(error), cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }
}

#[async_trait]
impl ChatRunner for FakeChatRunner {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError> {
        self.received.lock().unwrap().push(req.clone());
        if let Some(err) = &self.error {
            // Re-create the same class (ChatError is not Clone).
            return Err(match err {
                ChatError::RateLimited(m) => ChatError::RateLimited(m.clone()),
                ChatError::Spawn(m) => ChatError::Spawn(m.clone()),
                ChatError::NoResult => ChatError::NoResult,
                ChatError::Other(m) => ChatError::Other(m.clone()),
            });
        }
        let mut c = self.cursor.lock().unwrap();
        let idx = (*c).min(self.replies.len().saturating_sub(1));
        *c += 1;
        Ok(self
            .replies
            .get(idx)
            .cloned()
            .unwrap_or(ChatReply { text: String::new(), usage: Default::default() }))
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
    async fn can_be_seeded_to_error() {
        let fake = FakeChatRunner::failing(crate::chat::ChatError::RateLimited("429".into()));
        let err = fake.chat(&req("x")).await.unwrap_err();
        assert!(err.is_rate_limited());
    }
}
