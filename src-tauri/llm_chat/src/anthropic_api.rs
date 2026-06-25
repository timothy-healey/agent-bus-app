//! AnthropicApiChatRunner — the direct-API chat runner. Builds a Messages API
//! request from a `ChatRequest`, sends it via an injectable `SendFn` seam (real
//! `reqwest::blocking` in production, a canned-response closure in tests), and
//! parses the response back into the kernel chat shapes.
//!
//! This is the llm_chat ACL for the direct-API chat kind — the API sibling of
//! `claude_cli.rs`. It mirrors `runners::anthropic_api::AnthropicApiRunner`: a
//! pure `build_request_body`, a pure response parser, the 429-by-status mapping,
//! and the sealed `SendFn` transport. The `tool_use`/`tool_choice`/anthropic JSON
//! idiom stays INSIDE this file — only `ChatReply`/`ChatToolDef`/`StructuredReply`
//! (Value-based) cross the `ChatRunner` trait (the ACL seal, F3). There is no
//! session id here: the non-streaming Messages call is stateless and the caller's
//! `dialogue_id` never crosses out.

use crate::chat::{
    ChatError, ChatReply, ChatRequest, ChatRunner, ChatToolDef, ChatUsage, StructuredReply,
};
use async_trait::async_trait;
use serde_json::{json, Value};

/// The Messages API endpoint + version pinned by the ACL.
pub const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Hard cap on output tokens for a chat turn. A terminal/Design-Session turn is a
/// short reply or one tool call, not long prose; this bounds runaway output.
pub const MAX_OUTPUT_TOKENS: u32 = 8192;

/// Build the Messages API request body (pure). Maps `ChatRequest` → the anthropic
/// wire shape: model, system, one user message, max_tokens, budget-driven extended
/// thinking. When `tools` is non-empty the request carries the `tools` array and
/// (optionally) a forced `tool_choice` — this is the only place the anthropic
/// tool-use idiom is constructed. `force` pins the model to a specific tool name;
/// `None` with tools present sets `tool_choice` to `{ "type": "any" }` (the model
/// MUST call one of the tools). Extended thinking is incompatible with a forced
/// tool_choice, so it is omitted whenever a tool_choice is set.
pub fn build_request_body(req: &ChatRequest, tools: &[ChatToolDef], force: Option<&str>) -> Value {
    let mut body = json!({
        "model": req.model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "system": req.system_prompt,
        "messages": [
            { "role": "user", "content": req.user_message }
        ],
    });

    let tool_choice_set = !tools.is_empty();
    if tool_choice_set {
        let tool_defs: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();
        body["tools"] = Value::Array(tool_defs);
        body["tool_choice"] = match force {
            Some(name) => json!({ "type": "tool", "name": name }),
            None => json!({ "type": "any" }),
        };
    }

    // EffortMode→thinking budget. A forced/any tool_choice is incompatible with
    // extended thinking, so thinking is only enabled on the plain text path.
    if req.thinking_budget > 0 && !tool_choice_set {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": req.thinking_budget,
        });
    }
    body
}

/// Classify a `type:"error"` envelope into a `ChatError` (pure). Rate/quota/429 →
/// `RateLimited`; anything else → `Other`. Mirrors the runner's body-layer 429
/// half (the transport layer maps an HTTP 429 status to the same outcome).
fn classify_error_envelope(v: &Value) -> ChatError {
    let msg = v
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("error")
        .to_string();
    let kind = v
        .get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let lower = format!("{kind} {msg}").to_lowercase();
    if lower.contains("rate") || lower.contains("429") || lower.contains("quota") {
        ChatError::RateLimited(msg)
    } else {
        ChatError::Other(msg)
    }
}

/// Read `usage` → `ChatUsage` (pure). `model` is the fallback when the response
/// omits it.
fn parse_usage(v: &Value, model: &str) -> ChatUsage {
    let resolved_model = v
        .get("model")
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(model)
        .to_string();
    let g = |k: &str| {
        v.get("usage")
            .and_then(|u| u.get(k))
            .and_then(|x| x.as_u64())
            .unwrap_or(0)
    };
    ChatUsage {
        model: resolved_model,
        input_tokens: g("input_tokens"),
        output_tokens: g("output_tokens"),
        cache_creation: g("cache_creation_input_tokens"),
        cache_read: g("cache_read_input_tokens"),
    }
}

