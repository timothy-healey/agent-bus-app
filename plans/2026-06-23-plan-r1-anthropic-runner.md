# R1 — `anthropic-api` Runner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the second `RunnerKind` — a direct Anthropic Messages API runner using a per-team API key — behind the Runners ACL, and select the runner kind per team at the composition root.

**Architecture:** `AnthropicApiRunner` implements the existing `Runner` trait in `src-tauri/runners/`. The real HTTP call is isolated behind an injectable `SendFn` seam (mirroring `claude_cli.rs`'s `SpawnFn`); the production impl wires a `reqwest` blocking client, tests inject a canned-response closure. The request body is built by a pure, unit-tested function; the response (text + usage) is parsed from committed JSON fixtures into `RunnerOutput`. No anthropic/HTTP/SDK type crosses back past the `Runner` trait — Runtime sees only `RunnerOutput`/`RunnerError`. At the composition root (`app/src/lib.rs`), a small factory maps `team.effective_runner().kind` → `Arc<dyn Runner>`.

**Tech Stack:** Rust (cargo/rustc 1.95), `async-trait`, `serde`/`serde_json`, `reqwest` (already in the workspace lockfile via tauri; added as a direct dep with `json` + `rustls-tls`), `tokio`.

---

## Decisions

- **DD1 — HTTP crate.** Use `reqwest` (already pinned in `Cargo.lock` transitively). Add as a direct `runners` dep with `default-features = false, features = ["json", "rustls-tls"]`. Recommended option; keeps the call behind the seam so tests never hit the network. **AUTO-DECIDED: reqwest.**
- **DD2 — Sync vs async client at the seam.** The `SendFn` seam is a synchronous `Box<dyn Fn(&AnthropicRequest) -> Result<String, RunnerError>>` returning the raw response body string (mirrors `SpawnFn`'s `Fn(&[String]) -> Result<String, RunnerError>`). The production impl uses `reqwest::blocking` inside the closure. This keeps the seam identical in shape to the CLI runner and keeps the body-build + parse pure. The `Runner::invoke` async fn calls the sync seam directly (the CLI runner does the same — `process::Command::output()` is blocking). **AUTO-DECIDED: sync seam, blocking reqwest, mirrors SpawnFn.**
- **DD3 — Thinking param shape.** Build `thinking: {"type":"enabled","budget_tokens":N}` when `req.thinking_budget > 0`, omit the `thinking` field entirely when `0`. This maps `EffortMode`→budget exactly as the CLI runner does (`effort.budget_tokens()` is already resolved into `req.thinking_budget` by the pool — both runners consume the same field identically). The request uses `anthropic-version: 2023-06-01`. The committed model strings in this repo are `claude-opus-4-7`; the budget-driven `enabled` shape is the correct wire form for that family's predecessors and is what the seam is fixture-tested against (no live call). **AUTO-DECIDED: budget-driven enabled-thinking, symmetric with CLI runner.**
- **DD4 — Verdict/artifact parse reuse.** Reuse the existing `stream_json::{parse_verdict, parse_artifact}` (already `pub`) on the assistant text. The API runner concatenates all `content[].text` blocks into `final_text`, then runs the same lenient VERDICT:/ARTIFACT: convention. This keeps the two runners' verdict semantics identical and DRY. **AUTO-DECIDED: reuse parse_verdict/parse_artifact.**
- **DD5 — Error mapping.** HTTP 429 → `RunnerError::RateLimited`. A body whose top-level `type == "error"` with an `error.type`/`error.message` mentioning rate/429/quota → `RateLimited`. Transport failure (connect/timeout) → `RunnerError::Other`. A 4xx/5xx that isn't a rate limit → `RunnerError::Other` carrying the status + message. Empty/unparseable `content` → `RunnerError::NoResult`. **AUTO-DECIDED.**
- **DD6 — Runner-kind factory at the root.** A `runner_for(config: &RunnerConfig) -> Result<Arc<dyn Runner>, RunnerError>` factory in `app/src/lib.rs` maps `kind` → runner. `ClaudeCli` → `Arc::new(ClaudeCliRunner::new())`; `AnthropicApi` → resolve the API key from `config.api_key_env` via `std::env::var`, returning `RunnerError::Other` (NOT a panic) when the env var is absent/unresolvable, else `Arc::new(AnthropicApiRunner::new(key))`. Default stays claude-cli. The selection is per **team** (each team's `effective_runner()`). **AUTO-DECIDED.**
- **DD7 — Where the key lives.** The runner holds the resolved API key string (resolved once at the root from `api_key_env`). The `Runner` trait + `InvocationRequest` are unchanged — no key field crosses the ACL request shape. **AUTO-DECIDED.**
- **DD8 — Frontend surface.** None for v1.1. No runner-kind selection UI is added (S1 "Runners settings" is a separate backlog item). The wizard already authors `RunnerConfig.kind`/`api_key_env`; this item only makes `anthropic-api` actually run. **AUTO-DECIDED: backend-only.**

---

## File Structure

- **Create** `src-tauri/runners/src/anthropic_api.rs` — `AnthropicApiRunner`, the `SendFn` seam, the pure `build_request_body` fn, the pure `parse_response` fn, and all unit tests. One file, one responsibility: the anthropic-api ACL impl. Mirrors `claude_cli.rs` + `command.rs` + `stream_json.rs` collapsed into one module (the API surface is small enough that the split the CLI runner uses isn't warranted).
- **Create** `src-tauri/runners/src/fixtures/anthropic-messages-sample.json` — a committed real-shaped Messages API success response (text + usage).
- **Create** `src-tauri/runners/src/fixtures/anthropic-messages-rate-limit.json` — a committed `type: "error"` rate-limit body.
- **Modify** `src-tauri/runners/src/lib.rs` — add `pub mod anthropic_api;`.
- **Modify** `src-tauri/runners/Cargo.toml` — add `reqwest` dep.
- **Modify** `src-tauri/app/src/lib.rs` — add the `runner_for` factory; select per-team in `spawn_worker_loops`.
- **Modify** `docs/v1.1-backlog.md` — mark R1 done (final task).

---

## Task 1: Add the reqwest dependency

**Files:**
- Modify: `src-tauri/runners/Cargo.toml`

- [ ] **Step 1: Add reqwest to runners deps**

In `src-tauri/runners/Cargo.toml`, under `[dependencies]`, add:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "blocking", "rustls-tls"] }
```

(Place it after `tokio.workspace = true`.)

- [ ] **Step 2: Verify it resolves against the existing lockfile**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo check -p runners`
Expected: compiles (reqwest 0.12.x is already in `Cargo.lock`; this only promotes it to a direct dep). If cargo wants to update the lockfile, that is fine — confirm only the `runners` dependency edge is added, no version bumps to unrelated crates.

- [ ] **Step 3: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/Cargo.toml src-tauri/Cargo.lock
git commit -m "build(runners): add reqwest direct dependency for anthropic-api runner

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Commit the response fixtures

**Files:**
- Create: `src-tauri/runners/src/fixtures/anthropic-messages-sample.json`
- Create: `src-tauri/runners/src/fixtures/anthropic-messages-rate-limit.json`

- [ ] **Step 1: Write the success fixture**

Create `src-tauri/runners/src/fixtures/anthropic-messages-sample.json`:

```json
{
  "id": "msg_01ABC",
  "type": "message",
  "role": "assistant",
  "model": "claude-opus-4-7",
  "content": [
    { "type": "text", "text": "Analysing the repository.\n" },
    { "type": "text", "text": "VERDICT: approve\nARTIFACT: artifacts/analyses/T-1-v1.md" }
  ],
  "stop_reason": "end_turn",
  "usage": {
    "input_tokens": 1200,
    "output_tokens": 32,
    "cache_creation_input_tokens": 300,
    "cache_read_input_tokens": 50
  }
}
```

- [ ] **Step 2: Write the rate-limit fixture**

Create `src-tauri/runners/src/fixtures/anthropic-messages-rate-limit.json`:

```json
{
  "type": "error",
  "error": {
    "type": "rate_limit_error",
    "message": "Number of requests has exceeded your rate limit (429). Please retry later."
  }
}
```

- [ ] **Step 3: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/src/fixtures/anthropic-messages-sample.json src-tauri/runners/src/fixtures/anthropic-messages-rate-limit.json
git commit -m "test(runners): add anthropic Messages API response fixtures

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Pure request-body builder (TDD)

**Files:**
- Create: `src-tauri/runners/src/anthropic_api.rs`
- Modify: `src-tauri/runners/src/lib.rs`

- [ ] **Step 1: Register the module**

In `src-tauri/runners/src/lib.rs`, add `pub mod anthropic_api;` after `pub mod api;` (keep alphabetical-ish ordering with the existing modules — placing it right after `pub mod api;` is fine):

```rust
pub mod anthropic_api;
pub mod api;
pub mod claude_cli;
```

- [ ] **Step 2: Write the module skeleton + the failing request-body test**

Create `src-tauri/runners/src/anthropic_api.rs` with the module doc, the body builder signature (unimplemented), and the test:

```rust
//! AnthropicApiRunner — the v1.1 runner. Builds a Messages API request from an
//! InvocationRequest, sends it via an injectable SendFn seam (real reqwest in
//! production, a canned-response closure in tests), and parses the response
//! (text + usage) back into RunnerOutput. This is the Runners ACL for the
//! direct-API runner kind: NO anthropic/HTTP/SDK type crosses back past the
//! `Runner` trait — Runtime sees only RunnerOutput/RunnerError, exactly as the
//! CLI runner seals stream-json/flags. The only Claude-idiom side effect (the
//! HTTPS POST) is isolated behind the SendFn so the whole runner is unit-tested
//! without a live API key (mirrors claude_cli.rs's SpawnFn).

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

/// Build the Messages API request body (pure). Maps InvocationRequest →
/// the anthropic wire shape: model, system (operating prompt), one user
/// message, max_tokens, and budget-driven extended thinking. Mirrors how the
/// CLI runner maps the same fields to argv (command.rs::build_args).
pub fn build_request_body(req: &InvocationRequest) -> Value {
    let mut body = json!({
        "model": req.model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "system": req.system_prompt,
        "messages": [
            { "role": "user", "content": req.user_message }
        ],
    });
    // EffortMode→thinking budget, resolved by the caller into thinking_budget,
    // exactly as the CLI runner consumes it (--max-thinking-tokens). budget 0
    // = thinking off (omit the field), matching `EffortMode::Off`.
    if req.thinking_budget > 0 {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": req.thinking_budget,
        });
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // budget-driven thinking present
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
}
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p runners anthropic_api::tests::builds_the_exact_messages_request_body anthropic_api::tests::omits_thinking_when_budget_is_zero`
Expected: PASS (the builder is fully implemented in this task — the "failing" phase here is the assertion design; verify by temporarily breaking a field name if desired, then restore).

Note: this task implements the builder and its tests together because the body builder is a single pure function with no incremental sub-behaviors worth separating. If you prefer strict red-green, stub `build_request_body` to `json!({})`, watch the test fail, then fill it in.

- [ ] **Step 4: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/src/anthropic_api.rs src-tauri/runners/src/lib.rs
git commit -m "feat(runners): pure anthropic Messages API request-body builder

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Pure response parser (TDD)

**Files:**
- Modify: `src-tauri/runners/src/anthropic_api.rs`

- [ ] **Step 1: Write the failing response-parse tests**

Add to `src-tauri/runners/src/anthropic_api.rs`, above the `#[cfg(test)] mod tests` block, the parser declaration (stub first to see red), and add the tests inside `mod tests`. Stub:

```rust
/// Parse a Messages API response body (raw JSON text) into RunnerOutput (pure).
/// Concatenates all content[].text blocks into final_text, then applies the
/// SAME lenient VERDICT:/ARTIFACT: convention as the CLI runner (DRY: reuses
/// stream_json::parse_verdict / parse_artifact). Maps `usage` → RunnerUsage.
/// A `type:"error"` body mentioning rate/429/quota → RateLimited; empty content
/// → NoResult; malformed JSON → Other. `model` is the fallback when the
/// response omits it.
pub fn parse_response(raw: &str, model: &str) -> Result<RunnerOutput, RunnerError> {
    unimplemented!()
}
```

Tests (add inside `mod tests`):

```rust
    const SAMPLE: &str = include_str!("fixtures/anthropic-messages-sample.json");
    const RATE_LIMIT: &str = include_str!("fixtures/anthropic-messages-rate-limit.json");

    #[test]
    fn parses_sample_into_approve_with_artifact_and_usage() {
        let out = parse_response(SAMPLE, "fallback-model").unwrap();
        assert_eq!(out.verdict, agent_bus_core::Verdict::Approve);
        assert_eq!(out.artifact_path.as_deref(), Some("artifacts/analyses/T-1-v1.md"));
        // text is the concatenation of both content blocks
        assert!(out.final_text.contains("Analysing the repository."));
        assert!(out.final_text.contains("VERDICT: approve"));
        // usage mapped from the `usage` object
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p runners anthropic_api::tests::parses_sample_into_approve`
Expected: FAIL (panics on `unimplemented!()`).

- [ ] **Step 3: Implement `parse_response`**

Replace the `unimplemented!()` body with:

```rust
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
        let lower = format!("{kind} {msg}").to_lowercase();
        if lower.contains("rate") || lower.contains("429") || lower.contains("quota") {
            return Err(RunnerError::RateLimited(msg));
        }
        return Err(RunnerError::Other(msg));
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p runners anthropic_api::tests`
Expected: PASS (all parse + body tests).

- [ ] **Step 5: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/src/anthropic_api.rs
git commit -m "feat(runners): pure anthropic response parser (text+usage -> RunnerOutput)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: The SendFn seam + Runner impl (TDD)

**Files:**
- Modify: `src-tauri/runners/src/anthropic_api.rs`

- [ ] **Step 1: Write the failing Runner-trait tests**

Add to `mod tests` in `src-tauri/runners/src/anthropic_api.rs`:

```rust
    #[tokio::test]
    async fn invoke_parses_canned_response_without_a_real_key() {
        // The seam receives the built body and returns a canned response string.
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
```

Add the seam type + struct + constructors + `Runner` impl, above `mod tests`:

```rust
/// Produces the raw response body for a built request. Async-free + boxed so
/// the real implementation can POST synchronously (reqwest::blocking) and tests
/// can inject a canned-body closure. Returns Err(RunnerError) on transport /
/// rate-limit-by-status failure (the cases the body parser can't see). Mirrors
/// claude_cli::SpawnFn.
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
                // Map HTTP 429 → RateLimited at the status layer (the body parser
                // also catches type:"error" rate-limit envelopes — both paths
                // converge on RunnerError::RateLimited).
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
    // Note: invoke_stream is NOT overridden. The default in the Runner trait
    // delegates to invoke (no deltas) — the non-streaming Messages API call has
    // no per-chunk live-log path in v1.1. R4's streaming is a CLI-runner feature.
}
```

- [ ] **Step 2: Run tests to verify they fail then pass**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p runners anthropic_api::tests`
Expected: with the struct + impl in place, all PASS. (If you staged the struct as a stub first, the invoke tests fail until `Runner` is implemented.)

- [ ] **Step 3: Confirm the ACL seals — no anthropic/HTTP type leaks**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && grep -n "reqwest\|serde_json::Value\|anthropic" runners/src/output.rs`
Expected: NO matches. The `Runner` trait, `InvocationRequest`, `RunnerOutput`, `RunnerError`, `RunnerUsage` mention none of reqwest/Value/anthropic. The seam (`Value`, `reqwest`) is confined to `anthropic_api.rs`. This is the seal check (the same way `claude_cli.rs` keeps stream-json/flags out of `output.rs`).

- [ ] **Step 4: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/src/anthropic_api.rs
git commit -m "feat(runners): AnthropicApiRunner Runner impl behind injectable SendFn seam

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Runner-kind factory at the composition root (TDD)

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Write the failing factory test**

In `src-tauri/app/src/lib.rs`, add a test module near the existing `task_log_tests` (or extend it). The factory takes a resolved `RunnerConfig` and returns `Arc<dyn Runner>` or a clear error.

Add tests:

```rust
#[cfg(test)]
mod runner_factory_tests {
    use super::runner_for;
    use agent_bus_core::{EffortMode, RunnerKind};
    use pipeline::model::RunnerConfig;

    fn cfg(kind: RunnerKind, api_key_env: Option<&str>) -> RunnerConfig {
        RunnerConfig {
            kind,
            model: "claude-opus-4-7".into(),
            effort: EffortMode::Standard,
            api_key_env: api_key_env.map(|s| s.to_string()),
        }
    }

    #[test]
    fn claude_cli_kind_builds_a_runner() {
        let r = runner_for(&cfg(RunnerKind::ClaudeCli, None));
        assert!(r.is_ok(), "claude-cli must always build");
    }

    #[test]
    fn anthropic_api_with_resolvable_key_builds_a_runner() {
        // SAFETY: test-local env var, single-threaded within this test.
        std::env::set_var("R1_TEST_KEY_PRESENT", "sk-test-123");
        let r = runner_for(&cfg(RunnerKind::AnthropicApi, Some("R1_TEST_KEY_PRESENT")));
        std::env::remove_var("R1_TEST_KEY_PRESENT");
        assert!(r.is_ok(), "anthropic-api with a resolvable key must build");
    }

    #[test]
    fn anthropic_api_with_no_key_env_named_is_a_clear_error_not_a_panic() {
        let err = runner_for(&cfg(RunnerKind::AnthropicApi, None)).unwrap_err();
        match err {
            runners::output::RunnerError::Other(msg) => {
                assert!(msg.to_lowercase().contains("api_key_env"), "msg: {msg}");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn anthropic_api_with_unresolvable_key_is_a_clear_error_not_a_panic() {
        let err = runner_for(&cfg(RunnerKind::AnthropicApi, Some("R1_DEFINITELY_UNSET_ENV_VAR")))
            .unwrap_err();
        assert!(matches!(err, runners::output::RunnerError::Other(_)));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p app runner_factory_tests`
Expected: FAIL — `runner_for` is not defined.

- [ ] **Step 3: Implement `runner_for`**

In `src-tauri/app/src/lib.rs`, add imports near the top (the file already `use runners::claude_cli::ClaudeCliRunner;`):

```rust
use runners::anthropic_api::AnthropicApiRunner;
use runners::output::{Runner, RunnerError};
```

Add the factory function (place it directly above `fn spawn_worker_loops(`):

```rust
/// Composition-root factory: map a team's resolved RunnerConfig to a concrete
/// Runner. claude-cli is the default and always available. anthropic-api
/// resolves its per-team API key from `api_key_env` via the process
/// environment; a team requesting anthropic-api with no resolvable key yields
/// a clear RunnerError (NOT a panic) so the worker loop can surface it rather
/// than crash. The runner kind is chosen per team — Runtime depends only on
/// `Arc<dyn Runner>` and never learns which kind it got (the ACL seal).
fn runner_for(config: &pipeline::model::RunnerConfig) -> Result<Arc<dyn Runner>, RunnerError> {
    use agent_bus_core::RunnerKind;
    match config.kind {
        RunnerKind::ClaudeCli => Ok(Arc::new(ClaudeCliRunner::new())),
        RunnerKind::AnthropicApi => {
            let env_name = config.api_key_env.as_deref().ok_or_else(|| {
                RunnerError::Other(
                    "anthropic-api runner requires `api_key_env` to be set on the team's runner config".into(),
                )
            })?;
            let key = std::env::var(env_name).map_err(|_| {
                RunnerError::Other(format!(
                    "anthropic-api runner: API key env var `{env_name}` is not set"
                ))
            })?;
            Ok(Arc::new(AnthropicApiRunner::new(key)))
        }
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p app runner_factory_tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): runner-kind factory at the composition root (claude-cli | anthropic-api)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Select the runner per team in `spawn_worker_loops`

**Files:**
- Modify: `src-tauri/app/src/lib.rs:982-1039` (the `spawn_worker_loops` fn)

- [ ] **Step 1: Replace the single shared runner with a per-team selection**

In `spawn_worker_loops`, the current line (~994) is:

```rust
    let runner: Arc<dyn runners::output::Runner> = Arc::new(ClaudeCliRunner::new());
```

and the per-team loop later builds `PoolContext { runner: runner.clone(), .. }`.

Replace the single `let runner = ...` line by removing it, and inside the `for team in pipeline.teams.clone()` loop, select the runner per team. Change the loop body so that immediately after `let team = team.clone();` (and before constructing `PoolContext`), it resolves:

```rust
    let fanout = Arc::new(runtime::fanout_store::FanOutStore::new(pool));
    // Fallback runner if a team's anthropic-api key can't be resolved: keep the
    // worker loop alive on the default claude-cli runner rather than panicking.
    // (A team that genuinely needs anthropic-api will produce a clear error from
    // its own invocation; we never crash the whole pool over one team's config.)
    for team in pipeline.teams.clone() {
        let effective = team.effective_runner();
        let runner: Arc<dyn runners::output::Runner> = match runner_for(&effective) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "app: team `{}` runner selection failed ({e}); falling back to claude-cli",
                    team.id
                );
                Arc::new(ClaudeCliRunner::new())
            }
        };
        let ctx = PoolContext {
            pipeline: pipeline.clone(),
            runner,
            tasks: tasks.clone(),
            fanout: fanout.clone(),
            brake: brake.clone(),
            project_root: std::path::PathBuf::from(&project_root),
            read_prompt: Arc::new({
                let root = project_root.clone();
                move |t: &Team| {
                    std::fs::read_to_string(std::path::Path::new(&root).join(&t.prompt))
                        .unwrap_or_default()
                }
            }),
            usage_sink: usage_sink.clone(),
            revision_reader: revision_reader.clone(),
            log_sink: log_sink.clone(),
            audit: audit.clone(),
        };
        let handle = handle.clone();
        let team = team.clone();
        // ... rest of the existing spawn body unchanged ...
