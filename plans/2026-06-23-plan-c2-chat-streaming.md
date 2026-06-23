# C2 — Token Streaming for Chat Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stream the assistant's prose to the terminal as it arrives (display-only), while the agentic loop's tool-call extraction and turn recording still operate on the COMPLETE reply of each step.

**Architecture:** Add a streaming capability to the `ChatRunner` ACL: a new `chat_stream(req, sink)` method that incrementally parses Claude's line-delimited stream-json, pushes assistant *text deltas* to a `DeltaSink` callback as they parse, and still returns the same final `ChatReply`. The default `chat()` is kept (re-expressed as `chat_stream` with a no-op sink) so the non-streaming behaviour is unchanged. At the composition root, `AgenticChatEngine` and `LlmEngine` call `chat_stream` with a sink that emits throttled `conversation.delta` Tauri events; the frontend terminal appends those deltas to a live "streaming" assistant bubble until the authoritative `Conversation` arrives from `send_message`. Stream-json idioms stay sealed inside `llm_chat`; the sink sees only `&str` text fragments.

**Tech Stack:** Rust (async_trait, serde_json, tokio), Tauri events (`Emitter`), TypeScript/React, vitest.

---

## File Structure

**Backend (`src-tauri/llm_chat/`):**
- `chat.rs` — add `DeltaSink` type alias + `chat_stream` default method on the `ChatRunner` trait (default impl delegates to `chat`, so existing impls compile; concrete impls override). Modify.
- `stream_json.rs` — refactor `ChatAccumulator::feed` to return the *new text delta* produced by an `assistant` event so a streaming caller can forward it; add an incremental `StreamParser` that takes lines one at a time and a `parse_chat_stream_streaming` helper that drives a sink. Keep `parse_chat_stream` working (whole-buffer) by routing it through the same accumulator. Modify.
- `claude_cli.rs` — implement `chat_stream` on `ClaudeChatRunner`. Because the production `SpawnFn` returns the whole stdout `String` (no live `claude` here), the streaming path parses that buffered stdout incrementally and forwards each `assistant` text delta to the sink, then returns the final reply. This exercises the incremental parser + delta forwarding via fixtures; the live line-by-line subprocess pipe is a future refinement (noted). Modify.
- `fake.rs` — add a `scripted-deltas` mode: `FakeChatRunner::with_deltas(Vec<Vec<String>>)` seeds, per call, the ordered text deltas to emit before returning the matching reply. `chat_stream` forwards those deltas to the sink. Modify.

**Backend (`src-tauri/app/`):**
- `lib.rs` — `AgenticChatEngine` + `LlmEngine` gain an optional delta sink (a boxed `Fn(&str)`), built at the root to emit throttled `conversation.delta` events via the `AppHandle`. Each engine calls `chat_stream` with that sink. Add a throttle helper. Modify.

**Frontend (`src/`):**
- `ipc/terminal.ts` — add a `ConversationDelta` type + `onConversationDelta(cb)` listener wrapper. Modify.
- `hooks/useConversation.ts` — subscribe to `conversation.delta`; accumulate into a `streaming` string shown as a transient assistant bubble; clear it when `send` resolves with the authoritative turns. Modify.
- `components/Terminal.tsx` — render the transient streaming bubble (when present) after the persisted turns. Modify.

---

## Decisions

- **AUTO-DECIDE — Display-only streaming.** Deltas are for *feel*. Tool-call extraction (`extract_tool_call_block`), parsing, dispatch, the step loop, and the recorded composite `Turn` all still run on the COMPLETE `ChatReply.text` of each step. We never parse partial tool-call JSON. (Operator design constraint.)
- **AUTO-DECIDE — Sink is a `&str` callback, not a channel.** A `DeltaSink = Box<dyn Fn(&str) + Send + Sync>` is the clean seam: the ACL forwards plain prose fragments; the root maps them to Tauri events. No stream-json type, no `session_id`, no event name crosses the ACL. (Simplest seam that satisfies F3.)
- **AUTO-DECIDE — `chat_stream` is additive with a default impl.** Adding a defaulted trait method keeps every existing `ChatRunner` impl and test compiling; `chat()` stays the canonical non-streaming entry. Concrete runners override `chat_stream`; the engines call `chat_stream`. The non-streaming path is preserved exactly.
- **AUTO-DECIDE — Stream only the agentic loop's *final-ish* prose AND every step's prose.** The sink fires on every `assistant` text delta of every step. Each step resets the live buffer via an **explicit `reset()` signal** (vet F1 — the prose `DeltaSink` stays prose-only; the reset is a separate closure on `ConversationDeltaEmitter`), emitting a `conversation.delta` with `{ reset: true }` boundary at the start of each model call, so intermediate tool-thinking prose doesn't pile onto the final answer visually. The authoritative `Turn` (final prose only) replaces the transient bubble on completion.
- **AUTO-DECIDE — Event name `conversation.delta`.** Matches the operator's recommended name and the existing `task.changed` / `usage.changed` dotted convention. Payload `{ text: string, reset: boolean }`.
- **AUTO-DECIDE — Throttle at the root, not the ACL.** The ACL forwards every delta synchronously; the root coalesces with a small time/size throttle (flush every ~50ms or ~80 chars) before emitting, so the ACL stays idiom-free and the throttling policy is a UI concern owned by the root.
- **AUTO-DECIDE — Buffered-stdout incremental parse.** With no live `claude`, the real `SpawnFn` returns whole stdout. `ClaudeChatRunner::chat_stream` feeds that buffer to the incremental parser line-by-line, forwarding deltas. The live subprocess streaming pipe is structurally covered (same incremental parser) but not end-to-end; noted as a caveat.

