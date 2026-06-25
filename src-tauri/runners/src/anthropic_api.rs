//! AnthropicApiRunner — the v1.1 runner. Builds a Messages API request from an
//! InvocationRequest, sends it via an injectable SendFn seam (real reqwest in
//! production, a canned-response closure in tests), and parses the response
//! (text + usage) back into RunnerOutput.
//!
//! This is the Runners ACL for the direct-API runner kind. `invoke` is one
//! **Invocation** (DOMAIN.md → Runners: "one Claude call: CLI subprocess or API
//! request"); `SendFn` is the sealed *transport* — the HTTPS round-trip — and is
//! the only Claude-idiom side effect, isolated so the whole runner is unit-tested
//! without a live API key (mirrors `claude_cli.rs`'s `SpawnFn`, which seals the
//! subprocess spawn). NO anthropic/HTTP/SDK/`serde_json::Value` type crosses back
//! past the `Runner` trait — Runtime sees only RunnerOutput/RunnerError, exactly
//! as the CLI runner seals stream-json/flags inside `command.rs`/`stream_json.rs`.

use crate::output::{InvocationRequest, Runner, RunnerError, RunnerOutput, RunnerUsage};
use crate::stream_json::{parse_artifact, parse_verdict};
use async_trait::async_trait;
use serde_json::{json, Value};

/// The Messages API endpoint + version pinned by the ACL.
pub const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Hard cap on output tokens for a worker invocation. A worker emits a short
/// verdict line + an artifact path, not long prose; this bounds runaway output.
pub const MAX_OUTPUT_TOKENS: u32 = 8192;

/// Build the Messages API request body (pure). Maps InvocationRequest → the
/// anthropic wire shape: model, system (operating prompt), one user message,
/// max_tokens, and budget-driven extended thinking. Mirrors how the CLI runner
/// maps the same fields to argv (`command.rs::build_args`).
pub fn build_request_body(req: &InvocationRequest) -> Value {
    let mut body = json!({
        "model": req.model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "system": req.system_prompt,
        "messages": [
            { "role": "user", "content": req.user_message }
        ],
    });
    // EffortMode→thinking budget, already resolved by the caller into
    // thinking_budget (the SAME field the CLI runner consumes via
    // --max-thinking-tokens). budget 0 = thinking off (omit the field),
    // matching `EffortMode::Off`.
    if req.thinking_budget > 0 {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": req.thinking_budget,
        });
    }
    body
}

/// Parse a Messages API response body (raw JSON text) into RunnerOutput (pure).
/// Concatenates all content[].text blocks into final_text, then applies the SAME
/// lenient VERDICT:/ARTIFACT: convention as the CLI runner (DRY: reuses
/// stream_json::parse_verdict / parse_artifact). Maps `usage` → RunnerUsage.
///
/// Error mapping: a `type:"error"` body whose error type/message mentions
/// rate/429/quota → RateLimited (the body-layer half of the 429 mapping; the
/// transport layer in `AnthropicApiRunner::new` maps an HTTP 429 status to the
/// same RunnerError::RateLimited — two layers, one outcome); any other
/// `type:"error"` → Other; empty content → NoResult; malformed JSON → Other.
/// `model` is the fallback when the response omits it.
pub fn parse_response(raw: &str, model: &str) -> Result<RunnerOutput, RunnerError> {
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| RunnerError::Other(format!("bad anthropic response: {e}")))?;

    // Error envelope: { "type": "error", "error": { "type", "message" } }.
    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
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
        // Classify the error envelope distinctly: rate/quota → RateLimited,
        // model-not-found/unavailable → ModelUnavailable (G6), else → Other.
        // The error `type` (e.g. "not_found_error") + message both feed the
        // signal — a 404/not_found_error referencing the model is unavailable.
        let lower = format!("{kind} {msg}").to_lowercase();
        return Err(RunnerError::classify(&lower, msg));
    }

    // Concatenate all text content blocks.
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
        return Err(RunnerError::NoResult);
    }

    let verdict = parse_verdict(&text);
    let artifact_path = parse_artifact(&text);

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
    let usage = RunnerUsage {
        model: resolved_model,
        input_tokens: g("input_tokens"),
        output_tokens: g("output_tokens"),
        cache_creation: g("cache_creation_input_tokens"),
        cache_read: g("cache_read_input_tokens"),
    };

    Ok(RunnerOutput { verdict, artifact_path, final_text: text, usage })
}

/// Produces the raw response body for a built request. Async-free + boxed so the
/// real implementation can POST synchronously (reqwest::blocking) and tests can
/// inject a canned-body closure. Returns Err(RunnerError) on transport /
/// rate-limit-by-status failure (the cases the body parser can't see). This is
/// the sealed transport for an Invocation — mirrors `claude_cli::SpawnFn`.
pub type SendFn = Box<dyn Fn(&Value) -> Result<String, RunnerError> + Send + Sync>;

pub struct AnthropicApiRunner {
    send: SendFn,
}

impl AnthropicApiRunner {
    /// The production runner: POSTs to the Messages API with the per-team key.
    /// `api_key` is resolved once at the composition root from
    /// `RunnerConfig.api_key_env`; it never crosses the Runner trait.
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
                    .map_err(|e| RunnerError::Other(format!("anthropic request failed: {e}")))?;
                // Map HTTP 429 → RateLimited at the status layer. The body parser
                // (`parse_response`) also catches a `type:"error"` rate-limit
                // envelope — both paths converge on RunnerError::RateLimited
                // because a 429 may surface as either a status or a JSON error.
                if resp.status().as_u16() == 429 {
                    let msg = resp.text().unwrap_or_else(|_| "429".into());
                    return Err(RunnerError::RateLimited(msg));
                }
                resp.text()
                    .map_err(|e| RunnerError::Other(format!("anthropic read failed: {e}")))
            }),
        }
    }

    /// Test/alternate constructor: inject the response producer (no network).
    pub fn with_sender(send: SendFn) -> Self {
        Self { send }
    }
}