```

Concretely: delete the standalone `let runner: Arc<dyn ...> = Arc::new(ClaudeCliRunner::new());` line; keep the `let fanout = ...` line; add the `let effective = ...; let runner = match runner_for(&effective) {..}` block at the top of the loop; change `runner: runner.clone(),` in `PoolContext` to `runner,` (each iteration now owns its own `runner`).

- [ ] **Step 2: Verify the crate compiles**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo check -p app`
Expected: compiles. If the borrow checker complains that `team` is moved by `team.effective_runner()` then re-used, note that `effective_runner()` borrows `&self` (it returns an owned `RunnerConfig` by clone) so `team` remains usable; the existing `let team = team.clone();` further down still works.

- [ ] **Step 3: Run the app crate tests**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p app`
Expected: PASS (existing tests + the new `runner_factory_tests`).

- [ ] **Step 4: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): select runner kind per team in spawn_worker_loops (default claude-cli)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Full workspace verification

**Files:** none (verification only)

- [ ] **Step 1: cargo test workspace**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test --workspace`
Expected: PASS — all existing tests green, plus the new anthropic_api + runner_factory tests.

- [ ] **Step 2: cargo check workspace**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo check --workspace`
Expected: clean.

- [ ] **Step 3: cargo clippy workspace**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo clippy --workspace`
Expected: clean (no new warnings). If clippy flags the closure-in-`new` or the `g` usage closure, address minimally (e.g. allow or refactor) without changing behavior.

