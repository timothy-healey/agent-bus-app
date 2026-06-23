//! stream-json parsing — the inbound half of the chat ACL. Turns Claude's
//! line-delimited JSON into a ChatReply (assistant text + usage) plus the
//! captured claude session_id (kept INSIDE the crate, F3). Reuses the exact
//! event shapes from `runners` but with no verdict/artifact convention — a chat
//! reply is plain prose.

use crate::chat::{ChatError, ChatReply, ChatUsage};
use serde_json::Value;

/// Accumulator fed one parsed JSON line at a time. Collects assistant text,
/// usage totals, and the claude session id (the F3-sealed value).
#[derive(Debug, Default)]
struct ChatAccumulator {
    text: String,
    usage: ChatUsage,
    session_id: Option<String>,
    saw_result: bool,
}

impl ChatAccumulator {
    fn feed(&mut self, v: &Value) -> Result<(), ChatError> {
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        // Rate-limit detection: an error event whose message mentions rate/429/quota.
        if ty == "error" || v.get("is_error").and_then(|b| b.as_bool()) == Some(true) {
            let msg = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| v.get("result").and_then(|r| r.as_str()))
                .unwrap_or("error")
                .to_string();
            let lower = msg.to_lowercase();
            if lower.contains("rate") || lower.contains("429") || lower.contains("quota") {
                return Err(ChatError::RateLimited(msg));
            }
        }

        // Capture the session id wherever it appears (init line or result line).
        if let Some(sid) = v.get("session_id").and_then(|s| s.as_str()) {
            self.session_id = Some(sid.to_string());
        }

        match ty {
            "system" => {
                if let Some(model) = v.get("model").and_then(|m| m.as_str()) {
                    self.usage.model = model.to_string();
                }
            }
            "assistant" => {
                if let Some(content) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                self.text.push_str(t);
                            }
                        }
                    }
                }
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.add_usage(u);
                }
            }
            "result" => {
                self.saw_result = true;
                if let Some(u) = v.get("usage") {
                    // The result usage is authoritative for totals — replace,
                    // keeping the model captured from the system line.
                    let model = std::mem::take(&mut self.usage.model);
                    self.usage = ChatUsage { model, ..Default::default() };
                    self.add_usage(u);
                }
                // Prefer the result's prose; fall back to accumulated assistant
                // text only when the result line carries no `result` string.
                if let Some(r) = v.get("result").and_then(|r| r.as_str()) {
                    self.text = r.to_string();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn add_usage(&mut self, u: &Value) {
        let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        self.usage.input_tokens += g("input_tokens");
        self.usage.output_tokens += g("output_tokens");
        self.usage.cache_creation += g("cache_creation_input_tokens");
        self.usage.cache_read += g("cache_read_input_tokens");
    }

    fn finish(self, model: &str) -> Result<(ChatReply, Option<String>), ChatError> {
        if !self.saw_result && self.text.is_empty() {
            return Err(ChatError::NoResult);
        }
        let mut usage = self.usage;
        if usage.model.is_empty() {
            usage.model = model.to_string();
        }
        Ok((ChatReply { text: self.text, usage }, self.session_id))
    }
}

/// Parse a full chat stream (newline-delimited JSON). Returns the ChatReply plus
/// the captured claude session id (consumed internally by ClaudeChatRunner, then
/// dropped — it never reaches the caller, F3).
pub fn parse_chat_stream(raw: &str, model: &str) -> Result<(ChatReply, Option<String>), ChatError> {
    let mut acc = ChatAccumulator::default();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| ChatError::Other(format!("bad stream-json line: {e}")))?;
        acc.feed(&v)?;
    }
    acc.finish(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = include_str!("fixtures/chat-first-turn.txt");
    const FOLLOW: &str = include_str!("fixtures/chat-follow-up.txt");

    #[test]
    fn parses_first_turn_text_usage_and_session_id() {
        let (reply, session) = parse_chat_stream(FIRST, "fallback-model").unwrap();
        // text is the result line's prose (authoritative when present)
        assert_eq!(reply.text, "T-042 is in the design stage; the spec is awaiting review.");
        // result usage is authoritative (900/18/120/40), not the assistant delta
        assert_eq!(reply.usage.input_tokens, 900);
        assert_eq!(reply.usage.output_tokens, 18);
        assert_eq!(reply.usage.cache_creation, 120);
        assert_eq!(reply.usage.cache_read, 40);
        assert_eq!(reply.usage.model, "claude-opus-4-8");
        // the session id was captured from the init line (stays inside the crate)
        assert_eq!(session.as_deref(), Some("sess-first"));
    }

    #[test]
    fn parses_follow_up_turn_with_same_session() {
        let (reply, session) = parse_chat_stream(FOLLOW, "m").unwrap();
        assert_eq!(reply.text, "Yes — I injected the topic; it is now task T-043.");
        assert_eq!(reply.usage.output_tokens, 12);
        assert_eq!(session.as_deref(), Some("sess-first"));
    }

    #[test]
    fn rate_limit_event_is_an_error() {
        let raw = r#"{"type":"error","error":{"message":"Rate limit exceeded (429)"}}"#;
        let err = parse_chat_stream(raw, "m").unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[test]
    fn empty_stream_is_no_result() {
        let err = parse_chat_stream("\n  \n", "m").unwrap_err();
        assert!(matches!(err, crate::chat::ChatError::NoResult));
    }

    #[test]
    fn malformed_line_is_other_error() {
        let err = parse_chat_stream("not json", "m").unwrap_err();
        assert!(matches!(err, crate::chat::ChatError::Other(_)));
    }

    #[test]
    fn text_falls_back_to_assistant_blocks_when_result_text_absent() {
        let raw = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s","model":"m"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"partial reply"}],"usage":{"output_tokens":4}}}"#, "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"output_tokens":4},"session_id":"s"}"#
        );
        let (reply, session) = parse_chat_stream(raw, "m").unwrap();
        assert_eq!(reply.text, "partial reply");
        assert_eq!(session.as_deref(), Some("s"));
    }
}
