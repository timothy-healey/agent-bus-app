# R4 — Per-chunk live-log streaming from workers — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stream each running worker's assistant output chunk-by-chunk to the UI so a task's live log fills in as the worker runs, displayed in the card drawer's existing "live log" tab.

**Architecture:** Mirror C2's just-landed *chat* streaming pattern (`llm_chat`) for the *worker* runner (`runners`). Add a display-only `DeltaSink` + a streaming `invoke_stream` seam to the `Runner` trait (defaulting to `invoke`, so non-streaming runners keep working). Make `runners/src/stream_json.rs` incremental (a `feed` that returns the prose delta + a `parse_stream_streaming`), mirroring `llm_chat`. `ClaudeCliRunner` forwards deltas; `FakeRunner` gains a scripted-deltas mode. The pool (`runtime`) accepts an optional per-invocation log sink and calls `invoke_stream` when one is present, forwarding throttled `task.log` events from the composition root. The FINAL verdict/artifact/usage parse + settle/route are byte-for-byte unchanged. Frontend: a `task.log` listener accumulates per-task deltas into a buffer the CardDrawer's "live log" tab renders.

**Tech Stack:** Rust (cargo workspace under `src-tauri/`, tokio, async-trait, serde_json, thiserror), Tauri events, React + TypeScript + Vitest, `bun`.

---

## Design constraints (read before starting)

- **ACL stays sealed.** No stream-json idiom (event `type`, `assistant`/`result`, JSON shapes) may leak into Runtime. The only thing crossing the `Runner` ACL for streaming is a `&str` prose fragment through a `LogSink` callback — exactly as C2's `DeltaSink` does (`src-tauri/llm_chat/src/chat.rs:75`).
- **Streaming is additive.** `Runner::invoke` and its result (`RunnerOutput` = verdict + artifact + final_text + usage) are unchanged. `invoke_stream` returns the *identical* `RunnerOutput`; the only difference is prose fragments are forwarded to a sink as they parse. The default `invoke_stream` delegates to `invoke` (no deltas), so `FakeRunner` and any other runner keep compiling.
- **Two-aggregate model untouched.** Task and WorkerPool aggregates, the verdict route, fork/join barrier — all unchanged. The log sink is a *display-only side channel*; it never influences settle/route.
- **Documented duplication.** The incremental stream-json parser in `runners` deliberately mirrors `llm_chat`'s rather than sharing it — they are separate ACL crates by design (same reasoning C2 recorded). Add a doc-comment noting the parallel, per the DDD vet's expected ruling.
- **Mirror these C2 files exactly in spirit:**
  - `src-tauri/llm_chat/src/chat.rs:75-92` — `DeltaSink` type + defaulted `chat_stream` seam.
  - `src-tauri/llm_chat/src/stream_json.rs` — `feed` returns the delta; `parse_chat_stream_streaming` forwards non-empty deltas.
  - `src-tauri/llm_chat/src/claude_cli.rs:49-109` — one parameterised run core; streaming + non-streaming both route through it.
  - `src-tauri/llm_chat/src/fake.rs:13-92` — `with_deltas` scripted-deltas constructor.
  - `src-tauri/app/src/lib.rs:121-151` — throttled, coalescing event sink built at the composition root.

## File Structure

**Backend (Rust, `src-tauri/`):**
- `runners/src/output.rs` — Modify. Add `LogSink` type alias + defaulted `Runner::invoke_stream`. (~15 lines)
- `runners/src/stream_json.rs` — Modify. `StreamAccumulator::feed` returns the prose delta this event added; add `parse_stream_streaming`. The whole-buffer `parse_stream` delegates. (~30 lines + tests)
- `runners/src/claude_cli.rs` — Modify. One `run_once(req, forward)` core; `invoke` passes a no-op forwarder, `invoke_stream` forwards to the sink. (~20 lines + tests)
- `runners/src/fake.rs` — Modify. `with_deltas` constructor + scripted-delta forwarding in `invoke_stream`. (~25 lines + tests)
- `runtime/src/pool.rs` — Modify. `PoolContext.log_sink: Option<Arc<dyn LogSink-ish>>` (a per-task forwarder factory); `process_one_claim` calls `invoke_stream` with a task-scoped sink when present, else `invoke`. Settle/route unchanged. (~25 lines + tests)
- `app/src/lib.rs` — Modify. `make_task_log_sink(handle)` builds a throttled per-task coalescing sink that emits `task.log` `{task_id, delta}`; wire it into `spawn_worker_loops` / `PoolContext`. (~40 lines + test)

**Frontend (`src/`):**
- `src/ipc/runtime.ts` — Modify. `TaskLog` type + `onTaskLog` listener. (~12 lines)
- `src/hooks/useTaskLog.ts` — Create. Subscribes to `task.log`, accumulates a per-task-id buffer, exposes `logFor(taskId)`. (~35 lines)
- `src/hooks/useTaskLog.test.ts` — Create. (~40 lines)
- `src/App.tsx` — Modify. Use `useTaskLog`, pass `logText={liveLog.logFor(openTask.id)}` to `CardDrawer`. (~4 lines)

