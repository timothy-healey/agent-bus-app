//! ClaudeChatRunner — the real chat runner. Composes command::build_chat_args +
//! session::SessionMap + stream_json::parse_chat_stream; the only Claude-idiom
//! side effect (spawning the subprocess) is isolated behind a Spawner so the
//! whole runner is unit-tested without a live `claude`. The captured session id
//! is recorded internally and dropped before returning (F3).

use crate::chat::{ChatError, ChatReply, ChatRequest, ChatRunner, DeltaSink};
use crate::command::{build_chat_args, CLAUDE_BIN};
use crate::session::SessionMap;
use crate::stream_json::parse_chat_stream_streaming;
use async_trait::async_trait;

/// Produces the raw stream-json stdout for a given argv. Async-free + boxed so
/// the real implementation can run the child process; tests inject canned
/// stdout. Returns Err(ChatError) on spawn failure (the one error class the
/// parser can't produce).
pub type SpawnFn = Box<dyn Fn(&[String], Option<&str>) -> Result<String, ChatError> + Send + Sync>;

/// Map a finished subprocess's (stdout, stderr, success) into the spawn result.
/// Pure so it is unit-tested without a live `claude`. On success the stdout is
/// returned verbatim for the parser. On a non-zero exit the stderr is inspected:
/// rate-limit-looking stderr maps to `RateLimited`, anything else to `Other`
/// (NOT `Spawn` — `Spawn` is reserved for the `.output()` io-error of a missing
/// binary). `Other` keeps the D6 lost-session retry alive: `chat_with_retry`
/// treats `NoResult | Other` as the recoverable set, so a lost `--resume`
/// session that now errors non-zero still triggers the fresh retry, while a
/// real first-turn error surfaces the actual stderr to the user.
pub fn interpret_chat_output(
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    success: bool,
) -> Result<String, ChatError> {
    if success {
        return Ok(String::from_utf8_lossy(&stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&stderr).into_owned();
    let lower = err.to_lowercase();
    if lower.contains("rate") || lower.contains("429") || lower.contains("quota") {
        Err(ChatError::RateLimited(err))
    } else if err.trim().is_empty() {
        Err(ChatError::Other(
            "claude exited non-zero with no stderr".into(),
        ))
    } else {
        Err(ChatError::Other(err))
    }
}

pub struct ClaudeChatRunner {
    spawn: SpawnFn,
    sessions: SessionMap,
}

impl ClaudeChatRunner {
    /// The production runner: spawns `claude` and captures stdout.
    pub fn new() -> Self {
        Self {
            spawn: Box::new(|args: &[String], cwd: Option<&str>| {
                let mut cmd = std::process::Command::new(CLAUDE_BIN);
                cmd.args(args);
                if let Some(dir) = cwd {
                    cmd.current_dir(dir);
                }
                let output = cmd
                    .output()
                    .map_err(|e| ChatError::Spawn(e.to_string()))?;
                interpret_chat_output(
                    output.stdout,
                    output.stderr,
                    output.status.success(),
                )
            }),
            sessions: SessionMap::new(),
        }
    }

    /// Test/alternate constructor: inject the stdout producer.
    pub fn with_spawner(spawn: SpawnFn) -> Self {
        Self { spawn, sessions: SessionMap::new() }
    }

    /// Spawn once with the given resume option, parse (forwarding any assistant
    /// prose fragments to `forward`), and on success record the captured session
    /// under `dialogue_id`. The session id is dropped here — it never reaches the
    /// returned ChatReply (F3). A no-op `forward` means no deltas (identical
    /// result to the old whole-buffer parse — proven by stream_json tests).
    fn run_once(
        &self,
        req: &ChatRequest,
        resume: Option<&str>,
        forward: &mut dyn FnMut(&str),
    ) -> Result<ChatReply, ChatError> {
        let args = build_chat_args(req, resume);
        let stdout = (self.spawn)(&args, req.working_dir.as_deref())?;
        let (reply, session_id) = parse_chat_stream_streaming(&stdout, &req.model, forward)?;
        if let Some(sid) = session_id {
            self.sessions.record(&req.dialogue_id, &sid);
        }
        Ok(reply)
    }

    /// The D6 lost-session fallback, defined ONCE (vet F2): a resumed turn that
    /// yields no parseable result is treated as a dead session — clear it and
    /// retry once fresh. RateLimited / Spawn are NOT lost-session conditions and
    /// propagate as-is. Both `chat` (no forwarder) and `chat_stream` (sink
    /// forwarder) route through here, so the streaming and non-streaming paths
    /// can never drift on session handling.
    fn chat_with_retry(
        &self,
        req: &ChatRequest,
        forward: &mut dyn FnMut(&str),
    ) -> Result<ChatReply, ChatError> {
        let resume = self.sessions.get(&req.dialogue_id);
        match self.run_once(req, resume.as_deref(), forward) {
            Ok(reply) => Ok(reply),
            Err(err) => {
                let was_resumed = resume.is_some();
                let recoverable = matches!(err, ChatError::NoResult | ChatError::Other(_));
                if was_resumed && recoverable {
                    self.sessions.clear(&req.dialogue_id);
                    self.run_once(req, None, forward)
                } else {
                    Err(err)
                }
            }
        }
    }
}

impl Default for ClaudeChatRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChatRunner for ClaudeChatRunner {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError> {
        // No delta forwarding for the plain path.
        self.chat_with_retry(req, &mut |_d: &str| {})
    }

    async fn chat_stream(&self, req: &ChatRequest, sink: &DeltaSink) -> Result<ChatReply, ChatError> {
        let mut forward = |d: &str| sink(d);
        self.chat_with_retry(req, &mut forward)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatError, ChatRunner, ChatRequest};
    use std::sync::{Arc, Mutex};

    const FIRST: &str = include_str!("fixtures/chat-first-turn.txt");
    const FOLLOW: &str = include_str!("fixtures/chat-follow-up.txt");

    fn req(msg: &str) -> ChatRequest {
        ChatRequest {
            dialogue_id: "proj-1".into(),
            system_prompt: "sys".into(),
            user_message: msg.into(),
            model: "claude-opus-4-8".into(),
            thinking_budget: 8192,
            working_dir: None,
        }
    }

    #[tokio::test]
    async fn chat_spawner_receives_the_request_working_dir() {
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let s = seen.clone();
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |_args, cwd| {
            *s.lock().unwrap() = cwd.map(|c| c.to_string());
            Ok(FIRST.to_string())
        }));
        let mut r = req("hi");
        r.working_dir = Some("/tmp/chat-here".into());
        runner.chat(&r).await.unwrap();
        assert_eq!(seen.lock().unwrap().as_deref(), Some("/tmp/chat-here"));
    }

    #[tokio::test]
    async fn first_turn_omits_resume_and_returns_reply_without_session_id() {
        // Record the args each call saw so we can assert --resume presence.
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args, _cwd| {
            s.lock().unwrap().push(args.to_vec());
            Ok(FIRST.to_string())
        }));
        let reply = runner.chat(&req("how is T-042?")).await.unwrap();
        assert_eq!(reply.text, "T-042 is in the design stage; the spec is awaiting review.");
        assert_eq!(reply.usage.input_tokens, 900);
        // ChatReply has no session_id field at all (compile-time F3 guarantee).
        // First call must NOT carry --resume.
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].iter().any(|a| a == "--resume"));
    }

    #[tokio::test]
    async fn second_turn_for_same_dialogue_resumes_the_captured_session() {
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        // First call returns FIRST (session sess-first), second returns FOLLOW.
        let n = Arc::new(Mutex::new(0usize));
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args, _cwd| {
            s.lock().unwrap().push(args.to_vec());
            let mut k = n.lock().unwrap();
            let out = if *k == 0 { FIRST } else { FOLLOW };
            *k += 1;
            Ok(out.to_string())
        }));
        let _ = runner.chat(&req("first")).await.unwrap();
        let _ = runner.chat(&req("second")).await.unwrap();
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(!calls[0].iter().any(|a| a == "--resume"));
        // second call resumes sess-first (captured from the first turn)
        let r = calls[1].iter().position(|a| a == "--resume").unwrap();
        assert_eq!(calls[1][r + 1], "sess-first");
    }

    #[tokio::test]
    async fn lost_session_on_a_resumed_turn_retries_fresh_once() {
        // Seed a known session, then make a --resume call fail with NoResult
        // (empty stdout) but a fresh (no --resume) call succeed.
        let n = Arc::new(Mutex::new(0usize));
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args, _cwd| {
            let mut k = n.lock().unwrap();
            *k += 1;
            // call 1: first turn -> FIRST (captures sess-first)
            // call 2: resumed turn -> empty (lost session -> NoResult)
            // call 3: fresh retry -> FOLLOW
            let has_resume = args.iter().any(|a| a == "--resume");
            if *k == 1 {
                Ok(FIRST.to_string())
            } else if has_resume {
                Ok(String::new()) // empty stdout => NoResult
            } else {
                Ok(FOLLOW.to_string())
            }
        }));
        let _ = runner.chat(&req("first")).await.unwrap(); // establishes sess-first
        let reply = runner.chat(&req("second")).await.unwrap(); // resume fails, retries fresh
        assert_eq!(reply.text, "Yes — I injected the topic; it is now task T-043.");
    }

    #[tokio::test]
    async fn chat_stream_forwards_prose_deltas_and_returns_final_reply() {
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |_args, _cwd| Ok(FIRST.to_string())));
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        let sink: crate::chat::DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let reply = runner.chat_stream(&req("how is T-042?"), &sink).await.unwrap();
        assert_eq!(reply.text, "T-042 is in the design stage; the spec is awaiting review.");
        assert_eq!(reply.usage.input_tokens, 900);
        assert!(!seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn chat_stream_records_session_like_chat() {
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        let n = Arc::new(Mutex::new(0usize));
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args, _cwd| {
            s.lock().unwrap().push(args.to_vec());
            let mut k = n.lock().unwrap();
            let out = if *k == 0 { FIRST } else { FOLLOW };
            *k += 1;
            Ok(out.to_string())
        }));
        let noop: crate::chat::DeltaSink = Box::new(|_d: &str| {});
        let _ = runner.chat_stream(&req("first"), &noop).await.unwrap();
        let _ = runner.chat_stream(&req("second"), &noop).await.unwrap();
        let calls = seen.lock().unwrap();
        let r = calls[1].iter().position(|a| a == "--resume").unwrap();
        assert_eq!(calls[1][r + 1], "sess-first");
    }

    #[tokio::test]
    async fn spawn_failure_propagates_and_is_not_retried() {
        let runner = ClaudeChatRunner::with_spawner(Box::new(|_, _cwd| {
            Err(ChatError::Spawn("no binary".into()))
        }));
        let err = runner.chat(&req("x")).await.unwrap_err();
        assert!(matches!(err, ChatError::Spawn(_)));
    }

    #[tokio::test]
    async fn rate_limit_propagates_and_is_not_retried() {
        let runner = ClaudeChatRunner::with_spawner(Box::new(|_, _cwd| {
            Ok(r#"{"type":"error","error":{"message":"429 rate limit"}}"#.to_string())
        }));
        let err = runner.chat(&req("x")).await.unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[test]
    fn interpret_chat_output_success_returns_stdout() {
        let out = interpret_chat_output(b"hello stdout".to_vec(), b"ignored".to_vec(), true).unwrap();
        assert_eq!(out, "hello stdout");
    }

    #[test]
    fn interpret_chat_output_rate_limit_stderr_maps_to_rate_limited() {
        let err = interpret_chat_output(
            Vec::new(),
            b"Error: 429 rate limit exceeded".to_vec(),
            false,
        )
        .unwrap_err();
        assert!(matches!(err, ChatError::RateLimited(_)));
        assert!(err.is_rate_limited());
    }

    #[test]
    fn interpret_chat_output_other_stderr_maps_to_other_with_text() {
        let err = interpret_chat_output(
            Vec::new(),
            b"Error: When using --print, --output-format=stream-json requires --verbose".to_vec(),
            false,
        )
        .unwrap_err();
        match err {
            ChatError::Other(m) => assert!(m.contains("requires --verbose")),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn interpret_chat_output_empty_stderr_uses_fallback_message() {
        let err = interpret_chat_output(Vec::new(), Vec::new(), false).unwrap_err();
        match err {
            ChatError::Other(m) => assert_eq!(m, "claude exited non-zero with no stderr"),
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