- [ ] **Step 4: bun vitest**

Run: `cd /Users/tim/projects/agent-bus-app && export PATH=/opt/homebrew/bin:$PATH && bun vitest run`
Expected: PASS — frontend untouched (DD8), so the existing suite is unchanged and green.

- [ ] **Step 5: bun build**

Run: `cd /Users/tim/projects/agent-bus-app && export PATH=/opt/homebrew/bin:$PATH && bun run build`
Expected: succeeds.

- [ ] **Step 6: Commit (only if any fixups were needed)**

```bash
cd /Users/tim/projects/agent-bus-app
git add -A
git commit -m "chore(r1): verification fixups (clippy/build)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Merge, tag, backlog

**Files:**
- Modify: `docs/v1.1-backlog.md`

- [ ] **Step 1: Update the backlog entry**

In `docs/v1.1-backlog.md`, change the R1 line from:

```markdown
- [ ] **R1 · `anthropic-api` runner** — v1 is claude-cli only; add the per-team API-key runner (`RunnerKind::AnthropicApi` is a forward-pointer). Source: design doc §v1 boundary 4.
```

to:

```markdown
- [x] **R1 · `anthropic-api` runner** — `done` (tag `plan-r1-anthropic-runner`) — second `RunnerKind` shipped: `AnthropicApiRunner` (`runners/src/anthropic_api.rs`) implements the `Runner` trait, sending a direct Messages API request behind an injectable `SendFn` seam (mirrors `claude_cli.rs`'s `SpawnFn`; real `reqwest::blocking` POST in prod, canned-response closure in tests). Pure `build_request_body` (asserted JSON: model/system/user/`max_tokens`/budget-driven `thinking`) + pure `parse_response` (text + usage → `RunnerOutput`, reusing `parse_verdict`/`parse_artifact`) are fixture-tested. The ACL SEALS the API idiom — no anthropic/HTTP/`serde_json::Value` type crosses back past `Runner` (verified: `output.rs` mentions none). HTTP 429 → `RunnerError::RateLimited`. A `runner_for` factory at the composition root (`app/src/lib.rs`) maps `team.effective_runner().kind` → `Arc<dyn Runner>` per team (default claude-cli); anthropic-api resolves its key from `api_key_env` and yields a clear `RunnerError` (never a panic) when unresolvable. DDD vet `docs/vet-r1-anthropic-runner-2026-06-23.md`. Backend-only — no frontend surface (runner-kind selection UI stays under S1). Caveat: the live HTTPS POST can't run headless (no key), so the request-build + response-parse + 429-mapping are covered via fixtures/fakes; the real call is structural-only (same seam path). Added `reqwest` as a direct `runners` dep. Source: design doc §v1 boundary 4.
```

- [ ] **Step 2: Commit the backlog update**

```bash
cd /Users/tim/projects/agent-bus-app
git add docs/v1.1-backlog.md
git commit -m "docs(backlog): mark R1 anthropic-api runner done

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 3: Merge --no-ff into main and tag**

```bash
cd /Users/tim/projects/agent-bus-app
git checkout main
git merge --no-ff plan-r1-anthropic-runner -m "Merge plan-r1-anthropic-runner: anthropic-api runner (R1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
git tag plan-r1-anthropic-runner
git branch -d plan-r1-anthropic-runner
```

Do NOT push. Leave the working tree on `main`.

- [ ] **Step 4: Final confirmation**

Run: `cd /Users/tim/projects/agent-bus-app && git log --oneline -8 && git tag | grep r1 && git branch --show-current`
Expected: merge commit at HEAD, `plan-r1-anthropic-runner` tag present, on `main`.

---

## Self-Review

- **Spec coverage:** `AnthropicApiRunner` implementing `Runner` (Tasks 3–5) ✓; injectable client seam mirroring `SpawnFn` (Task 5 `SendFn`) ✓; pure unit-tested request body asserting exact JSON (Task 3) ✓; response parse from committed fixtures (Tasks 2,4) ✓; usage → `RunnerOutput` (Task 4) ✓; ACL seals the API idiom — no leak past `Runner` (Task 5 Step 3 grep check) ✓; HTTP 429 → `RateLimited` (Task 5 `new`, Task 4 error-body) ✓; `EffortMode`→thinking budget same as CLI (Task 3, via `req.thinking_budget`) ✓; per-team factory at the root, default claude-cli, clear error not panic on missing key (Tasks 6–7) ✓; build/test/clippy/vitest/build verification (Task 8) ✓; merge/tag/backlog (Task 9) ✓.
- **Placeholder scan:** no TBD/TODO; all code shown in full.
- **Type consistency:** `build_request_body(&InvocationRequest) -> Value`, `parse_response(&str, &str) -> Result<RunnerOutput, RunnerError>`, `SendFn = Box<dyn Fn(&Value) -> Result<String, RunnerError> + Send + Sync>`, `AnthropicApiRunner::{new(String), with_sender(SendFn)}`, `runner_for(&RunnerConfig) -> Result<Arc<dyn Runner>, RunnerError>` — names consistent across Tasks 3–7. `RunnerConfig` fields (`kind`, `model`, `effort`, `api_key_env`) match `pipeline/src/model.rs`. `RunnerUsage` fields (`model`, `input_tokens`, `output_tokens`, `cache_creation`, `cache_read`) match `output.rs`.