The CardDrawer "live log" tab already exists (`src/components/CardDrawer.tsx:124-139`) and already renders a `logText` prop; we only need to feed it.

---

## Task 1: `LogSink` type + defaulted `invoke_stream` seam on the `Runner` trait

**Files:**
- Modify: `src-tauri/runners/src/output.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/runners/src/output.rs` (after `runner_usage_default_is_zeroed`):

```rust
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
        };
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: LogSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        // FakeRunner has not been given the streaming override yet, so the
        // default invoke_stream must delegate to invoke and forward nothing.
        let result = fake.invoke_stream(&req, &sink).await.unwrap();
        assert_eq!(result.verdict, Verdict::Approve);
        assert!(seen.lock().unwrap().is_empty());
    }
```

This requires `use agent_bus_core::Verdict;` — already imported in the test module.

- [ ] **Step 2: Run test to verify it fails (does not compile yet)**

Run: `cd src-tauri && cargo test -p runners --lib output::tests::default_invoke_stream_delegates_to_invoke_with_no_deltas 2>&1 | tail -20`
Expected: FAIL — `cannot find type \`LogSink\`` and `no method named \`invoke_stream\``.

- [ ] **Step 3: Add the `LogSink` type + defaulted `invoke_stream` to the trait**

In `src-tauri/runners/src/output.rs`, replace the trait block (lines 81-87) with:

```rust
/// A display-only log sink. The streaming worker path forwards each assistant
/// text fragment here as it parses, for live-log display. Deliberately a plain
/// `&str` callback: NO stream-json idiom, CLI flag, or event name crosses the
/// ACL through it — the composition root maps fragments to whatever UI event it
/// likes. Mirrors `llm_chat::chat::DeltaSink` (separate ACL crate, by design).
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p runners --lib output::tests::default_invoke_stream_delegates_to_invoke_with_no_deltas 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runners/src/output.rs
git commit -m "feat(r4): Runner gains LogSink + defaulted invoke_stream seam (mirrors C2 DeltaSink)"
```

---

## Task 2: Incremental stream-json parse in `runners` (feed returns the delta)

**Files:**
- Modify: `src-tauri/runners/src/stream_json.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src-tauri/runners/src/stream_json.rs`:

```rust
    #[test]
    fn feed_returns_prose_delta_for_assistant_event_only() {
        use serde_json::Value;
        let mut acc = StreamAccumulator::new();
        let sys: Value = serde_json::from_str(
            r#"{"type":"system","subtype":"init","model":"m"}"#,
        ).unwrap();
        let asst: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello "}]}}"#,
        ).unwrap();
        let res: Value = serde_json::from_str(
            r#"{"type":"result","subtype":"success","result":"hello world"}"#,
        ).unwrap();
        assert_eq!(acc.feed(&sys).unwrap(), "");
        assert_eq!(acc.feed(&asst).unwrap(), "hello ");
        // the result line carries authoritative text but is NOT a streamed delta
        assert_eq!(acc.feed(&res).unwrap(), "");
    }

    #[test]
    fn streaming_parse_forwards_only_assistant_prose_and_returns_same_output() {
        let mut deltas: Vec<String> = vec![];
        let out = parse_stream_streaming(SAMPLE, "fallback-model", &mut |d| deltas.push(d.to_string())).unwrap();
        // identical final output to the whole-buffer parse
        let plain = parse_stream(SAMPLE, "fallback-model").unwrap();
        assert_eq!(out, plain);
        // the two assistant lines streamed their prose; the result line did not
        assert_eq!(deltas, vec![
            "Analysing the repository.\n".to_string(),
            "VERDICT: approve\nARTIFACT: artifacts/analyses/T-1-v1.md".to_string(),
        ]);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runners --lib stream_json 2>&1 | tail -25`
Expected: FAIL — `feed` returns `()` not `String`; `parse_stream_streaming` not found.

- [ ] **Step 3: Make `feed` return the prose delta**

In `src-tauri/runners/src/stream_json.rs`, change the signature and body of `feed` (lines 26-86). Replace the whole `feed` method with:

```rust
    /// Feed one stream-json line (already a parsed Value). Returns the prose text
    /// *this* event added (empty for non-prose events) so a streaming caller can
    /// forward it; the accumulator keeps the running full text for the final
    /// RunnerOutput. Returns Err only on a recognised rate-limit error event.
    pub fn feed(&mut self, v: &Value) -> Result<String, RunnerError> {
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let mut delta = String::new();

        // Rate-limit detection: an error event whose message mentions rate/429.
        if ty == "error" || v.get("is_error").and_then(|b| b.as_bool()) == Some(true) {
            let msg = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| v.get("result").and_then(|r| r.as_str()))
                .unwrap_or("error")
                .to_string();
            let lower = msg.to_lowercase();
            if lower.contains("rate") || lower.contains("429") || lower.contains("quota") {
                return Err(RunnerError::RateLimited(msg));
            }
        }

        match ty {
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
            "result" => {
                self.saw_result = true;
                if let Some(u) = v.get("usage") {
                    // The result usage is authoritative for totals — replace,
                    // but keep the model already captured from the system line.
                    let model = std::mem::take(&mut self.usage.model);
                    self.usage = RunnerUsage { model, ..Default::default() };
                    self.add_usage(u);
                }
                if self.text.is_empty() {
                    if let Some(r) = v.get("result").and_then(|r| r.as_str()) {
                        self.text = r.to_string();
                    }
                }
            }
            "system" => {
                if let Some(model) = v.get("model").and_then(|m| m.as_str()) {
                    self.usage.model = model.to_string();
                }
            }
            _ => {}
        }
        Ok(delta)
    }
```

