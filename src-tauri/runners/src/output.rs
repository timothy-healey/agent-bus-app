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
}

/// The ACL seam. Runtime depends only on this trait; the concrete runner kind
/// is selected once at the composition root. Object-safe so it can be held as
/// `Arc<dyn Runner>`.
#[async_trait]
pub trait Runner: Send + Sync {
    async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError>;
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
    fn runner_usage_default_is_zeroed() {
        let u = RunnerUsage::default();
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.model, "");
    }
}