/// Parse a Messages API response (raw JSON text) into a plain-text `ChatReply`
/// (pure). Concatenates all `content[].text` blocks (the same shape the runner
/// uses); `tool_use` blocks are ignored on this path. Error envelope → mapped
/// `ChatError`; empty text → `NoResult`; malformed JSON → `Other`.
pub fn parse_text_response(raw: &str, model: &str) -> Result<ChatReply, ChatError> {
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| ChatError::Other(format!("bad anthropic response: {e}")))?;

    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
        return Err(classify_error_envelope(&v));
    }

    let mut text = String::new();
    if let Some(blocks) = v.get("content").and_then(|c| c.as_array()) {
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    text.push_str(t);
                }
            }
        }
    }
    if text.is_empty() {
        return Err(ChatError::NoResult);
    }

    Ok(ChatReply { text, usage: parse_usage(&v, model) })
}

/// Parse a Messages API response (raw JSON text) into a `StructuredReply` (pure):
/// finds the FIRST `tool_use` content block and maps `name` → `tool_name`,
/// `input` → `args`. This is the only place the anthropic `tool_use` idiom is
/// decoded — it never crosses the trait. Error envelope → mapped `ChatError`;
/// no `tool_use` block → `NoResult` (the model answered in prose instead of
/// calling a tool); malformed JSON → `Other`.
pub fn parse_structured_response(raw: &str, model: &str) -> Result<StructuredReply, ChatError> {
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| ChatError::Other(format!("bad anthropic response: {e}")))?;

    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
        return Err(classify_error_envelope(&v));
    }

    let tool_block = v
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        });

    let block = tool_block.ok_or(ChatError::NoResult)?;
    let tool_name = block
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| ChatError::Other("tool_use block missing `name`".into()))?
        .to_string();
    let args = block
        .get("input")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    Ok(StructuredReply { tool_name, args, usage: parse_usage(&v, model) })
}

/// Produces the raw response body for a built request. Async-free + boxed so the
/// real impl POSTs synchronously (reqwest::blocking) and tests inject a canned
/// body. Returns `Err(ChatError)` on transport / rate-limit-by-status failure
/// (the cases the body parser can't see). The sealed transport — mirrors
/// `claude_cli::SpawnFn` and `runners::anthropic_api::SendFn`.
pub type SendFn = Box<dyn Fn(&Value) -> Result<String, ChatError> + Send + Sync>;

pub struct AnthropicApiChatRunner {
    send: SendFn,
}

impl AnthropicApiChatRunner {
    /// The production runner: POSTs to the Messages API with the resolved key.
    /// `api_key` is resolved once at the composition root (keychain / api_key_env,
    /// S1); it never crosses the `ChatRunner` trait.
    pub fn new(api_key: String) -> Self {
        Self {
            send: Box::new(move |body: &Value| {
                let client = reqwest::blocking::Client::new();
                let resp = client
                    .post(ANTHROPIC_URL)
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .header("content-type", "application/json")
                    .json(body)
                    .send()
                    .map_err(|e| ChatError::Other(format!("anthropic request failed: {e}")))?;
                // Map HTTP 429 → RateLimited at the status layer; the body parser
                // also catches a rate-limit error envelope — both converge.
                if resp.status().as_u16() == 429 {
                    let msg = resp.text().unwrap_or_else(|_| "429".into());
                    return Err(ChatError::RateLimited(msg));
                }
                resp.text()
                    .map_err(|e| ChatError::Other(format!("anthropic read failed: {e}")))
            }),
        }
    }

    /// Test/alternate constructor: inject the response producer (no network).
    pub fn with_sender(send: SendFn) -> Self {
        Self { send }
    }
}