---

## Task 1: stream_json — accumulator yields per-event text delta

**Files:**
- Modify: `src-tauri/llm_chat/src/stream_json.rs`
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `stream_json.rs`:

```rust
#[test]
fn feed_returns_text_delta_for_assistant_event_only() {
    let mut acc = ChatAccumulator::default();
    let sys: Value = serde_json::from_str(
        r#"{"type":"system","subtype":"init","session_id":"s","model":"m"}"#,
    ).unwrap();
    let asst: Value = serde_json::from_str(
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello "}],"usage":{"output_tokens":1}}}"#,
    ).unwrap();
    let asst2: Value = serde_json::from_str(
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"world"}],"usage":{"output_tokens":1}}}"#,
    ).unwrap();
    // system events produce no prose delta
    assert_eq!(acc.feed(&sys).unwrap(), "");
    // each assistant event yields exactly the text it added
    assert_eq!(acc.feed(&asst).unwrap(), "hello ");
    assert_eq!(acc.feed(&asst2).unwrap(), "world");
    // the accumulator still has the full concatenation
    assert_eq!(acc.text, "hello world");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p llm_chat feed_returns_text_delta -- --nocapture`
Expected: FAIL — `feed` currently returns `Result<(), ChatError>`, so `assert_eq!(..., "")` is a type error / no return value.

- [ ] **Step 3: Change `feed` to return the new prose delta**

In `stream_json.rs`, change the `feed` signature and the `assistant` arm to capture and return the text appended by THIS event. Replace the `fn feed` signature line and the `assistant` arm:

```rust
    /// Feed one parsed JSON line. Returns the prose text *this* event added
    /// (empty for non-prose events) so a streaming caller can forward it; the
    /// accumulator keeps the running full text for the final ChatReply.
    fn feed(&mut self, v: &Value) -> Result<String, ChatError> {
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let mut delta = String::new();
```

Then in the `"assistant"` arm, accumulate into `delta` first, then push to `self.text`:

```rust
            "assistant" => {
                if let Some(content) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                delta.push_str(t);
                            }
                        }
                    }
                }
                self.text.push_str(&delta);
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.add_usage(u);
                }
            }
```

The `"result"` arm must NOT return a delta (the result prose replaces `self.text` and is the authoritative final, not an incremental delta — display already streamed it). Leave the `result` arm body unchanged but ensure it falls through to `Ok(delta)` with `delta` still empty.

Change the final line of `feed` from `Ok(())` to:

```rust
        Ok(delta)
```

- [ ] **Step 4: Update the existing whole-buffer `parse_chat_stream` to ignore the delta**

In `parse_chat_stream`, the loop currently does `acc.feed(&v)?;`. Change to discard the returned delta:

```rust
        let _ = acc.feed(&v)?;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p llm_chat stream_json`
Expected: PASS (the new test + all existing stream_json tests, including `parses_first_turn_text_usage_and_session_id`, still green).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/llm_chat/src/stream_json.rs
git commit -m "feat(c2): stream_json accumulator returns per-event prose delta"
```

---

## Task 2: stream_json — incremental streaming parse helper

**Files:**
- Modify: `src-tauri/llm_chat/src/stream_json.rs`
- Test: same file

- [ ] **Step 1: Write the failing test**

Add to `mod tests`:

```rust
#[test]
fn parse_streaming_forwards_assistant_deltas_in_order_and_returns_final() {
    // Two assistant prose events then a result line.
    let raw = concat!(
        r#"{"type":"system","subtype":"init","session_id":"s","model":"claude-opus-4-8"}"#, "\n",
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hel"}],"usage":{"output_tokens":1}}}"#, "\n",
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"lo"}],"usage":{"output_tokens":1}}}"#, "\n",
        r#"{"type":"result","subtype":"success","is_error":false,"result":"Hello","usage":{"input_tokens":5,"output_tokens":2},"session_id":"s"}"#
    );
    let seen = std::sync::Mutex::new(Vec::<String>::new());
    let (reply, session) = parse_chat_stream_streaming(raw, "m", &mut |d: &str| {
        seen.lock().unwrap().push(d.to_string());
    }).unwrap();
    // deltas forwarded in arrival order, prose only
    assert_eq!(*seen.lock().unwrap(), vec!["Hel".to_string(), "lo".to_string()]);
    // final reply matches the whole-buffer parse (result prose authoritative)
    assert_eq!(reply.text, "Hello");
    assert_eq!(reply.usage.input_tokens, 5);
    assert_eq!(session.as_deref(), Some("s"));
}