#[async_trait]
impl Runner for AnthropicApiRunner {
    async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
        let body = build_request_body(req);
        let raw = (self.send)(&body)?;
        parse_response(&raw, &req.model)
    }
    // invoke_stream is NOT overridden. The default in the Runner trait delegates
    // to invoke (no deltas) — the non-streaming Messages API call has no
    // per-chunk live-log path in v1.1. R4's streaming (Log delta / LogSink) is a
    // CLI-runner feature.
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("fixtures/anthropic-messages-sample.json");
    const RATE_LIMIT: &str = include_str!("fixtures/anthropic-messages-rate-limit.json");

    fn req() -> InvocationRequest {
        InvocationRequest {
            task_id: "T-1".into(),
            team_id: "research".into(),
            model: "claude-opus-4-7".into(),
            thinking_budget: 8192,
            system_prompt: "You are research.".into(),
            user_message: "Investigate topic X".into(),
            settings_path: "/tmp/s.json".into(),
            add_dirs: vec![],
            sandbox_profile: None,
        }
    }

    #[test]
    fn builds_the_exact_messages_request_body() {
        let body = build_request_body(&req());
        assert_eq!(body["model"], "claude-opus-4-7");
        assert_eq!(body["max_tokens"], MAX_OUTPUT_TOKENS);
        assert_eq!(body["system"], "You are research.");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Investigate topic X");
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8192);
    }

    #[test]
    fn omits_thinking_when_budget_is_zero() {
        let mut r = req();
        r.thinking_budget = 0;
        let body = build_request_body(&r);
        assert!(body.get("thinking").is_none(), "thinking must be omitted at budget 0");
    }

    #[test]
    fn parses_sample_into_approve_with_artifact_and_usage() {
        let out = parse_response(SAMPLE, "fallback-model").unwrap();
        assert_eq!(out.verdict, agent_bus_core::Verdict::Approve);
        assert_eq!(out.artifact_path.as_deref(), Some("artifacts/analyses/T-1-v1.md"));
        assert!(out.final_text.contains("Analysing the repository."));
        assert!(out.final_text.contains("VERDICT: approve"));
        assert_eq!(out.usage.input_tokens, 1200);
        assert_eq!(out.usage.output_tokens, 32);
        assert_eq!(out.usage.cache_creation, 300);
        assert_eq!(out.usage.cache_read, 50);
        // model comes from the response body, not the fallback
        assert_eq!(out.usage.model, "claude-opus-4-7");
    }

    #[test]
    fn rate_limit_error_body_maps_to_rate_limited() {
        let err = parse_response(RATE_LIMIT, "m").unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[test]
    fn empty_content_is_no_result() {
        let raw = r#"{"type":"message","model":"m","content":[],"usage":{"input_tokens":1,"output_tokens":0}}"#;
        let err = parse_response(raw, "m").unwrap_err();
        assert!(matches!(err, RunnerError::NoResult));
    }

    #[test]
    fn malformed_json_is_other_error() {
        let err = parse_response("not json", "m").unwrap_err();
        assert!(matches!(err, RunnerError::Other(_)));
    }

    #[test]
    fn falls_back_to_model_arg_when_response_omits_model() {
        let raw = r#"{"type":"message","content":[{"type":"text","text":"VERDICT: approve"}],"usage":{"input_tokens":5,"output_tokens":7}}"#;
        let out = parse_response(raw, "the-fallback").unwrap();
        assert_eq!(out.usage.model, "the-fallback");
        assert_eq!(out.verdict, agent_bus_core::Verdict::Approve);
    }

    #[test]
    fn non_rate_limit_error_body_maps_to_other() {
        let raw = r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad model"}}"#;
        let err = parse_response(raw, "m").unwrap_err();
        assert!(matches!(err, RunnerError::Other(_)));
        assert!(!err.is_rate_limited());
    }

    #[test]
    fn not_found_model_error_body_maps_to_model_unavailable() {
        let raw = r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-nope not found"}}"#;
        let err = parse_response(raw, "m").unwrap_err();
        assert!(matches!(err, RunnerError::ModelUnavailable(_)), "got {err:?}");
        assert!(err.is_model_unavailable());
    }

    #[tokio::test]
    async fn invoke_parses_canned_response_without_a_real_key() {
        let runner = AnthropicApiRunner::with_sender(Box::new(move |body: &Value| {
            // confirm the ACL built a real Messages request before "sending"
            assert_eq!(body["model"], "claude-opus-4-7");
            assert_eq!(body["messages"][0]["role"], "user");
            Ok(SAMPLE.to_string())
        }));
        let out = runner.invoke(&req()).await.unwrap();
        assert_eq!(out.verdict, agent_bus_core::Verdict::Approve);
        assert_eq!(out.artifact_path.as_deref(), Some("artifacts/analyses/T-1-v1.md"));
        assert_eq!(out.usage.input_tokens, 1200);
    }

    #[tokio::test]
    async fn invoke_maps_rate_limit_response() {
        let runner = AnthropicApiRunner::with_sender(Box::new(|_b| Ok(RATE_LIMIT.to_string())));
        let err = runner.invoke(&req()).await.unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[tokio::test]
    async fn invoke_propagates_transport_failure_as_other() {
        let runner = AnthropicApiRunner::with_sender(Box::new(|_b| {
            Err(RunnerError::Other("connection refused".into()))
        }));
        let err = runner.invoke(&req()).await.unwrap_err();
        assert!(matches!(err, RunnerError::Other(_)));
    }
}