Note: the only behavioural change vs the original is the return value (the prose delta) and that assistant text is pushed via the local `delta` first; the result line's authoritative-text fallback is unchanged.

- [ ] **Step 4: Update `parse_stream` for the new `feed` return + add `parse_stream_streaming`**

In `parse_stream` (lines 144-156), the call `acc.feed(&v)?;` now returns a `String`; discard it. Replace the loop body line `acc.feed(&v)?;` with `let _ = acc.feed(&v)?;`.

Then add, right after `parse_stream`:

```rust
/// Streaming variant of `parse_stream`. Parses the same newline-delimited JSON
/// but invokes `on_delta` with each assistant *prose fragment* as it is parsed
/// (display-only), then returns the identical final RunnerOutput. The result
/// line's authoritative prose is NOT forwarded as a delta — it has already been
/// streamed via the assistant events. Mirrors
/// `llm_chat::stream_json::parse_chat_stream_streaming` (separate ACL crate).
pub fn parse_stream_streaming(
    raw: &str,
    model: &str,
    on_delta: &mut dyn FnMut(&str),
) -> Result<RunnerOutput, RunnerError> {
    let mut acc = StreamAccumulator::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| RunnerError::Other(format!("bad stream-json line: {e}")))?;
        let delta = acc.feed(&v)?;
        if !delta.is_empty() {
            on_delta(&delta);
        }
    }
    acc.finish(model)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p runners --lib stream_json 2>&1 | tail -25`