#[test]
fn parse_streaming_matches_non_streaming_for_fixtures() {
    let on = |_: &str| {};
    let mut on = on;
    let (a, sa) = parse_chat_stream(FIRST, "m").unwrap();
    let (b, sb) = parse_chat_stream_streaming(FIRST, "m", &mut on).unwrap();
    assert_eq!(a, b);
    assert_eq!(sa, sb);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p llm_chat parse_streaming`
Expected: FAIL — `parse_chat_stream_streaming` does not exist.

- [ ] **Step 3: Add the streaming parse helper**

In `stream_json.rs`, after `parse_chat_stream`, add:

```rust
/// Streaming variant of `parse_chat_stream`. Parses the same newline-delimited
/// JSON but invokes `on_delta` with each assistant *prose fragment* as it is
/// parsed (display-only feel), then returns the identical final ChatReply +
/// session id. The result line's authoritative prose is NOT forwarded as a
/// delta — it has already been streamed via the assistant events.
pub fn parse_chat_stream_streaming(
    raw: &str,
    model: &str,
    on_delta: &mut dyn FnMut(&str),
) -> Result<(ChatReply, Option<String>), ChatError> {
    let mut acc = ChatAccumulator::default();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| ChatError::Other(format!("bad stream-json line: {e}")))?;
        let delta = acc.feed(&v)?;
        if !delta.is_empty() {
            on_delta(&delta);
        }
    }
    acc.finish(model)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p llm_chat stream_json`
Expected: PASS (both new tests + existing).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/llm_chat/src/stream_json.rs
git commit -m "feat(c2): incremental parse_chat_stream_streaming forwards prose deltas"
```

---

## Task 3: ChatRunner trait — DeltaSink + chat_stream

**Files:**
- Modify: `src-tauri/llm_chat/src/chat.rs`
- Test: same file

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `chat.rs`:

```rust
#[tokio::test]
async fn default_chat_stream_delegates_to_chat_with_no_deltas() {
    use crate::fake::FakeChatRunner;
    let fake = FakeChatRunner::new(vec![ChatReply { text: "hi".into(), usage: ChatUsage::default() }]);
    let seen = std::sync::Mutex::new(Vec::<String>::new());
    let req = ChatRequest {
        dialogue_id: "d".into(), system_prompt: "s".into(), user_message: "u".into(),
        model: "m".into(), thinking_budget: 0,
    };
    let sink: DeltaSink = Box::new(move |d: &str| { /* default fake forwards none */ let _ = d; });
    let reply = fake.chat_stream(&req, &sink).await.unwrap();
    assert_eq!(reply.text, "hi");
    let _ = seen; // FakeChatRunner::new emits no deltas; covered by Task 5
}
```

Note: `FakeChatRunner::new` will gain `chat_stream` via the default trait method in Step 3, which calls `chat` (no deltas). The scripted-deltas mode is Task 5.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p llm_chat default_chat_stream_delegates`
Expected: FAIL — `DeltaSink` and `chat_stream` do not exist.

- [ ] **Step 3: Add `DeltaSink` + defaulted `chat_stream`**

In `chat.rs`, after the `ChatReply` definitions and before the `ChatRunner` trait, add:

```rust
/// A display-only prose sink. The streaming chat path forwards each assistant
/// text fragment here as it parses. Deliberately a plain `&str` callback: NO
/// stream-json idiom, session id, or event name crosses the ACL through it —
/// the composition root maps fragments to whatever UI event it likes (F3).
pub type DeltaSink = Box<dyn Fn(&str) + Send + Sync>;
```

Then extend the trait with a defaulted method:

```rust
#[async_trait]
pub trait ChatRunner: Send + Sync {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError>;