#[async_trait]
impl ChatRunner for AnthropicApiChatRunner {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError> {
        let body = build_request_body(req, &[], None);
        let raw = (self.send)(&body)?;
        parse_text_response(&raw, &req.model)
    }

    // chat_stream is NOT overridden: the non-streaming Messages call has no
    // per-chunk live-log path (same as the runner). The default delegates to
    // `chat` (no deltas).

    fn supports_structured(&self) -> bool {
        true
    }

    async fn chat_structured(
        &self,
        req: &ChatRequest,
        tools: &[ChatToolDef],
        force: Option<&str>,
    ) -> Result<StructuredReply, ChatError> {
        if tools.is_empty() {
            return Err(ChatError::Other(
                "chat_structured requires at least one tool".into(),
            ));
        }
        let body = build_request_body(req, tools, force);
        let raw = (self.send)(&body)?;
        parse_structured_response(&raw, &req.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOL_USE: &str = include_str!("fixtures/anthropic-tool-use-sample.json");
    const TEXT: &str = include_str!("fixtures/anthropic-text-sample.json");
    const RATE_LIMIT: &str = include_str!("fixtures/anthropic-rate-limit.json");

    fn req() -> ChatRequest {
        ChatRequest {
            dialogue_id: "d-1".into(),
            system_prompt: "You are the terminal.".into(),
            user_message: "emit the team set".into(),
            model: "claude-opus-4-8".into(),
            thinking_budget: 8192,
        }
    }

    fn tool() -> ChatToolDef {
        ChatToolDef {
            name: "emit_slice".into(),
            description: "Emit the pipeline slice".into(),
            input_schema: json!({ "type": "object", "properties": { "kind": { "type": "string" } } }),
        }
    }

    #[test]
    fn plain_text_body_has_no_tools_or_tool_choice() {
        let body = build_request_body(&req(), &[], None);
        assert_eq!(body["model"], "claude-opus-4-8");
        assert_eq!(body["max_tokens"], MAX_OUTPUT_TOKENS);
        assert_eq!(body["system"], "You are the terminal.");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "emit the team set");
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        // thinking enabled on the plain text path
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8192);
    }