Expected: PASS (all existing stream_json tests + the 2 new ones).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runners/src/stream_json.rs
git commit -m "feat(r4): stream_json feed returns per-event prose delta + parse_stream_streaming"
```

---

## Task 3: `ClaudeCliRunner` forwards deltas via `invoke_stream`

**Files:**
- Modify: `src-tauri/runners/src/claude_cli.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/runners/src/claude_cli.rs`:

```rust
    #[tokio::test]
    async fn invoke_stream_forwards_prose_deltas_and_returns_same_output() {
        use crate::output::LogSink;
        use std::sync::{Arc, Mutex};
        let canned = r#"{"type":"system","model":"claude-opus-4-7"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Analysing. "}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Analysing. VERDICT: approve","usage":{"input_tokens":5,"output_tokens":7}}"#;
        let runner = ClaudeCliRunner::with_spawner(Box::new(move |_args| Ok(canned.to_string())));
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        let sink: LogSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let out = runner.invoke_stream(&req(), &sink).await.unwrap();
        // final output identical to the non-streaming path
        assert_eq!(out.verdict, Verdict::Approve);
        // the assistant prose was streamed (result line is not a delta)
        assert_eq!(*seen.lock().unwrap(), vec!["Analysing. ".to_string()]);
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runners --lib claude_cli 2>&1 | tail -25`
Expected: FAIL — `invoke_stream` falls back to the default (delegates to `invoke`), so `seen` is empty → first assertion fails.

- [ ] **Step 3: Add a parameterised run core + override `invoke_stream`**

In `src-tauri/runners/src/claude_cli.rs`, update the imports at the top:

```rust
use crate::command::{build_args, CLAUDE_BIN};
use crate::output::{InvocationRequest, LogSink, Runner, RunnerError, RunnerOutput};
use crate::stream_json::parse_stream_streaming;
use async_trait::async_trait;
```

Add a private run core to the `impl ClaudeCliRunner` block (after `with_spawner`):

```rust
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
```

Replace the `#[async_trait] impl Runner for ClaudeCliRunner` block (lines 48-55) with:

```rust
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
```

Remove the now-unused `use crate::stream_json::parse_stream;` import (it was line 9; replaced by `parse_stream_streaming`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p runners --lib claude_cli 2>&1 | tail -25`
Expected: PASS (all existing claude_cli tests — `invoke_parses_canned_stdout...`, spawn-failure, rate-limit — plus the 2 new ones).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runners/src/claude_cli.rs
git commit -m "feat(r4): ClaudeCliRunner one run-core; invoke_stream forwards prose deltas"
```

---

## Task 4: `FakeRunner` scripted-deltas mode

**Files:**
- Modify: `src-tauri/runners/src/fake.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src-tauri/runners/src/fake.rs`:

```rust
    #[tokio::test]
    async fn scripted_deltas_are_forwarded_then_output_returned() {
        use crate::output::LogSink;
        use std::sync::{Arc, Mutex};
        let fake = FakeRunner::with_deltas(
            vec![Ok(output(Verdict::Approve))],
            vec![vec!["Analy".into(), "sing.".into()]],
        );
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: LogSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let out = fake.invoke_stream(&req(), &sink).await.unwrap();
        assert_eq!(out.verdict, Verdict::Approve);
        assert_eq!(*seen.lock().unwrap(), vec!["Analy".to_string(), "sing.".to_string()]);
    }

    #[tokio::test]
    async fn deltas_default_to_empty_for_new_constructor() {
        use crate::output::LogSink;
        use std::sync::{Arc, Mutex};
        let fake = FakeRunner::always(output(Verdict::Approve));
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: LogSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let _ = fake.invoke_stream(&req(), &sink).await.unwrap();
        assert!(seen.lock().unwrap().is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runners --lib fake 2>&1 | tail -25`
Expected: FAIL — `with_deltas` not found; `invoke_stream` is the default (no deltas) so the first test's `seen` is empty.

- [ ] **Step 3: Add the `deltas` field, `with_deltas`, and `invoke_stream` override**

In `src-tauri/runners/src/fake.rs`, update the imports:

```rust
use crate::output::{InvocationRequest, LogSink, Runner, RunnerError, RunnerOutput};
use async_trait::async_trait;
use std::sync::Mutex;
```

Replace the struct + `impl FakeRunner` block (lines 9-25) with:

```rust
pub struct FakeRunner {
    /// The next outputs to return, in order. When exhausted, the last is reused.
    responses: Vec<Result<RunnerOutput, RunnerError>>,
    /// Per-call scripted prose deltas to forward via invoke_stream before
    /// returning that call's output. A call with no entry (index >= len)
    /// forwards nothing. Indexed by the same cursor as `responses`.
    deltas: Vec<Vec<String>>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<InvocationRequest>>,
}

impl FakeRunner {
    pub fn new(responses: Vec<Result<RunnerOutput, RunnerError>>) -> Self {
        Self { responses, deltas: vec![], cursor: Mutex::new(0), received: Mutex::new(Vec::new()) }
    }

    /// Seed responses AND, per call, the ordered prose deltas invoke_stream
    /// forwards before returning that call's output.
    pub fn with_deltas(
        responses: Vec<Result<RunnerOutput, RunnerError>>,
        deltas: Vec<Vec<String>>,
    ) -> Self {
        Self { responses, deltas, cursor: Mutex::new(0), received: Mutex::new(Vec::new()) }
    }

    /// Convenience: always returns the given output.
    pub fn always(output: RunnerOutput) -> Self {
        Self::new(vec![Ok(output)])
    }
}
```

Replace the `#[async_trait] impl Runner for FakeRunner` block (lines 27-39) with:

```rust
#[async_trait]
impl Runner for FakeRunner {
    async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
        self.received.lock().unwrap().push(req.clone());
        let mut cur = self.cursor.lock().unwrap();
        let idx = (*cur).min(self.responses.len() - 1);
        *cur += 1;
        match &self.responses[idx] {
            Ok(o) => Ok(o.clone()),
            Err(e) => Err(clone_err(e)),
        }
    }

    async fn invoke_stream(
        &self,
        req: &InvocationRequest,
        sink: &LogSink,
    ) -> Result<RunnerOutput, RunnerError> {
        self.received.lock().unwrap().push(req.clone());
        let idx = {
            let mut cur = self.cursor.lock().unwrap();
            let i = (*cur).min(self.responses.len() - 1);
            *cur += 1;
            i
        };
        // Forward this call's scripted deltas (if any) before returning.
        if let Some(call_deltas) = self.deltas.get(idx) {
            for d in call_deltas {
                sink(d);
            }
        }
        match &self.responses[idx] {
            Ok(o) => Ok(o.clone()),
            Err(e) => Err(clone_err(e)),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p runners --lib fake 2>&1 | tail -25`
Expected: PASS (`returns_seeded_responses_in_order_then_repeats_last` + the 2 new ones).

- [ ] **Step 5: Run the full runners crate to confirm nothing regressed**

Run: `cd src-tauri && cargo test -p runners 2>&1 | grep "test result"`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runners/src/fake.rs
git commit -m "feat(r4): FakeRunner gains with_deltas scripted-deltas mode for streaming tests"
```

---

## Task 5: Pool forwards per-task log deltas via `invoke_stream`

The pool needs a way to obtain a per-task `LogSink`. We pass an optional factory into `PoolContext` that, given a `task_id`, builds a `LogSink`. When present, `process_one_claim` calls `invoke_stream`; otherwise `invoke`. The settle/route path is byte-for-byte unchanged.

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/runtime/src/pool.rs` (after `settle_emits_a_usage_event_to_the_sink`):

```rust
    #[tokio::test]
    async fn streaming_forwards_log_deltas_per_task_and_settles_unchanged() {
        use runners::output::LogSink;
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("gate-1"), None)],
            vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }]);
        // FakeRunner with scripted deltas for the single invocation.
        let runner = Arc::new(FakeRunner::with_deltas(
            vec![Ok(approve_output())],
            vec![vec!["chunk-a ".into(), "chunk-b".into()]],
        ));
        let mut ctx = ctx_with(pool.clone(), p.clone(), runner, temp_root());

        // Record (task_id, delta) pairs the factory's sinks emit.
        let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let seen_c = seen.clone();
        ctx.log_sink = Some(Arc::new(move |task_id: &str| -> LogSink {
            let tid = task_id.to_string();
            let seen_c = seen_c.clone();
            Box::new(move |d: &str| seen_c.lock().unwrap().push((tid.clone(), d.to_string())))
        }));

        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        let outcome = process_one_claim(&ctx, &p.teams[0]).await.unwrap();
        // settle is unchanged: approve into the gate, parked gated
        assert_eq!(outcome, ClaimOutcome::Settled {
            task_id: t.id.0.clone(), next_stage: "gate-1".into(), next_state: TaskState::Gated,
        });
        // the deltas were forwarded, tagged with this task's id
        let seen = seen.lock().unwrap();
        assert_eq!(*seen, vec![
            (t.id.0.clone(), "chunk-a ".to_string()),
            (t.id.0.clone(), "chunk-b".to_string()),
        ]);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p runtime --lib pool::tests::streaming_forwards_log_deltas_per_task_and_settles_unchanged 2>&1 | tail -25`
Expected: FAIL — `PoolContext` has no `log_sink` field.

- [ ] **Step 3: Add the `log_sink` factory field to `PoolContext`**

In `src-tauri/runtime/src/pool.rs`, add to the imports near the top:

```rust
use runners::output::{InvocationRequest, LogSink, Runner};
```

(replace the existing `use runners::output::{InvocationRequest, Runner};` line).

Add a type alias above `PoolContext` (after the `ClaimOutcome` enum, before `PoolContext`):

```rust
/// Builds a per-task display-only log sink. Given a task id, returns a `LogSink`
/// the runner forwards assistant prose to as the worker streams. Display-only:
/// the deltas never influence settle/route. None = no streaming (the runner's
/// non-streaming `invoke` is used) — the no-op case for runtime-only tests.
pub type LogSinkFactory = dyn Fn(&str) -> LogSink + Send + Sync;
```

Add the field to `PoolContext` (after `revision_reader`, inside the struct):

```rust
    /// Per-task log-sink factory (R4 live-log streaming). None = use the runner's
    /// non-streaming `invoke`. Display-only side channel; settle/route ignore it.
    pub log_sink: Option<Arc<LogSinkFactory>>,
```

- [ ] **Step 4: Use `invoke_stream` when a factory is present**

In `process_one_claim`, replace the single invoke line (currently `let result = ctx.runner.invoke(&req).await;`, line 116) with:

```rust
    let result = match &ctx.log_sink {
        Some(factory) => {
            let sink = factory(&task.id.0);
            ctx.runner.invoke_stream(&req, &sink).await
        }
        None => ctx.runner.invoke(&req).await,
    };
```

Everything after this point (cleanup, rate-limit handling, usage, settle_and_route) is unchanged.

- [ ] **Step 5: Add `log_sink: None` to every `PoolContext` construction in tests**

In `src-tauri/runtime/src/pool.rs`, the `ctx_with` helper (around line 356) builds a `PoolContext`. Add `log_sink: None,` to that struct literal (after `revision_reader: None,`).

Search the whole crate for other `PoolContext {` literals: `cd src-tauri && grep -rn "PoolContext {" runtime/src`. Add `log_sink: None,` to each (e.g. any in `contract_tests.rs` or `api.rs`). If a construction uses `..Default::default()` it is fine; otherwise add the field.

- [ ] **Step 6: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p runtime --lib pool 2>&1 | tail -25`
Expected: PASS (all existing pool tests + the new streaming test).

- [ ] **Step 7: Run the full runtime crate**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | grep "test result"`
Expected: all PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(r4): pool forwards per-task log deltas via invoke_stream (settle unchanged)"
```

---

## Task 6: Composition root — `task.log` event sink + wiring

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Write the failing test**

The throttle/coalesce sink is timing-based; test the *non-throttled correctness* by setting a buffer threshold that flushes immediately. We test the factory shape: each task id gets a sink that, on flush, emits `task.log` with `{task_id, delta}`. Because Tauri's `AppHandle` is hard to construct in a unit test, we factor the coalescing logic into a small pure helper `TaskLogBuffer` and test that. Add a `#[cfg(test)] mod task_log_tests` near the other test modules:

```rust
#[cfg(test)]
mod task_log_tests {
    use super::TaskLogBuffer;
    use std::sync::{Arc, Mutex};

    #[test]
    fn buffer_coalesces_until_flushed_then_emits_task_id_and_delta() {
        let emitted: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let e = emitted.clone();
        let mut buf = TaskLogBuffer::new("T-1".into());
        // push two fragments; force_flush combines and emits once with the task id
        buf.push("chunk-a ");
        buf.push("chunk-b");
        buf.force_flush(&mut |task_id: &str, delta: &str| {
            e.lock().unwrap().push((task_id.to_string(), delta.to_string()));
        });
        assert_eq!(*emitted.lock().unwrap(), vec![("T-1".to_string(), "chunk-a chunk-b".to_string())]);
    }

    #[test]
    fn force_flush_on_empty_buffer_emits_nothing() {
        let emitted: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let e = emitted.clone();
        let mut buf = TaskLogBuffer::new("T-2".into());
        buf.force_flush(&mut |t: &str, d: &str| e.lock().unwrap().push((t.to_string(), d.to_string())));
        assert!(emitted.lock().unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p app --lib task_log_tests 2>&1 | tail -20`
Expected: FAIL — `TaskLogBuffer` not found.

- [ ] **Step 3: Add `TaskLogBuffer` + `make_task_log_sink` to `app/src/lib.rs`**

In `src-tauri/app/src/lib.rs`, after `make_conversation_delta_sink` (line 151), add:

```rust
/// Coalescing buffer for one task's live-log fragments. Pure (no Tauri) so the
/// flush/emit shape is unit-tested. `push` accumulates; `flush_if_due` and
/// `force_flush` invoke the emit callback with `(task_id, combined_delta)` and
/// clear the buffer. Mirrors the conversation.delta coalescing (R4 / C2 F1).
pub struct TaskLogBuffer {
    task_id: String,
    text: String,
    last: std::time::Instant,
}

impl TaskLogBuffer {
    pub fn new(task_id: String) -> Self {
        Self { task_id, text: String::new(), last: std::time::Instant::now() }
    }

    pub fn push(&mut self, frag: &str) {
        self.text.push_str(frag);
    }

    /// Flush when ~50ms elapsed or the buffer reached ~80 chars (same thresholds
    /// as the conversation.delta sink), emitting (task_id, delta) and clearing.
    pub fn flush_if_due(&mut self, emit: &mut dyn FnMut(&str, &str)) {
        let due = self.last.elapsed() >= std::time::Duration::from_millis(50)
            || self.text.len() >= 80;
        if due {
            self.force_flush(emit);
        }
    }

    /// Emit whatever is buffered (if any) and clear; resets the timer.
    pub fn force_flush(&mut self, emit: &mut dyn FnMut(&str, &str)) {
        if !self.text.is_empty() {
            emit(&self.task_id, &self.text);
            self.text.clear();
        }
        self.last = std::time::Instant::now();
    }
}

/// Build a per-task `LogSink` factory that emits throttled `task.log` Tauri
/// events `{ task_id, delta }`. Display-only: the payload is the task id + a
/// prose fragment; no stream-json idiom crosses here. Each task gets its own
/// coalescing buffer so concurrent workers' logs never interleave within a flush.
fn make_task_log_sink(
    handle: tauri::AppHandle,
) -> Arc<runtime::pool::LogSinkFactory> {
    Arc::new(move |task_id: &str| -> runners::output::LogSink {
        use std::sync::Mutex;
        let buf = Arc::new(Mutex::new(TaskLogBuffer::new(task_id.to_string())));
        let handle = handle.clone();
        Box::new(move |frag: &str| {
            let mut b = buf.lock().unwrap();
            b.push(frag);
            b.flush_if_due(&mut |tid: &str, delta: &str| {
                let _ = handle.emit(
                    "task.log",
                    serde_json::json!({ "task_id": tid, "delta": delta }),
                );
            });
        })
    })
}
```

Ensure `use tauri::Emitter;` (or whatever brings `emit` into scope) is already present — it is, since `handle.emit("task.changed", ...)` already compiles in this file.

- [ ] **Step 4: Run the buffer test to verify it passes**

Run: `cd src-tauri && cargo test -p app --lib task_log_tests 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Wire the factory into the worker loops**

In `src-tauri/app/src/lib.rs`, `spawn_worker_loops` (line 913): add a parameter and set it on `PoolContext`.

Change the signature to add `log_sink: Option<Arc<runtime::pool::LogSinkFactory>>,` as the final argument:

```rust
#[allow(clippy::too_many_arguments)]
fn spawn_worker_loops(
    handle: tauri::AppHandle,
    pipeline: Arc<Pipeline>,
    tasks: Arc<TaskStore>,
    brake: Arc<Brake>,
    project_root: String,
    usage_sink: Option<Arc<dyn agent_bus_core::UsageSink>>,
    revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>>,
    pool: sqlx::SqlitePool,
    log_sink: Option<Arc<runtime::pool::LogSinkFactory>>,
) {
```

In the `PoolContext { ... }` literal inside that function (line 926), add as the last field:

```rust
            log_sink: log_sink.clone(),
```

At the call site (line 839), build and pass the sink:

```rust
                    spawn_worker_loops(handle.clone(), pipe.clone(), tasks.clone(), brake.clone(), project_root, Some(usage_sink.clone()), revision_reader, pool.clone(), Some(make_task_log_sink(handle.clone())));
```

- [ ] **Step 6: Run the full app crate to confirm it compiles + tests pass**

Run: `cd src-tauri && cargo test -p app 2>&1 | grep "test result"`
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(r4): composition root emits throttled task.log {task_id,delta} per worker"
```

---

## Task 7: Frontend IPC — `task.log` type + listener

**Files:**
- Modify: `src/ipc/runtime.ts`

- [ ] **Step 1: Add the `TaskLog` type + `onTaskLog` listener**

In `src/ipc/runtime.ts`, add at the top after the existing import:

```ts
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
```

At the end of the file, add:

```ts
/// Display-only live-log fragment for one running task. `delta` is a prose
/// fragment streamed from the worker as it runs; the authoritative settled state
/// still arrives via `task.changed`. Purely for feel — accumulate per task_id.
export interface TaskLog {
  task_id: string;
  delta: string;
}

/// Subscribe to backend display-only worker log fragments (R4).
export async function onTaskLog(cb: (log: TaskLog) => void): Promise<UnlistenFn> {
  return await listen<TaskLog>("task.log", (e) => cb(e.payload));
}
```

- [ ] **Step 2: Verify it type-checks**

Run: `cd /Users/tim/projects/agent-bus-app && bun run build 2>&1 | tail -15` (run later in Task 9; for now just ensure no syntax error by eye). No standalone test for this file.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/runtime.ts
git commit -m "feat(r4): frontend onTaskLog listener + TaskLog type"
```

---

## Task 8: `useTaskLog` hook — accumulate per-task live log

**Files:**
- Create: `src/hooks/useTaskLog.ts`
- Create: `src/hooks/useTaskLog.test.ts`

- [ ] **Step 1: Write the failing test**

Create `src/hooks/useTaskLog.test.ts`:

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";

// Capture the registered task.log callback so the test can push fragments.
let logCb: ((log: { task_id: string; delta: string }) => void) | undefined;
vi.mock("../ipc/runtime", () => ({
  onTaskLog: vi.fn(async (cb: (log: { task_id: string; delta: string }) => void) => {
    logCb = cb;
    return () => { logCb = undefined; };
  }),
}));

import { useTaskLog } from "./useTaskLog";

describe("useTaskLog", () => {
  beforeEach(() => { logCb = undefined; });

  it("accumulates fragments per task id", async () => {
    const { result } = renderHook(() => useTaskLog());
    // wait a tick for the async subscribe
    await act(async () => { await Promise.resolve(); });
    act(() => {
      logCb?.({ task_id: "T-1", delta: "chunk-a " });
      logCb?.({ task_id: "T-1", delta: "chunk-b" });
      logCb?.({ task_id: "T-2", delta: "other" });
    });
    expect(result.current.logFor("T-1")).toBe("chunk-a chunk-b");
    expect(result.current.logFor("T-2")).toBe("other");
    expect(result.current.logFor("T-3")).toBe("");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run src/hooks/useTaskLog.test.ts 2>&1 | tail -15`
Expected: FAIL — `./useTaskLog` cannot be resolved.

- [ ] **Step 3: Create the hook**

Create `src/hooks/useTaskLog.ts`:

```ts
import { useCallback, useEffect, useRef, useState } from "react";
import { onTaskLog } from "../ipc/runtime";

/// Accumulates display-only worker log fragments (`task.log`) into a per-task
/// buffer the CardDrawer's "live log" tab renders. The authoritative settled
/// state still arrives via `task.changed`; this is purely for live feel.
export function useTaskLog() {
  // A ref holds the canonical buffers (mutated on every fragment); a version
  // counter triggers re-render so `logFor` returns fresh text without rebuilding
  // a new object on every fragment.
  const buffers = useRef<Record<string, string>>({});
  const [, setVersion] = useState(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await onTaskLog((log) => {
        buffers.current[log.task_id] = (buffers.current[log.task_id] ?? "") + log.delta;
        setVersion((v) => v + 1);
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const logFor = useCallback((taskId: string): string => {
    return buffers.current[taskId] ?? "";
  }, []);

  return { logFor };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run src/hooks/useTaskLog.test.ts 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useTaskLog.ts src/hooks/useTaskLog.test.ts
git commit -m "feat(r4): useTaskLog hook accumulates per-task live-log fragments"
```

---

## Task 9: Wire the live log into the CardDrawer

**Files:**
- Modify: `src/App.tsx`

The CardDrawer already renders `logText` in its "live log" tab (`src/components/CardDrawer.tsx:124-139`). We just feed it the live buffer for the open task.

- [ ] **Step 1: Import and use `useTaskLog` in App**

In `src/App.tsx`, add the import after `useConversation`:

```ts
import { useTaskLog } from "./hooks/useTaskLog";
```

Inside `App()`, after `const { tasks, reload: reloadTasks } = useTasks();`, add:

```ts
  const liveLog = useTaskLog();
```

- [ ] **Step 2: Pass `logText` to `CardDrawer`**

In `src/App.tsx`, in the `<CardDrawer ... />` JSX (line 188), add the prop:

```tsx
          <CardDrawer
            task={openTask}
            artifactMarkdown={artifactMarkdown}
            logText={liveLog.logFor(openTask.id)}
            reviseTarget={reviseTargetFor(openTask)}
            onOpenArtifact={setLineagePath}
            onApprove={handleApprove}
            onRevise={handleRevise}
            onReject={handleReject}
          />
```

- [ ] **Step 3: Run the App tests + full vitest to confirm no regression**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run 2>&1 | tail -8`
Expected: all PASS (existing 218 + the new useTaskLog test = 219).

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat(r4): CardDrawer live-log tab renders streamed worker output"
```

---

## Task 10: Full verification

- [ ] **Step 1: cargo test workspace**

Run: `cd src-tauri && cargo test --workspace 2>&1 | grep -E "test result|error\["`
Expected: all `test result: ok`, no errors.

- [ ] **Step 2: cargo check workspace**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -5`
Expected: `Finished`.

- [ ] **Step 3: cargo clippy workspace (clean)**

Run: `cd src-tauri && cargo clippy --workspace --all-targets 2>&1 | grep -E "warning|error" | head -20`
Expected: no warnings/errors. Fix any clippy lint the new code introduced (e.g. needless borrows) before proceeding.

- [ ] **Step 4: vitest**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run 2>&1 | tail -8`
Expected: all PASS.

- [ ] **Step 5: frontend build**

Run: `cd /Users/tim/projects/agent-bus-app && bun run build 2>&1 | tail -10`
Expected: build succeeds (tsc + vite).

- [ ] **Step 6: Final commit if any clippy/build fixes were made**

```bash
git add -A
git commit -m "chore(r4): clippy/build fixes"
```

---

## Decisions

- **D1 — Mirror, don't share, the incremental parser.** `runners` gets its own `parse_stream_streaming` rather than importing one from `llm_chat`. They are separate ACL crates with different output idioms (`RunnerOutput` with verdict/artifact vs `ChatReply` prose). Sharing would couple two ACLs and leak each other's vocabulary. This matches C2's reasoning and (per the expected DDD ruling) is documented duplication with a doc-comment cross-reference. Recommended option, auto-selected.
- **D2 — `LogSink` is a `Fn(&str)` callback, not a channel/stream type.** Mirrors C2's `DeltaSink` exactly. Keeps the trait object-safe, keeps the ACL prose-only, and the composition root maps fragments to a Tauri event. No `task.log` idiom or stream-json shape crosses the `Runner` seam — only `&str`.
- **D3 — `invoke_stream` defaults to `invoke`.** Additive seam. `FakeRunner`'s old `new`/`always` callers, and any non-streaming runner, keep compiling and behaving identically. The streaming and non-streaming paths in `ClaudeCliRunner` share one `run_once` core so the final `RunnerOutput` can never drift.
- **D4 — The pool takes a `LogSinkFactory` (task_id → LogSink), not a single LogSink.** Each invocation gets a task-scoped sink so the composition root can tag every fragment with its `task_id` and concurrent workers never interleave. `None` = use non-streaming `invoke` (the runtime-only-test default). The factory lives in `PoolContext` alongside the other optional seams (`usage_sink`, `revision_reader`) — same pattern.
- **D5 — Settle/route is byte-for-byte unchanged.** The only edit in `process_one_claim` is choosing `invoke_stream` vs `invoke`; both return the identical `RunnerOutput`, and the verdict/artifact/usage/route code after it is untouched. The two-aggregate Task/WorkerPool model and the fork/join barrier are unaffected. Streaming is a display-only side channel.
- **D6 — Event name `task.log` with `{task_id, delta}`.** Parallel to C2's `conversation.delta` `{text, reset}`. We include `task_id` (not `reset`) because, unlike the single terminal conversation, many tasks stream concurrently and the frontend must route each fragment to the right card. No per-step `reset` is needed: a task's log is a single growing stream (one invocation per claim); a fresh claim/attempt is a new `task.changed` the frontend already reloads on, and the buffer simply continues — acceptable for v1.1 (the live-log tab shows the latest run's accumulated output). Recommended; revisit if reattempts should clear the log (out of scope).
- **D7 — Throttle/coalesce at the composition root** (50ms / 80 chars), identical thresholds to the conversation.delta sink, via the pure `TaskLogBuffer` so the flush shape is unit-testable without a Tauri handle. The runner/pool forward raw fragments; only the root batches them into events.
- **D8 — Reuse the existing CardDrawer "live log" tab.** It already renders a `logText` prop with an empty-state message; we feed it the live buffer. No new component, no gold-plating — the impeccable pass can polish the tab later.
- **D9 — `useTaskLog` keeps buffers in a ref + a version counter.** Avoids rebuilding a fresh object map on every fragment (which would re-render every consumer); `logFor(taskId)` reads the ref. v1.1-adequate; an unbounded buffer is acceptable for the live-log feature (one project session) and can be capped in a later hardening pass.
- **D10 — No `SCHEMA_VERSION` / persistence change.** The live log is ephemeral display state; nothing is persisted. `RunnerOutput.final_text` already carries the settled full text for the post-hoc log; streaming just fills the tab earlier. No migration.

## Caveat — headless live path

There is no live `claude` binary in this environment and we never run a blocking GUI command. The incremental parser, the `invoke_stream` delta forwarding, the per-task pool forwarding, and the coalescing buffer are all covered by fakes/fixtures (Tasks 1–6, 8). The end-to-end real-subprocess streaming path (claude stdout → Tauri `task.log` event → React) is **structural-only / not executed headless**; it is wired exactly parallel to C2's proven chat path. Note this in the report.
