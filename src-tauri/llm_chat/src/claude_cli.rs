//! ClaudeChatRunner — the real chat runner. Composes command::build_chat_args +
//! session::SessionMap + stream_json::parse_chat_stream; the only Claude-idiom
//! side effect (spawning the subprocess) is isolated behind a Spawner so the
//! whole runner is unit-tested without a live `claude`. The captured session id
//! is recorded internally and dropped before returning (F3).

use crate::chat::{ChatError, ChatReply, ChatRequest, ChatRunner};
use crate::command::{build_chat_args, CLAUDE_BIN};
use crate::session::SessionMap;
use crate::stream_json::parse_chat_stream;
use async_trait::async_trait;

/// Produces the raw stream-json stdout for a given argv. Async-free + boxed so
/// the real implementation can run the child process; tests inject canned
/// stdout. Returns Err(ChatError) on spawn failure (the one error class the
/// parser can't produce).
pub type SpawnFn = Box<dyn Fn(&[String]) -> Result<String, ChatError> + Send + Sync>;

pub struct ClaudeChatRunner {
    spawn: SpawnFn,
    sessions: SessionMap,
}

impl ClaudeChatRunner {
    /// The production runner: spawns `claude` and captures stdout.
    pub fn new() -> Self {
        Self {
            spawn: Box::new(|args: &[String]| {
                let output = std::process::Command::new(CLAUDE_BIN)
                    .args(args)
                    .output()
                    .map_err(|e| ChatError::Spawn(e.to_string()))?;
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }),
            sessions: SessionMap::new(),
        }
    }

    /// Test/alternate constructor: inject the stdout producer.
    pub fn with_spawner(spawn: SpawnFn) -> Self {
        Self { spawn, sessions: SessionMap::new() }
    }

    /// Spawn once with the given resume option, parse, and on success record the
    /// captured session under `dialogue_id`. The session id is dropped here — it
    /// never reaches the returned ChatReply (F3).
    fn run_once(
        &self,
        req: &ChatRequest,
        resume: Option<&str>,
    ) -> Result<ChatReply, ChatError> {
        let args = build_chat_args(req, resume);
        let stdout = (self.spawn)(&args)?;
        let (reply, session_id) = parse_chat_stream(&stdout, &req.model)?;
        if let Some(sid) = session_id {
            self.sessions.record(&req.dialogue_id, &sid);
        }
        Ok(reply)
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
        let resume = self.sessions.get(&req.dialogue_id);
        match self.run_once(req, resume.as_deref()) {
            Ok(reply) => Ok(reply),
            // Lost-session fallback (D6): a resumed turn that yields no parseable
            // result is treated as a dead session — clear it and retry once
            // fresh. RateLimited / Spawn are NOT lost-session conditions and
            // propagate as-is.
            Err(err) => {
                let was_resumed = resume.is_some();
                let recoverable = matches!(err, ChatError::NoResult | ChatError::Other(_));
                if was_resumed && recoverable {
                    self.sessions.clear(&req.dialogue_id);
                    self.run_once(req, None)
                } else {
                    Err(err)
                }
            }
        }
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
        }
    }

    #[tokio::test]
    async fn first_turn_omits_resume_and_returns_reply_without_session_id() {
        // Record the args each call saw so we can assert --resume presence.
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args| {
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
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args| {
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
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args| {
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
    async fn spawn_failure_propagates_and_is_not_retried() {
        let runner = ClaudeChatRunner::with_spawner(Box::new(|_| {
            Err(ChatError::Spawn("no binary".into()))
        }));
        let err = runner.chat(&req("x")).await.unwrap_err();
        assert!(matches!(err, ChatError::Spawn(_)));
    }

    #[tokio::test]
    async fn rate_limit_propagates_and_is_not_retried() {
        let runner = ClaudeChatRunner::with_spawner(Box::new(|_| {
            Ok(r#"{"type":"error","error":{"message":"429 rate limit"}}"#.to_string())
        }));
        let err = runner.chat(&req("x")).await.unwrap_err();
        assert!(err.is_rate_limited());
    }
}
