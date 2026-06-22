//! ClaudeCliRunner — the v1 runner. Composes command::build_args +
//! stream_json::parse_stream; the only Claude-idiom side effect (spawning the
//! subprocess) is isolated behind a Spawner so the whole runner is unit-tested
//! without a live `claude` binary. Real subprocess spawning is the default
//! Spawner; tests inject a canned-stdout closure.

use crate::command::{build_args, CLAUDE_BIN};
use crate::output::{InvocationRequest, Runner, RunnerError, RunnerOutput};
use crate::stream_json::parse_stream;
use async_trait::async_trait;

/// Produces the raw stream-json stdout for a given argv. Async + boxed so the
/// real implementation can await the child process. Returns Err(RunnerError)
/// on spawn failure (the one error class the parser can't produce).
pub type SpawnFn =
    Box<dyn Fn(&[String]) -> Result<String, RunnerError> + Send + Sync>;

pub struct ClaudeCliRunner {
    spawn: SpawnFn,
}

impl ClaudeCliRunner {
    /// The production runner: spawns `claude` and captures stdout.
    pub fn new() -> Self {
        Self {
            spawn: Box::new(|args: &[String]| {
                let output = std::process::Command::new(CLAUDE_BIN)
                    .args(args)
                    .output()
                    .map_err(|e| RunnerError::Spawn(e.to_string()))?;
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }),
        }
    }

    /// Test/alternate constructor: inject the stdout producer.
    pub fn with_spawner(spawn: SpawnFn) -> Self {
        Self { spawn }
    }
}

impl Default for ClaudeCliRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Runner for ClaudeCliRunner {
    async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
        let args = build_args(req);
        let stdout = (self.spawn)(&args)?;
        parse_stream(&stdout, &req.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::Verdict;

    fn req() -> InvocationRequest {
        InvocationRequest {
            task_id: "T-1".into(),
            team_id: "research".into(),
            model: "claude-opus-4-7".into(),
            thinking_budget: 8192,
            system_prompt: "p".into(),
            user_message: "go".into(),
            settings_path: "/tmp/s.json".into(),
            add_dirs: vec![],
        }
    }

    #[tokio::test]
    async fn invoke_parses_canned_stdout_without_a_real_binary() {
        let canned = r#"{"type":"result","is_error":false,"result":"VERDICT: approve\nARTIFACT: a.md","usage":{"input_tokens":5,"output_tokens":7}}"#;
        let runner = ClaudeCliRunner::with_spawner(Box::new(move |args| {
            // confirm the args were built (spec command line) before "spawning"
            assert!(args.contains(&"--print".to_string()));
            Ok(canned.to_string())
        }));
        let out = runner.invoke(&req()).await.unwrap();
        assert_eq!(out.verdict, Verdict::Approve);
        assert_eq!(out.artifact_path.as_deref(), Some("a.md"));
        assert_eq!(out.usage.input_tokens, 5);
    }

    #[tokio::test]
    async fn invoke_propagates_spawn_failure() {
        let runner = ClaudeCliRunner::with_spawner(Box::new(|_| {
            Err(RunnerError::Spawn("no binary".into()))
        }));
        let err = runner.invoke(&req()).await.unwrap_err();
        assert!(matches!(err, RunnerError::Spawn(_)));
    }

    #[tokio::test]
    async fn invoke_propagates_rate_limit_from_stream() {
        let runner = ClaudeCliRunner::with_spawner(Box::new(|_| {
            Ok(r#"{"type":"error","error":{"message":"429 rate limit"}}"#.to_string())
        }));
        let err = runner.invoke(&req()).await.unwrap_err();
        assert!(err.is_rate_limited());
    }
}
