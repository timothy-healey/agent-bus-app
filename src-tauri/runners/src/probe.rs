//! Model availability probe (G6). A `test_model` OHS command fires a 1-token
//! invocation against a model and reports whether it is usable at authoring
//! time. The LIVE call is structural-only here (no headless `claude` in tests);
//! the classification of the result into the `ModelProbe` DTO is pure and
//! unit-tested, and the probe is run through the SAME `Runner` seam (so a
//! `FakeRunner` drives the tests). The model-unavailable class is the one G6
//! cares about — it surfaces as "model unavailable — pick another".

use crate::output::{InvocationRequest, Runner, RunnerError};
use serde::{Deserialize, Serialize};

/// Status of a model probe crossing the OHS. Mirrors the TS `ModelTestResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    /// The 1-token probe succeeded — the model is usable.
    Ok,
    /// The runner classified a model-not-found / unavailable error (G6).
    Unavailable,
    /// Any other failure (rate-limit, spawn, transport, no result).
    Error,
}

/// The probe result DTO. Only this crosses the OHS — no RunnerError/idiom leaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProbe {
    pub status: ProbeStatus,
    pub message: String,
}

/// Build the 1-token probe request for `model`. Minimal system/user prompt; an
/// empty settings path (no scope needed to prove the model resolves). Pure.
pub fn probe_request(model: &str) -> InvocationRequest {
    InvocationRequest {
        task_id: "probe".into(),
        team_id: "probe".into(),
        model: model.to_string(),
        thinking_budget: 0,
        system_prompt: "Reply with the single token: ok".into(),
        user_message: "ok".into(),
        settings_path: String::new(),
        add_dirs: vec![],
        sandbox_profile: None,
    }
}

/// Classify a probe invocation's outcome into the `ModelProbe` DTO (pure). A
/// success (any RunnerOutput) ⇒ Ok; a `ModelUnavailable` ⇒ Unavailable with the
/// "pick another" framing; everything else ⇒ Error carrying the message.
pub fn classify_probe(result: Result<(), RunnerError>) -> ModelProbe {
    match result {
        Ok(()) => ModelProbe { status: ProbeStatus::Ok, message: "model available".into() },
        Err(RunnerError::ModelUnavailable(m)) => ModelProbe {
            status: ProbeStatus::Unavailable,
            message: format!("model unavailable — pick another ({m})"),
        },
        Err(e) => ModelProbe { status: ProbeStatus::Error, message: e.to_string() },
    }
}

/// Run the probe through the injected `Runner` seam and classify it. The runner
/// is real (`ClaudeCliRunner`) at the composition root and a `FakeRunner` in
/// tests — the live subprocess path is therefore exercised structurally only.
pub async fn run_probe(runner: &dyn Runner, model: &str) -> ModelProbe {
    let req = probe_request(model);
    classify_probe(runner.invoke(&req).await.map(|_| ()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeRunner;
    use crate::output::{RunnerOutput, RunnerUsage};
    use agent_bus_core::Verdict;
    use std::collections::BTreeSet;

    fn ok_output() -> RunnerOutput {
        RunnerOutput {
            verdict: Verdict::Approve,
            artifact_path: None,
            final_text: "ok".into(),
            usage: RunnerUsage::default(),
        }
    }

    #[test]
    fn classify_maps_each_outcome() {
        assert_eq!(classify_probe(Ok(())).status, ProbeStatus::Ok);
        let unavail = classify_probe(Err(RunnerError::ModelUnavailable("claude-x not found".into())));
        assert_eq!(unavail.status, ProbeStatus::Unavailable);
        assert!(unavail.message.contains("pick another"));
        let other = classify_probe(Err(RunnerError::RateLimited("429".into())));
        assert_eq!(other.status, ProbeStatus::Error);
    }

    #[test]
    fn probe_request_is_one_token_shaped() {
        let req = probe_request("claude-opus-4-8");
        assert_eq!(req.model, "claude-opus-4-8");
        assert_eq!(req.thinking_budget, 0);
        assert!(!req.user_message.is_empty());
    }

    #[tokio::test]
    async fn run_probe_reports_ok_on_a_successful_invocation() {
        let fake = FakeRunner::always(ok_output());
        let probe = run_probe(&fake, "claude-opus-4-8").await;
        assert_eq!(probe.status, ProbeStatus::Ok);
    }

    #[tokio::test]
    async fn run_probe_reports_unavailable_on_model_error() {
        let fake = FakeRunner::new(vec![Err(RunnerError::ModelUnavailable("unknown model".into()))]);
        let probe = run_probe(&fake, "claude-nope").await;
        assert_eq!(probe.status, ProbeStatus::Unavailable);
    }

    #[test]
    fn model_probe_wire_contract_matches_ts() {
        let p = ModelProbe { status: ProbeStatus::Ok, message: "model available".into() };
        let v = serde_json::to_value(&p).unwrap();
        let keys: BTreeSet<String> = v.as_object().unwrap().keys().cloned().collect();
        let want: BTreeSet<String> = ["status", "message"].iter().map(|s| s.to_string()).collect();
        assert_eq!(keys, want);
        // status serialises as a snake_case string matching the TS union
        assert_eq!(v["status"], "ok");
        assert_eq!(
            serde_json::to_value(ProbeStatus::Unavailable).unwrap(),
            serde_json::Value::String("unavailable".into())
        );
    }
}