    /// Streaming variant: identical contract to `chat` (same final `ChatReply`),
    /// but assistant prose fragments are forwarded to `sink` as they arrive for
    /// live display. The default delegates to `chat` (no deltas), so existing
    /// runners keep working; streaming runners override this.
    async fn chat_stream(&self, req: &ChatRequest, sink: &DeltaSink) -> Result<ChatReply, ChatError> {
        let _ = sink;
        self.chat(req).await
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p llm_chat default_chat_stream_delegates`
Expected: PASS.

- [ ] **Step 5: Run the whole crate to confirm nothing broke**

Run: `cargo test -p llm_chat`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/llm_chat/src/chat.rs
git commit -m "feat(c2): ChatRunner gains DeltaSink + defaulted chat_stream seam"
```

---

## Task 4: ClaudeChatRunner — streaming chat_stream over buffered stdout

**Files:**
- Modify: `src-tauri/llm_chat/src/claude_cli.rs`
- Test: same file

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `claude_cli.rs`:

```rust
#[tokio::test]
async fn chat_stream_forwards_prose_deltas_and_returns_final_reply() {
    let runner = ClaudeChatRunner::with_spawner(Box::new(move |_args| Ok(FIRST.to_string())));
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    let sink: crate::chat::DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
    let reply = runner.chat_stream(&req("how is T-042?"), &sink).await.unwrap();
    // final reply is the authoritative result prose (unchanged from chat())
    assert_eq!(reply.text, "T-042 is in the design stage; the spec is awaiting review.");
    assert_eq!(reply.usage.input_tokens, 900);
    // at least one prose delta was forwarded from the assistant event(s)
    assert!(!seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn chat_stream_records_session_like_chat() {
    // A resumed second turn must still resume sess-first when using chat_stream.
    let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    let n = Arc::new(Mutex::new(0usize));
    let runner = ClaudeChatRunner::with_spawner(Box::new(move |args| {
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p llm_chat chat_stream`
Expected: FAIL — `ClaudeChatRunner` uses the default `chat_stream` (delegates to `chat`), so `chat_stream_forwards_prose_deltas` asserts `!seen.is_empty()` and fails (no deltas emitted).

- [ ] **Step 3: Refactor to ONE parameterised run + retry core (vet F2), then add chat_stream**

(Vet F2 — *Refactor before you add*.) Rather than a parallel `run_once_streaming`
duplicating the D6 lost-session retry, collapse to a single core that both `chat`
and `chat_stream` call with/without a delta forwarder. First update the imports at the top:

```rust
use crate::chat::{ChatError, ChatReply, ChatRequest, ChatRunner, DeltaSink};
use crate::stream_json::parse_chat_stream_streaming;
```

(`parse_chat_stream` is no longer used by this file — the parameterised core
always goes through `parse_chat_stream_streaming` with a None-ish forwarder, which
Task 2 proves yields identical results. Drop the `parse_chat_stream` import here.)

Replace `run_once` with a forwarder-parameterised version, and add a single
`chat_with_retry` helper carrying the D6 policy once:

```rust
    /// Spawn once with the given resume option, parse (forwarding any assistant
    /// prose fragments to `forward`), and on success record the captured session
    /// under `dialogue_id`. The session id is dropped here — it never reaches the
    /// returned ChatReply (F3). A `None` forwarder means no deltas (identical
    /// result to the old whole-buffer parse — proven by stream_json Task 2).
    fn run_once(
        &self,
        req: &ChatRequest,
        resume: Option<&str>,
        forward: &mut dyn FnMut(&str),
    ) -> Result<ChatReply, ChatError> {
        let args = build_chat_args(req, resume);
        let stdout = (self.spawn)(&args)?;
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
```

Now replace the `impl ChatRunner for ClaudeChatRunner` block's two methods so both
delegate to `chat_with_retry`:

```rust
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
```

(The retry on a re-streamed turn re-forwards from the fresh attempt — acceptable:
the frontend resets the live bubble at step boundaries, so a re-stream just
re-paints the current step.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p llm_chat claude_cli`
Expected: PASS (new streaming tests + all existing claude_cli tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/llm_chat/src/claude_cli.rs
git commit -m "feat(c2): ClaudeChatRunner.chat_stream forwards prose deltas, records session"
```

---

## Task 5: FakeChatRunner — scripted-deltas mode

**Files:**
- Modify: `src-tauri/llm_chat/src/fake.rs`
- Test: same file

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `fake.rs`:

```rust
#[tokio::test]
async fn scripted_deltas_are_forwarded_then_reply_returned() {
    let fake = FakeChatRunner::with_deltas(
        vec![ChatReply { text: "Hello world".into(), usage: ChatUsage::default() }],
        vec![vec!["Hello ".into(), "world".into()]],
    );
    let seen = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    let s = seen.clone();
    let sink: crate::chat::DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
    let reply = fake.chat_stream(&req("hi"), &sink).await.unwrap();
    assert_eq!(reply.text, "Hello world");
    assert_eq!(*seen.lock().unwrap(), vec!["Hello ".to_string(), "world".to_string()]);
}

#[tokio::test]
async fn deltas_default_to_empty_for_new_constructor() {
    // FakeChatRunner::new still works via the defaulted chat_stream (no deltas).
    let fake = FakeChatRunner::new(vec![ChatReply { text: "x".into(), usage: ChatUsage::default() }]);
    let seen = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    let s = seen.clone();
    let sink: crate::chat::DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
    let reply = fake.chat_stream(&req("hi"), &sink).await.unwrap();
    assert_eq!(reply.text, "x");
    assert!(seen.lock().unwrap().is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p llm_chat scripted_deltas`
Expected: FAIL — `with_deltas` does not exist.

- [ ] **Step 3: Add scripted-deltas field + constructor + chat_stream override**

In `fake.rs`, add the import and a `deltas` field. Change the struct + constructors:

```rust
use crate::chat::{ChatError, ChatReply, ChatRequest, ChatRunner, DeltaSink};
use async_trait::async_trait;
use std::sync::Mutex;

pub struct FakeChatRunner {
    replies: Vec<ChatReply>,
    /// Per-call scripted prose deltas to forward via chat_stream before returning
    /// that call's reply. Empty (or shorter than `replies`) => no deltas for that
    /// call. Indexed by the same cursor as `replies`.
    deltas: Vec<Vec<String>>,
    error: Option<ChatError>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<ChatRequest>>,
}

impl FakeChatRunner {
    /// Seed with replies returned in order (no streamed deltas).
    pub fn new(replies: Vec<ChatReply>) -> Self {
        Self { replies, deltas: vec![], error: None, cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }

    /// Seed with replies AND, per call, the ordered prose deltas chat_stream
    /// forwards before returning that call's reply.
    pub fn with_deltas(replies: Vec<ChatReply>, deltas: Vec<Vec<String>>) -> Self {
        Self { replies, deltas, error: None, cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }

    /// Seed with an error that every `chat` call returns.
    pub fn failing(error: ChatError) -> Self {
        Self { replies: vec![], deltas: vec![], error: Some(error), cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }
}
```

The existing `chat` impl reads `self.cursor` to pick a reply and increments it. To keep `chat` and `chat_stream` consistent, refactor the reply-picking into a helper that does NOT advance the cursor, and have each entry point advance once. Replace the `impl ChatRunner` block:

```rust
impl FakeChatRunner {
    /// Reply at index `idx` (clamped), or empty when unseeded.
    fn reply_at(&self, idx: usize) -> ChatReply {
        self.replies
            .get(idx.min(self.replies.len().saturating_sub(1)))
            .cloned()
            .unwrap_or(ChatReply { text: String::new(), usage: Default::default() })
    }

    fn clone_error(&self) -> Option<ChatError> {
        self.error.as_ref().map(|err| match err {
            ChatError::RateLimited(m) => ChatError::RateLimited(m.clone()),
            ChatError::Spawn(m) => ChatError::Spawn(m.clone()),
            ChatError::NoResult => ChatError::NoResult,
            ChatError::Other(m) => ChatError::Other(m.clone()),
        })
    }
}

#[async_trait]
impl ChatRunner for FakeChatRunner {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError> {
        self.received.lock().unwrap().push(req.clone());
        if let Some(err) = self.clone_error() {
            return Err(err);
        }
        let mut c = self.cursor.lock().unwrap();
        let idx = *c;
        *c += 1;
        Ok(self.reply_at(idx))
    }

    async fn chat_stream(&self, req: &ChatRequest, sink: &DeltaSink) -> Result<ChatReply, ChatError> {
        self.received.lock().unwrap().push(req.clone());
        if let Some(err) = self.clone_error() {
            return Err(err);
        }
        let idx = {
            let mut c = self.cursor.lock().unwrap();
            let i = *c;
            *c += 1;
            i
        };
        if let Some(call_deltas) = self.deltas.get(idx.min(self.deltas.len().saturating_sub(1))) {
            // only forward when this call actually has a scripted entry
            if idx < self.deltas.len() {
                for d in call_deltas {
                    sink(d);
                }
            }
        }
        Ok(self.reply_at(idx))
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p llm_chat fake`
Expected: PASS (new scripted-delta tests + the existing `returns_seeded_replies_in_order`, `clamps_to_the_last_reply`, `can_be_seeded_to_error`).

- [ ] **Step 5: Run the whole crate**

Run: `cargo test -p llm_chat`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/llm_chat/src/fake.rs
git commit -m "feat(c2): FakeChatRunner scripted-deltas mode for streaming tests"
```

---

## Task 6: app root — engines stream via a delta sink

**Files:**
- Modify: `src-tauri/app/src/lib.rs`
- Test: same file (`#[cfg(test)] mod` for engines)

- [ ] **Step 1: Write the failing test**

In the engine tests module of `lib.rs` (the `mod` around line 1202 that imports `AgenticChatEngine`), add a test that the agentic engine forwards deltas through a sink while behaviour is unchanged. First note the `AgenticChatEngine::new` signature gains a trailing `delta_sink: Option<DeltaSink>` arg. Add:

```rust
#[tokio::test]
async fn agentic_engine_streams_deltas_for_a_plain_answer() {
    use llm_chat::chat::DeltaSink;
    let runner = Arc::new(FakeChatRunner::with_deltas(
        vec![reply("All clear, nothing to do.")],
        vec![vec!["All clear, ".into(), "nothing to do.".into()]],
    ));
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let s = seen.clone();
    let sink: DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
    let eng = AgenticChatEngine::new(
        runner as Arc<dyn ChatRunner>,
        Arc::new(NoToolDispatcher) as Arc<dyn ToolDispatcher>,
        Arc::new(Brake::new()),
        "p".into(), "framing".into(), "m".into(), 0,
    ).with_delta_sink(Some(sink));
    let out = eng.respond("status?", &empty_catalog()).await;
    assert_eq!(out.text, "All clear, nothing to do.");
    assert!(out.tool_calls.is_empty());
    assert_eq!(*seen.lock().unwrap(), vec!["All clear, ".to_string(), "nothing to do.".to_string()]);
}
```

Reuse whatever `reply(...)`, dispatcher fake, and catalog helpers already exist in that test module (check the helpers around line 1227 — `mk(...)`, `reply(...)`). If a no-op dispatcher / empty catalog helper does not already exist, use the ones the existing tests use (e.g. the dispatcher built in the `mk` helper) rather than inventing new names.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app agentic_engine_streams_deltas`
Expected: FAIL — `with_delta_sink` does not exist.

- [ ] **Step 3: Add a ConversationDeltaEmitter (prose sink + separate reset — vet F1), field, builder, stream call**

(Vet F1 — keep the `DeltaSink` prose-only; the step-boundary reset is a *separate*
signal, not a magic string in the prose channel.) In `lib.rs`, import the sink near
the other llm_chat imports:

```rust
use llm_chat::chat::{ChatRequest, ChatRunner, DeltaSink};
```

Add the emitter struct near the other root helpers (above `run()`):

```rust
/// Couples the display-only prose sink with an explicit step-boundary `reset`,
/// so the `DeltaSink` stays prose-only (vet F1) — no control marker rides the
/// prose channel. The engine calls `reset()` at the top of each model step and
/// passes `&sink` (prose fragments only) to `chat_stream`.
pub struct ConversationDeltaEmitter {
    pub sink: DeltaSink,
    pub reset: Box<dyn Fn() + Send + Sync>,
}
```

Add a field to `AgenticChatEngine`:

```rust
pub struct AgenticChatEngine {
    runner: Arc<dyn ChatRunner>,
    dispatcher: Arc<dyn ToolDispatcher>,
    brake: Arc<Brake>,
    dialogue_id: String,
    system_prompt_framing: String,
    model: String,
    thinking_budget: u32,
    max_steps: usize,
    delta: Option<ConversationDeltaEmitter>,
}
```

In `AgenticChatEngine::new`, initialise `delta: None,` and add a builder:

```rust
    /// Attach a display-only delta emitter (root emits throttled conversation.delta).
    pub fn with_delta_sink(mut self, delta: Option<ConversationDeltaEmitter>) -> Self {
        self.delta = delta;
        self
    }
```

In `respond`, replace the `self.runner.chat(&req).await` call with a stream call
that fires the explicit `reset()` at the start of each step (NOT through the prose
sink) and forwards prose deltas. Replace:

```rust
            let reply = match self.runner.chat(&req).await {
                Ok(r) => r,
                Err(e) => return EngineReply { text: format!("[terminal error] {e}"), tool_calls },
            };
```

with:

```rust
            // Display-only streaming: each step resets the live bubble via the
            // explicit reset signal (prose sink stays prose-only — vet F1), then
            // forwards prose fragments. Tool-call extraction below still runs on
            // the COMPLETE reply.text (never partial JSON).
            let reply = match &self.delta {
                Some(emitter) => {
                    (emitter.reset)();
                    self.runner.chat_stream(&req, &emitter.sink).await
                }
                None => self.runner.chat(&req).await,
            };
            let reply = match reply {
                Ok(r) => r,
                Err(e) => return EngineReply { text: format!("[terminal error] {e}"), tool_calls },
            };
```

Apply the SAME field (`delta: Option<ConversationDeltaEmitter>`) + builder +
None-default to `LlmEngine`, and switch its `respond` to stream when present:

```rust
// In LlmEngine struct add: delta: Option<ConversationDeltaEmitter>,
// In LlmEngine::new initialise delta: None,
// Add the same with_delta_sink builder.
// In respond:
let reply = match &self.delta {
    Some(emitter) => { (emitter.reset)(); self.runner.chat_stream(&req, &emitter.sink).await }
    None => self.runner.chat(&req).await,
};
```

Adjust the Task 6 Step 1 test to build the emitter (prose sink unchanged; reset is
a no-op for the assertion):

```rust
    let emitter = ConversationDeltaEmitter { sink, reset: Box::new(|| {}) };
    let eng = AgenticChatEngine::new(/* …same args… */)
        .with_delta_sink(Some(emitter));
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app agentic_engine_streams_deltas`
Expected: PASS. Then run the whole engine test module: `cargo test -p app` — all existing engine tests still green (they construct engines without `with_delta_sink`, so `delta_sink` is `None` and `chat()` is used, behaviour unchanged).

- [ ] **Step 5: Add the throttling sink + wire it at run()**

In `lib.rs`, add a helper that builds a `ConversationDeltaEmitter` (prose sink +
explicit reset) emitting throttled `conversation.delta` Tauri events. Add near the
other root helpers (above `run()`):

```rust
/// Build a display-only ConversationDeltaEmitter that emits throttled
/// `conversation.delta` Tauri events. The prose `sink` coalesces fragments and
/// flushes every ~50ms or when the buffer reaches ~80 chars. The separate
/// `reset` flushes any buffered prose then emits a `{reset:true}` boundary (a new
/// model step started) — vet F1: the prose channel stays prose-only. Display-only:
/// payload is prose + a reset flag; no stream-json idiom crosses here.
fn make_conversation_delta_sink(handle: tauri::AppHandle) -> ConversationDeltaEmitter {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    struct Buf { text: String, last: Instant }
    let state = Arc::new(Mutex::new(Buf { text: String::new(), last: Instant::now() }));

    let sink_state = state.clone();
    let sink_handle = handle.clone();
    let sink: DeltaSink = Box::new(move |frag: &str| {
        let mut b = sink_state.lock().unwrap();
        b.text.push_str(frag);
        let due = b.last.elapsed() >= Duration::from_millis(50) || b.text.len() >= 80;
        if due {
            let _ = sink_handle.emit("conversation.delta", serde_json::json!({ "text": b.text, "reset": false }));
            b.text.clear();
            b.last = Instant::now();
        }
    });

    let reset: Box<dyn Fn() + Send + Sync> = Box::new(move || {
        let mut b = state.lock().unwrap();
        if !b.text.is_empty() {
            let _ = handle.emit("conversation.delta", serde_json::json!({ "text": b.text, "reset": false }));
            b.text.clear();
        }
        let _ = handle.emit("conversation.delta", serde_json::json!({ "text": "", "reset": true }));
        b.last = Instant::now();
    });

    ConversationDeltaEmitter { sink, reset }
}
```

Then in `run()`, build an emitter per engine and attach via the builder. Change the
`agentic_engine` construction (around line 717) to append
`.with_delta_sink(Some(make_conversation_delta_sink(handle.clone())))` after
`AgenticChatEngine::new(...)`. (Wrap the `Arc::new(AgenticChatEngine::new(...))` so
the builder runs before the `Arc::new`.)

Important: a final flush is needed when the model finishes a step with prose still buffered under the throttle. The next `"\u{0}reset"` (next step) flushes it; the LAST step's residue is flushed by the authoritative `send_message` result replacing the bubble on the frontend — so a missed final fragment is harmless (the frontend swaps in the full persisted turn). No extra flush call required.

- [ ] **Step 6: Run cargo check + the app tests**

Run: `cargo check -p app && cargo test -p app`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(c2): root engines stream prose via throttled conversation.delta sink"
```

---

## Task 7: frontend — listen to conversation.delta

**Files:**
- Modify: `src/ipc/terminal.ts`
- Test: `src/ipc/terminal.test.ts`

- [ ] **Step 1: Write the failing test**

Check `src/ipc/terminal.test.ts` for the existing mock of `@tauri-apps/api`. Add a test that `onConversationDelta` subscribes to the `conversation.delta` event and forwards payloads. Mirror the mocking pattern already used in `src/hooks/useRuntimeEvents.test.ts` (it mocks `@tauri-apps/api/event`'s `listen`). Add:

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

import { listen } from "@tauri-apps/api/event";
import { onConversationDelta } from "./terminal";

describe("onConversationDelta", () => {
  beforeEach(() => vi.clearAllMocks());
  it("subscribes to conversation.delta and forwards payloads", async () => {
    const handlers: Array<(e: { payload: unknown }) => void> = [];
    (listen as unknown as vi.Mock).mockImplementation((_name: string, cb: (e: { payload: unknown }) => void) => {
      handlers.push(cb);
      return Promise.resolve(() => {});
    });
    const got: Array<{ text: string; reset: boolean }> = [];
    await onConversationDelta((d) => got.push(d));
    expect(listen).toHaveBeenCalledWith("conversation.delta", expect.any(Function));
    handlers[0]({ payload: { text: "hi", reset: false } });
    expect(got).toEqual([{ text: "hi", reset: false }]);
  });
});
```

If `terminal.test.ts` already mocks `@tauri-apps/api/core` (for `invoke`), MERGE — keep both mocks; don't duplicate the `vi.mock` for the same module.

- [ ] **Step 2: Run test to verify it fails**

Run (export bun PATH first): `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run src/ipc/terminal.test.ts`
Expected: FAIL — `onConversationDelta` is not exported.

- [ ] **Step 3: Add the type + listener**

In `src/ipc/terminal.ts`, add the import and the listener:

```ts
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ConversationDelta {
  text: string;
  reset: boolean;
}

/// Subscribe to backend display-only streaming fragments for the terminal. The
/// payload is plain prose plus a `reset` flag marking a new model step (clears
/// the live bubble). Authoritative turns still arrive via `sendMessage`'s result.
export async function onConversationDelta(
  cb: (delta: ConversationDelta) => void,
): Promise<UnlistenFn> {
  return await listen<ConversationDelta>("conversation.delta", (e) => cb(e.payload));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run src/ipc/terminal.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/terminal.ts src/ipc/terminal.test.ts
git commit -m "feat(c2): frontend onConversationDelta listener + ConversationDelta type"
```

---

## Task 8: frontend — useConversation accumulates streaming text

**Files:**
- Modify: `src/hooks/useConversation.ts`
- Test: `src/hooks/useConversation.test.ts`

- [ ] **Step 1: Write the failing test**

Open `src/hooks/useConversation.test.ts` to match its existing mock/render setup (it uses `@testing-library/react`'s `renderHook` + mocks `../ipc/terminal`). Add a test that delta events accumulate into a `streaming` string and a `reset` clears it. Add (adapt mock names to the file's existing style):

```ts
it("accumulates conversation.delta into streaming text and resets on boundary", async () => {
  let deltaCb: ((d: { text: string; reset: boolean }) => void) | undefined;
  vi.mocked(onConversationDelta).mockImplementation(async (cb) => {
    deltaCb = cb;
    return () => {};
  });
  const { result } = renderHook(() => useConversation());
  await act(async () => { deltaCb?.({ text: "Hel", reset: false }); });
  await act(async () => { deltaCb?.({ text: "lo", reset: false }); });
  expect(result.current.streaming).toBe("Hello");
  await act(async () => { deltaCb?.({ text: "", reset: true }); });
  expect(result.current.streaming).toBe("");
});
```

Ensure `onConversationDelta` is added to the existing `vi.mock("../ipc/terminal", ...)` factory in that file (return a `vi.fn()` for it alongside `getConversation`/`sendMessage`).

- [ ] **Step 2: Run test to verify it fails**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run src/hooks/useConversation.test.ts`
Expected: FAIL — `result.current.streaming` is undefined / `onConversationDelta` not subscribed.

- [ ] **Step 3: Subscribe + accumulate in the hook**

Rewrite `src/hooks/useConversation.ts`:

```ts
import { useCallback, useEffect, useState } from "react";
import { getConversation, onConversationDelta, sendMessage, type Turn } from "../ipc/terminal";

/// Loads the project's persistent conversation and exposes a `send` that posts a
/// user line and replaces local turns with the backend's authoritative result.
/// Also subscribes to display-only `conversation.delta` streaming fragments,
/// accumulating them into `streaming` (a transient assistant bubble) that the
/// terminal shows until the authoritative turns arrive.
export function useConversation() {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [busy, setBusy] = useState(false);
  const [streaming, setStreaming] = useState("");

  useEffect(() => {
    let cancelled = false;
    getConversation()
      .then((c) => { if (!cancelled && c) setTurns(c.turns); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await onConversationDelta((d) => {
        setStreaming((prev) => (d.reset ? "" : prev + d.text));
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const send = useCallback(async (input: string) => {
    const trimmed = input.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      const c = await sendMessage(trimmed);
      setTurns(c.turns);
    } finally {
      setStreaming("");
      setBusy(false);
    }
  }, []);

  return { turns, send, busy, streaming };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run src/hooks/useConversation.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useConversation.ts src/hooks/useConversation.test.ts
git commit -m "feat(c2): useConversation accumulates streaming deltas into a live bubble"
```

---

## Task 9: frontend — Terminal renders the live streaming bubble

**Files:**
- Modify: `src/components/Terminal.tsx`
- Test: `src/components/Terminal.test.tsx`

- [ ] **Step 1: Write the failing test**

Open `src/components/Terminal.test.tsx` to match its render helper. Add a test that a non-empty `streaming` prop renders a live assistant bubble while empty renders none:

```tsx
it("renders the live streaming bubble when streaming text is present", () => {
  render(<Terminal turns={[]} contextLine="ctx" onSend={() => {}} streaming="typing now" />);
  expect(screen.getByText("typing now")).toBeInTheDocument();
});

it("renders no streaming bubble when streaming is empty", () => {
  const { container } = render(<Terminal turns={[]} contextLine="ctx" onSend={() => {}} streaming="" />);
  expect(container.querySelector('[data-streaming="true"]')).toBeNull();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run src/components/Terminal.test.tsx`
Expected: FAIL — `Terminal` has no `streaming` prop; the bubble is not rendered.

- [ ] **Step 3: Add the `streaming` prop + transient bubble**

In `src/components/Terminal.tsx`, add `streaming` to `TerminalProps` (optional, default `""`) and render a transient assistant bubble after the mapped turns:

```tsx
interface TerminalProps {
  turns: Turn[];
  contextLine: string;
  onSend: (input: string) => void;
  streaming?: string;
}

export function Terminal({ turns, contextLine, onSend, streaming = "" }: TerminalProps) {
```

Inside the scroll container, AFTER the `turns.map(...)` block and before its closing `</div>`, add:

```tsx
            {streaming && (
              <div data-streaming="true" style={{ marginBottom: 10 }}>
                <div style={{ color: "var(--text-3)", marginBottom: 2 }}>claude</div>
                <div style={{ color: "var(--text)" }}>
                  {streaming}
                  <span style={{ color: "var(--accent)" }}>▍</span>
                </div>
              </div>
            )}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run src/components/Terminal.test.tsx`
Expected: PASS.

- [ ] **Step 5: Wire `streaming` from the hook to the component**

Find where `<Terminal ... />` is rendered (grep `useConversation` / `<Terminal`, likely `src/App.tsx`). Pass the hook's `streaming` through:

```tsx
const { turns, send, busy, streaming } = useConversation();
// ...
<Terminal turns={turns} contextLine={contextLine} onSend={send} streaming={streaming} />
```

(Keep existing props intact; only add `streaming`. If `busy` is already used, leave it.)

- [ ] **Step 6: Run the full frontend test suite**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run`
Expected: PASS (all suites, including the new ones).

- [ ] **Step 7: Commit**

```bash
git add src/components/Terminal.tsx src/components/Terminal.test.tsx src/App.tsx
git commit -m "feat(c2): Terminal renders live streaming bubble wired from useConversation"
```

---

## Task 10: full verification + merge

**Files:** none (verification only)

- [ ] **Step 1: Backend full suite + lints**

Run:
```bash
cargo test --workspace
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: all PASS, clippy clean. Fix any clippy findings (e.g. drop an unused import, prefer `is_empty`) and amend the relevant commit.

- [ ] **Step 2: Frontend full suite + build**

Run:
```bash
export PATH="/opt/homebrew/bin:$PATH"
bun vitest run
bun run build
```
Expected: all tests PASS, build succeeds (tsc + vite).

- [ ] **Step 3: Merge to main (no-ff) + tag**

Run:
```bash
git checkout main
git merge --no-ff plan-c2-chat-streaming -m "Merge C2: display-only token streaming for the god terminal

Adds chat_stream to the ChatRunner ACL (DeltaSink prose callback), an
incremental stream-json parser that forwards assistant prose deltas, scripted
FakeChatRunner deltas, root engines that emit throttled conversation.delta
events, and a frontend live streaming bubble. Tool-call extraction + turn
recording still run on the COMPLETE reply (display-only streaming)."
git tag plan-c2-chat-streaming
```
(Do NOT push. Stay on `main`.)

---

## Task 11: backlog update

**Files:**
- Modify: `docs/v1.1-backlog.md`

- [ ] **Step 1: Mark C2 done**

Change the C2 line under "Terminal / chat" from `- [ ]` to `- [x]` and append: `` `done` (tag `plan-c2-chat-streaming`) `` plus a one-line summary of what shipped (chat_stream DeltaSink seam, incremental parser, conversation.delta throttled events, live frontend bubble; display-only — loop + Turn unchanged; vet `docs/vet-c2-chat-streaming-2026-06-23.md`).

- [ ] **Step 2: Commit**

```bash
git add docs/v1.1-backlog.md
git commit -m "docs(backlog): mark C2 done (tag plan-c2-chat-streaming)"
```

---

## Self-Review Notes

- **Spec coverage:** incremental stream-json parse (Tasks 1–2); throttled events (Task 6 sink); `chat_stream` capability on `ChatRunner` returning final `ChatReply` (Tasks 3–4); FakeChatRunner scripted-deltas (Task 5); display-only — loop + `Turn` unchanged (Task 6 keeps `extract_tool_call_block` on full `reply.text`); frontend appends deltas (Tasks 7–9). Covered.
- **ACL discipline:** `DeltaSink` is `Box<dyn Fn(&str)>` — no stream-json type, `session_id`, or event name crosses out (Decisions). Event name + throttling owned by the root.
- **Non-streaming preserved:** `chat()` and `parse_chat_stream` unchanged; engines fall back to `chat()` when `delta_sink` is `None`; all existing tests construct engines without a sink.
- **Type consistency:** `DeltaSink`, `chat_stream`, `parse_chat_stream_streaming`, `with_delta_sink`, `with_deltas`, `make_conversation_delta_sink`, `onConversationDelta`, `ConversationDelta`, `streaming` prop — used consistently across tasks.
- **Caveat:** with no live `claude`, real streaming parses buffered stdout incrementally (same parser path); the live subprocess line-by-line pipe is structurally covered, not end-to-end.
