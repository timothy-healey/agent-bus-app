//! AnthropicApiRunner — the direct-API runner kind. Worker invocations are
//! unsupported: a worker's result must be Structured output, which only the
//! `claude-cli` runner provides (`--json-schema`). The kind is hidden from the
//! UI; it stays constructible so a pipeline naming it fails with a clear runner
//! error instead of failing to load.

use crate::output::{InvocationRequest, Runner, RunnerError, RunnerOutput};
use async_trait::async_trait;

pub struct AnthropicApiRunner;

impl AnthropicApiRunner {
    /// The key is resolved at the composition root; it is not used while worker
    /// invocations are unsupported.
    pub fn new(_api_key: String) -> Self {
        Self
    }
}

#[async_trait]
impl Runner for AnthropicApiRunner {
    async fn invoke(&self, _req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
        Err(RunnerError::Other(
            "unsupported: the anthropic-api runner cannot return Structured output; use the claude-cli runner"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{Effort, OutputKind};

    fn req() -> InvocationRequest {
        InvocationRequest {
            task_id: "T-1".into(),
            team_id: "research".into(),
            model: "claude-opus-4-7".into(),
            effort: Effort::Level("high".into()),
            system_prompt: "You are research.".into(),
            user_message: "Investigate topic X".into(),
            settings_path: "/tmp/s.json".into(),
            add_dirs: vec![],
            sandbox_profile: None,
            working_dir: None,
            output_kind: OutputKind::Producer,
        }
    }

    #[tokio::test]
    async fn a_worker_invocation_is_unsupported() {
        let err = AnthropicApiRunner::new("sk-test".into()).invoke(&req()).await.unwrap_err();
        match err {
            RunnerError::Other(m) => assert!(m.starts_with("unsupported"), "{m}"),
            other => panic!("expected an unsupported error, got {other:?}"),
        }
    }
}
