//! The Runners idiom — what crosses the ACL boundary back into Runtime.
//! Nothing here mentions stream-json, CLI flags, or HTTP: those are sealed
//! inside the concrete runners. Runtime sees only RunnerOutput/RunnerError.

use agent_bus_core::Verdict;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Token usage for one invocation, in Runtime's vocabulary. Runners publishes
/// this to Usage Telemetry (Customer-Supplier) in Plan 5; here it is just the
/// shape the runner records on completion.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

/// The result of one completed invocation, translated out of Claude's idiom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerOutput {
    /// The verdict the worker settled on (parsed from the model's final text).
    pub verdict: Verdict,
    /// Path (relative to project root) of the artifact the worker produced, if
    /// any. None when the team produces no artifact (e.g. a terminal "done").
    pub artifact_path: Option<String>,
    /// The full assistant text, kept for the live-log + diagnostics.
    pub final_text: String,
    /// Token usage recorded across the invocation.
    pub usage: RunnerUsage,
}

#[derive(Debug, Error)]
pub enum RunnerError {
    /// The provider returned a rate-limit / quota error. Per the spec, Runtime
    /// must release the held task and (Plan 5) set the brake on this.
    #[error("rate limited: {0}")]
    RateLimited(String),
    /// The requested model is not found / unavailable to this account (G6). A
    /// distinct class so the operator surface can say "model unavailable — pick
    /// another" instead of a generic failure (full card-side detail is L3's job;
    /// here we ensure the class + a clear message exist). Distinguished from a
    /// generic `Other` by the provider's not-found/unavailable signal.
    #[error("model unavailable: {0}")]
    ModelUnavailable(String),
    /// The subprocess could not be spawned (binary missing, etc.).
    #[error("spawn failed: {0}")]
    Spawn(String),
    /// The stream produced no parseable result.
    #[error("no result parsed from runner output")]
    NoResult,
    /// Any other failure, with a message.
    #[error("runner failed: {0}")]
    Other(String),
}

impl RunnerError {
    /// True when the failure is a rate-limit — the one case Runtime treats
    /// specially (release the task, don't bump attempts).
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, RunnerError::RateLimited(_))
    }

    /// True when the failure is a model-not-found/unavailable (G6) — surfaced as
    /// "model unavailable — pick another" rather than a generic error.
    pub fn is_model_unavailable(&self) -> bool {
        matches!(self, RunnerError::ModelUnavailable(_))
    }

    /// Classify a provider error message into a distinct class. Rate/quota/429 →
    /// `RateLimited`; a not-found/unavailable/unknown-model signal →
    /// `ModelUnavailable`; otherwise `Other`. Shared by the CLI (stderr) and API
    /// (error envelope) runners so both classify the same way (G6). `lower` is
    /// the lowercased text to inspect; `msg` is the original message to carry.
    pub fn classify(lower: &str, msg: String) -> RunnerError {
        if lower.contains("rate") || lower.contains("429") || lower.contains("quota") {
            RunnerError::RateLimited(msg)
        } else if is_model_unavailable_signal(lower) {
            RunnerError::ModelUnavailable(msg)
        } else {
            RunnerError::Other(msg)
        }
    }
}

/// True when the lowercased provider text signals a model-not-found / unavailable
/// condition (G6). Pure, so it is shared + unit-tested. Requires a `model`
/// mention paired with a specific not-found/unavailable phrase. Deliberately
/// does NOT trip on the bare word "invalid" (the generic `invalid_request_error`
/// type contains it) — only on "invalid model"/"model ... invalid", to avoid
/// mis-classifying unrelated validation errors as a model problem.
pub fn is_model_unavailable_signal(lower: &str) -> bool {
    if !lower.contains("model") {
        return false;
    }
    lower.contains("not found")
        || lower.contains("not_found")
        || lower.contains("unknown")
        || lower.contains("does not exist")
        || lower.contains("not available")
        || lower.contains("unavailable")
        || lower.contains("invalid model")
        || lower.contains("model is invalid")
        || lower.contains("404")
}

/// What the runner needs to perform one invocation. A plain owned struct so the
/// trait stays object-safe and callers (Runtime) don't need pipeline types in
/// their signatures beyond what they already hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationRequest {
    pub task_id: String,
    pub team_id: String,
    pub model: String,
    /// Thinking-token budget, already resolved from EffortMode by the caller.
    pub thinking_budget: u32,
    /// The team's operating prompt (already read from prompts/<team>.md).
    pub system_prompt: String,
    /// The user message handed to the model (e.g. the topic, or revise notes).
    pub user_message: String,
    /// Absolute path to the per-invocation settings.json (written by Scope).
    pub settings_path: String,
    /// Directories the runner should grant via --add-dir.
    pub add_dirs: Vec<String>,
    /// **EXPERIMENTAL (S3) · macOS-only · CLI-runner-only.** When `Some`, the CLI
    /// runner wraps the `claude` subprocess in `sandbox-exec -p <profile>`.
    /// CLI-shaped data owned by the Runners ACL — the SBPL idiom never crosses
    /// the `Runner` trait outward: Runtime sets only the engine's `sandbox`
    /// bool and never reads this string. `None` (the default) = unchanged
    /// behavior. The AnthropicApiRunner ignores it (no subprocess to confine).
    /// Live confinement is structural-only (unverified) — NOT a proven boundary.
    pub sandbox_profile: Option<String>,
}