    #[test]
    fn structured_body_carries_tools_and_forced_tool_choice() {
        let body = build_request_body(&req(), &[tool()], Some("emit_slice"));
        // tools array is the anthropic shape: name/description/input_schema
        assert_eq!(body["tools"][0]["name"], "emit_slice");
        assert_eq!(body["tools"][0]["description"], "Emit the pipeline slice");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        // forced tool_choice pins the named tool
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], "emit_slice");
        // thinking is omitted whenever a tool_choice is set (incompatible)
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn structured_body_without_force_uses_any_tool_choice() {
        let body = build_request_body(&req(), &[tool()], None);
        assert_eq!(body["tool_choice"]["type"], "any");
        assert!(body["tool_choice"].get("name").is_none());
    }

    #[test]
    fn parses_text_response_concatenating_blocks() {
        let reply = parse_text_response(TEXT, "fallback").unwrap();
        assert_eq!(reply.text, "T-042 is in the design stage; the spec is awaiting review.");
        assert_eq!(reply.usage.input_tokens, 900);
        assert_eq!(reply.usage.output_tokens, 28);
        // model comes from the response, not the fallback
        assert_eq!(reply.usage.model, "claude-opus-4-8");
    }

    #[test]
    fn parses_tool_use_response_into_structured_reply() {
        let reply = parse_structured_response(TOOL_USE, "fallback").unwrap();
        assert_eq!(reply.tool_name, "emit_slice");
        assert_eq!(reply.args["kind"], "teams");
        assert_eq!(reply.args["teams"][0]["id"], "research");
        assert_eq!(reply.usage.input_tokens, 800);
        assert_eq!(reply.usage.output_tokens, 64);
        assert_eq!(reply.usage.cache_creation, 100);
        assert_eq!(reply.usage.model, "claude-opus-4-8");
    }

    #[test]
    fn structured_parse_when_no_tool_use_is_no_result() {
        // a plain text response has no tool_use block
        let err = parse_structured_response(TEXT, "m").unwrap_err();
        assert!(matches!(err, ChatError::NoResult));
    }

    #[test]
    fn text_parse_rate_limit_envelope_maps_to_rate_limited() {
        let err = parse_text_response(RATE_LIMIT, "m").unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[test]
    fn structured_parse_rate_limit_envelope_maps_to_rate_limited() {
        let err = parse_structured_response(RATE_LIMIT, "m").unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[test]
    fn text_parse_empty_content_is_no_result() {
        let raw = r#"{"type":"message","model":"m","content":[],"usage":{"input_tokens":1}}"#;
        let err = parse_text_response(raw, "m").unwrap_err();
        assert!(matches!(err, ChatError::NoResult));
    }

    #[test]
    fn text_parse_malformed_json_is_other() {
        let err = parse_text_response("not json", "m").unwrap_err();
        assert!(matches!(err, ChatError::Other(_)));
    }

    #[test]
    fn non_rate_limit_error_envelope_is_other() {
        let raw = r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad"}}"#;
        let err = parse_structured_response(raw, "m").unwrap_err();
        assert!(matches!(err, ChatError::Other(_)));
        assert!(!err.is_rate_limited());
    }

    #[tokio::test]
    async fn supports_structured_is_true() {
        let runner = AnthropicApiChatRunner::with_sender(Box::new(|_b| Ok(TEXT.to_string())));
        assert!(runner.supports_structured());
    }

    #[tokio::test]
    async fn chat_parses_canned_text_without_a_real_key() {
        let runner = AnthropicApiChatRunner::with_sender(Box::new(move |body: &Value| {
            // the ACL built a real Messages request with no tools on the chat path
            assert_eq!(body["model"], "claude-opus-4-8");
            assert!(body.get("tools").is_none());
            Ok(TEXT.to_string())
        }));
        let reply = runner.chat(&req()).await.unwrap();
        assert_eq!(reply.text, "T-042 is in the design stage; the spec is awaiting review.");
        assert_eq!(reply.usage.input_tokens, 900);
    }

    #[tokio::test]
    async fn chat_structured_forces_the_tool_and_returns_the_call() {
        let runner = AnthropicApiChatRunner::with_sender(Box::new(move |body: &Value| {
            // the ACL forced the named tool before "sending"
            assert_eq!(body["tool_choice"]["type"], "tool");
            assert_eq!(body["tool_choice"]["name"], "emit_slice");
            assert_eq!(body["tools"][0]["name"], "emit_slice");
            Ok(TOOL_USE.to_string())
        }));
        let reply = runner.chat_structured(&req(), &[tool()], Some("emit_slice")).await.unwrap();
        assert_eq!(reply.tool_name, "emit_slice");
        assert_eq!(reply.args["kind"], "teams");
    }

    #[tokio::test]
    async fn chat_structured_with_no_tools_errors() {
        let runner = AnthropicApiChatRunner::with_sender(Box::new(|_b| Ok(TOOL_USE.to_string())));
        let err = runner.chat_structured(&req(), &[], None).await.unwrap_err();
        assert!(matches!(err, ChatError::Other(_)));
    }

    #[tokio::test]
    async fn chat_maps_rate_limit_response() {
        let runner = AnthropicApiChatRunner::with_sender(Box::new(|_b| Ok(RATE_LIMIT.to_string())));
        let err = runner.chat(&req()).await.unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[tokio::test]
    async fn chat_structured_maps_rate_limit_by_status() {
        let runner = AnthropicApiChatRunner::with_sender(Box::new(|_b| {
            Err(ChatError::RateLimited("429".into()))
        }));
        let err = runner
            .chat_structured(&req(), &[tool()], Some("emit_slice"))
            .await
            .unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[tokio::test]
    async fn chat_propagates_transport_failure_as_other() {
        let runner = AnthropicApiChatRunner::with_sender(Box::new(|_b| {
            Err(ChatError::Other("connection refused".into()))
        }));
        let err = runner.chat(&req()).await.unwrap_err();
        assert!(matches!(err, ChatError::Other(_)));
    }
}
