//! ClaudeCliRunner — the v1 runner. Composes command::build_args +
//! stream_json::parse_stream; the only Claude-idiom side effect (spawning the
//! subprocess) is isolated behind a Spawner so the whole runner is unit-tested
//! without a live `claude` binary. Real subprocess spawning is the default
//! Spawner; tests inject a canned-stdout closure.

use crate::command::{build_args, CLAUDE_BIN};
use crate::output::{InvocationRequest, LogSink, Runner, RunnerError, RunnerOutput};
use crate::stream_json::parse_stream_streaming;
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

    /// Spawn once and parse, forwarding any assistant prose fragments to
    /// `forward`. A no-op `forward` means no deltas (identical result to the old
    /// whole-buffer parse — proven by the stream_json tests). Both `invoke` (no
    /// forwarder) and `invoke_stream` (sink forwarder) route through here, so the
    /// streaming and non-streaming paths can never drift on the final output.
    fn run_once(
        &self,
        req: &InvocationRequest,
        forward: &mut dyn FnMut(&str),
    ) -> Result<RunnerOutput, RunnerError> {
        let args = build_args(req);
        let stdout = (self.spawn)(&args)?;
        parse_stream_streaming(&stdout, &req.model, forward)
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
        self.run_once(req, &mut |_d: &str| {})
    }

    async fn invoke_stream(
        &self,
        req: &InvocationRequest,
        sink: &LogSink,
    ) -> Result<RunnerOutput, RunnerError> {
        let mut forward = |d: &str| sink(d);
        self.run_once(req, &mut forward)
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

    #[tokio::test]
    async fn invoke_stream_forwards_prose_deltas_and_returns_same_output() {
        use crate::output::LogSink;
        use std::sync::{Arc, Mutex};
        // The assistant prose carries the verdict (as in the real stream-json
        // sample); the result line is authoritative but is NOT streamed as a delta.
        // The assistant prose carries the verdict on its own line (as in the real
        // stream-json sample); the result line is authoritative but is NOT streamed.
        let canned = r#"{"type":"system","model":"claude-opus-4-7"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Analysing.\nVERDICT: approve"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Analysing.\nVERDICT: approve","usage":{"input_tokens":5,"output_tokens":7}}"#;
        let runner = ClaudeCliRunner::with_spawner(Box::new(move |_args| Ok(canned.to_string())));
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        let sink: LogSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let out = runner.invoke_stream(&req(), &sink).await.unwrap();
        // final output identical to the non-streaming path
        assert_eq!(out.verdict, Verdict::Approve);
        // the assistant prose was streamed (result line is not a delta)
        assert_eq!(*seen.lock().unwrap(), vec!["Analysing.\nVERDICT: approve".to_string()]);
    }

    #[tokio::test]
    async fn invoke_and_invoke_stream_produce_identical_output() {
        let canned = r#"{"type":"result","is_error":false,"result":"VERDICT: approve\nARTIFACT: a.md","usage":{"input_tokens":5,"output_tokens":7}}"#;
        let runner = ClaudeCliRunner::with_spawner(Box::new(move |_args| Ok(canned.to_string())));
        let plain = runner.invoke(&req()).await.unwrap();
        let noop: crate::output::LogSink = Box::new(|_d: &str| {});
        let streamed = runner.invoke_stream(&req(), &noop).await.unwrap();
        assert_eq!(plain, streamed);
    }
}