/// A display-only log sink. The streaming worker path forwards each assistant
/// text fragment here as it parses, for live-log display. Deliberately a plain
/// `&str` callback: NO stream-json idiom, CLI flag, or event name crosses the
/// ACL through it — the composition root maps fragments to whatever UI event it
/// likes. Mirrors `llm_chat::chat::DeltaSink` (separate ACL crate, by design —
/// the worker ACL is verdict-shaped, the chat ACL is prose-shaped; vet F2).
pub type LogSink = Box<dyn Fn(&str) + Send + Sync>;

/// The ACL seam. Runtime depends only on this trait; the concrete runner kind
/// is selected once at the composition root. Object-safe so it can be held as
/// `Arc<dyn Runner>`.
#[async_trait]
pub trait Runner: Send + Sync {
    async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError>;

    /// Streaming variant: identical contract to `invoke` (same final
    /// `RunnerOutput` — verdict, artifact, usage), but assistant prose fragments
    /// are forwarded to `sink` as they arrive for live-log display. The verdict/
    /// artifact/usage parse + the value returned are unchanged; streaming is
    /// purely additive. The default delegates to `invoke` (no deltas) so existing
    /// runners keep working; streaming runners override this.
    async fn invoke_stream(
        &self,
        req: &InvocationRequest,
        sink: &LogSink,
    ) -> Result<RunnerOutput, RunnerError> {
        let _ = sink;
        self.invoke(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_output_round_trips() {
        let out = RunnerOutput {
            verdict: Verdict::Approve,
            artifact_path: Some("artifacts/specs/T-1-v1.md".into()),
            final_text: "done".into(),
            usage: RunnerUsage {
                model: "claude-opus-4-7".into(),
                input_tokens: 10,
                output_tokens: 20,
                cache_creation: 0,
                cache_read: 5,
            },
        };
        let s = serde_json::to_string(&out).unwrap();
        let back: RunnerOutput = serde_json::from_str(&s).unwrap();
        assert_eq!(out, back);
    }

    #[test]
    fn rate_limited_is_distinguishable() {
        assert!(RunnerError::RateLimited("429".into()).is_rate_limited());
        assert!(!RunnerError::Spawn("no binary".into()).is_rate_limited());
        assert!(!RunnerError::NoResult.is_rate_limited());
    }

    #[test]
    fn classify_maps_model_unavailable_distinctly() {
        // API not_found_error referencing the model
        let e = RunnerError::classify(
            "not_found_error model: claude-x not found",
            "model: claude-x not found".into(),
        );
        assert!(e.is_model_unavailable(), "got {e:?}");
        assert!(!e.is_rate_limited());
        // CLI-style "unknown model"
        assert!(RunnerError::classify("error: unknown model 'claude-x'", "x".into()).is_model_unavailable());
        // a 404 mentioning the model
        assert!(RunnerError::classify("model claude-x 404", "x".into()).is_model_unavailable());
    }

    #[test]
    fn classify_keeps_rate_limit_and_other_separate() {
        assert!(RunnerError::classify("429 rate limit", "x".into()).is_rate_limited());
        // a generic validation error mentioning "model" but not a not-found signal
        let e = RunnerError::classify("invalid_request_error bad model", "bad model".into());
        assert!(matches!(e, RunnerError::Other(_)), "got {e:?}");
        assert!(!e.is_model_unavailable());
    }

    #[test]
    fn model_unavailable_signal_ignores_bare_invalid() {
        // the generic invalid_request_error type must not trip the model signal
        assert!(!is_model_unavailable_signal("invalid_request_error something about a model"));
        // but an explicit "invalid model" does
        assert!(is_model_unavailable_signal("invalid model name"));
    }

    #[test]
    fn runner_usage_default_is_zeroed() {
        let u = RunnerUsage::default();
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.model, "");
    }

    #[tokio::test]
    async fn default_invoke_stream_delegates_to_invoke_with_no_deltas() {
        use crate::fake::FakeRunner;
        let out = RunnerOutput {
            verdict: Verdict::Approve,
            artifact_path: Some("a.md".into()),
            final_text: "x".into(),
            usage: RunnerUsage::default(),
        };
        let fake = FakeRunner::always(out);
        let req = InvocationRequest {
            task_id: "T".into(), team_id: "t".into(), model: "m".into(),
            thinking_budget: 0, system_prompt: String::new(), user_message: String::new(),
            settings_path: String::new(), add_dirs: vec![],
            sandbox_profile: None,
        };
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: LogSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        // FakeRunner gets a streaming override in a later task; even with it, a
        // FakeRunner built without scripted deltas forwards nothing.
        let result = fake.invoke_stream(&req, &sink).await.unwrap();
        assert_eq!(result.verdict, Verdict::Approve);
        assert!(seen.lock().unwrap().is_empty());
    }
}
