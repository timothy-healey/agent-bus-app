# Live Worker View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the operator click any running worker — including the generator — and watch its live `claude` output AND thinking stream, with cards that read in plain language (description + slug, not a raw `T-…` id).

**Architecture:** Five seams, all additive. (A) The Runners/llm_chat stream-json parsers tag each delta `Output|Thinking`; `LogSink` becomes `Box<dyn Fn(&LogDelta)>`; the composition root coalesces per kind and emits `task-log {task_id, delta, kind}`. (B) `useTaskLog` accumulates ordered tagged segments; CardDrawer renders output as prose, thinking dimmed/italic. (C) `generate_once` runs under a stable `gen:<run_id>:<source-stage>` id and returns a status the worker loop emits as a dot-free `generator-status` event; the frontend renders a transient clickable source card and resolves a synthetic Task for the `gen:` id. (E) The output contract gains `DESCRIPTION:`; `parse_items` captures it; the engine stores it on the work-item `topic`; cards lead with the description, show the slug as secondary, and drop the raw `task.id` headline.

**Tech Stack:** Tauri 2 + Rust (Cargo workspace under `src-tauri/`: crates `runners`, `llm_chat`, `runtime`, `app`), React 18 + TypeScript + Vite (`src/`), vitest. macOS. Tauri event names contain NO dots (hyphenate / use `:` only where shown).

**Conventions reminder:** strict TDD (failing test first, run it red, minimal impl, run green, commit). `runtime` stays Tauri-free (the engine returns status; the loop in `app` emits the event). Live `claude` stays structural-only — every test uses a fixture / `FakeRunner`. Run Rust gates from `src-tauri/` (`cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build`) and frontend gates from the repo root (`npx vitest run`, `npx tsc --noEmit`).

---

## Section A — Capture thinking (Runners + llm_chat ACL)

The parsers today forward only `assistant`→`text` blocks as a prose `String`. We introduce a tagged `LogDelta`, forward `thinking` blocks too, and keep only `Output` in the accumulated text used for verdict/item parsing.

### Task A1: Introduce the tagged `LogDelta` type (runners)

**Files:**
- Modify: `src-tauri/runners/src/output.rs` (add `LogKind`/`LogDelta`; change the `LogSink` alias at `output.rs:144`)
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test.** Add to `runners/src/output.rs` tests:

```rust
    #[test]
    fn log_delta_carries_kind_and_text() {
        let out = LogDelta { kind: LogKind::Output, text: "hi".into() };
        let think = LogDelta { kind: LogKind::Thinking, text: "hmm".into() };
        assert_eq!(out.kind, LogKind::Output);
        assert_eq!(think.kind, LogKind::Thinking);
        assert_ne!(out.kind, think.kind);
        assert_eq!(out.text, "hi");
    }
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p runners log_delta_carries_kind_and_text` → FAIL (`cannot find type LogDelta`).

- [ ] **Step 3: Minimal impl.** In `runners/src/output.rs`, replace the `LogSink` block (currently `output.rs:138-144`) with:

```rust
/// Which channel a streamed fragment belongs to. `Output` is the model's visible
/// prose (accumulated into the final text used for verdict/item parsing);
/// `Thinking` is reasoning the model emits in `thinking` blocks — forwarded for
/// live display only, NEVER accumulated into the parsed text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    Output,
    Thinking,
}

/// A tagged display-only log fragment. The streaming worker path forwards each
/// fragment here as it parses, for live-log display. NO stream-json idiom, CLI
/// flag, or event name crosses the ACL through it — the composition root maps
/// fragments to whatever UI event it likes. Mirrors `llm_chat`'s tagged delta
/// (separate ACL crate, by design — vet F2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogDelta {
    pub kind: LogKind,
    pub text: String,
}

/// A display-only log sink. The streaming worker path forwards each tagged
/// fragment here as it parses. Deliberately a plain `&LogDelta` callback: the
/// composition root coalesces per kind and emits whatever UI event it likes.
pub type LogSink = Box<dyn Fn(&LogDelta) + Send + Sync>;
```

- [ ] **Step 4: Run it green.** `cd src-tauri && cargo test -p runners log_delta_carries_kind_and_text` → PASS. (The crate will NOT fully build yet — callers of the old `Fn(&str)` sink break; A2 fixes them. That is expected; only run this single test here.)

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/src/output.rs
git commit -m "feat(runners): introduce tagged LogDelta/LogKind for the log sink"
```

### Task A2: `StreamAccumulator::feed` emits tagged deltas; thinking excluded from final text

**Files:**
- Modify: `src-tauri/runners/src/stream_json.rs` (`feed` at `stream_json.rs:29-91`; `parse_stream_streaming` at `:318-337`; the `on_delta` callback type)
- Modify: `src-tauri/runners/src/output.rs` (the `default_invoke_stream_delegates_to_invoke_with_no_deltas` test sink at `:257`)
- Modify: `src-tauri/runners/src/fake.rs` (the `LogSink` builders in tests at `:136`, `:149`, and the `with_deltas` forwarding at `:66-70`)
- Modify: `src-tauri/runners/src/claude_cli.rs` (`run_once` forwarder at `:82-98`, `invoke`/`invoke_stream` at `:109-120`, test sink at `:217`, `:230`)
- Test: `src-tauri/runners/src/stream_json.rs`

- [ ] **Step 1: Write the failing test.** Add to `runners/src/stream_json.rs` tests (after `streaming_parse_forwards_only_assistant_prose_and_returns_same_output`):

```rust
    #[test]
    fn feed_tags_text_as_output_and_thinking_as_thinking_and_excludes_thinking_from_final() {
        let raw = concat!(
            r#"{"type":"system","subtype":"init","model":"m"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"let me reason"},{"type":"text","text":"VERDICT: approve"}]}}"#, "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"result":"VERDICT: approve","usage":{"input_tokens":1,"output_tokens":1}}"#
        );
        let mut seen: Vec<LogDelta> = vec![];
        let out = parse_stream_streaming(raw, "m", &mut |d: &LogDelta| seen.push(d.clone())).unwrap();
        // both channels were forwarded, in document order (thinking before text)
        assert_eq!(seen, vec![
            LogDelta { kind: LogKind::Thinking, text: "let me reason".into() },
            LogDelta { kind: LogKind::Output, text: "VERDICT: approve".into() },
        ]);
        // the verdict parsed from OUTPUT only; thinking never reached final_text
        assert_eq!(out.verdict, agent_bus_core::Verdict::Approve);
        assert!(!out.final_text.contains("let me reason"));
    }
```

Also update the existing `feed_returns_prose_delta_for_assistant_event_only` (at `:393`) — `feed` now returns `Vec<LogDelta>`:

```rust
    #[test]
    fn feed_returns_output_delta_for_assistant_text_only() {
        let mut acc = StreamAccumulator::new();
        let sys: Value = serde_json::from_str(r#"{"type":"system","subtype":"init","model":"m"}"#).unwrap();
        let asst: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello "}]}}"#,
        ).unwrap();
        let res: Value = serde_json::from_str(r#"{"type":"result","subtype":"success","result":"hello world"}"#).unwrap();
        assert!(acc.feed(&sys).unwrap().is_empty());
        assert_eq!(acc.feed(&asst).unwrap(), vec![LogDelta { kind: LogKind::Output, text: "hello ".into() }]);
        assert!(acc.feed(&res).unwrap().is_empty());
    }
```

And update `streaming_parse_forwards_only_assistant_prose_and_returns_same_output` (at `:483`) to collect `LogDelta`s and compare their `.text` for the Output channel:

```rust
    #[test]
    fn streaming_parse_forwards_only_assistant_prose_and_returns_same_output() {
        let mut deltas: Vec<LogDelta> = vec![];
        let out = parse_stream_streaming(SAMPLE, "fallback-model", &mut |d| deltas.push(d.clone())).unwrap();
        let plain = parse_stream(SAMPLE, "fallback-model").unwrap();
        assert_eq!(out, plain);
        let texts: Vec<String> = deltas.iter().filter(|d| d.kind == LogKind::Output).map(|d| d.text.clone()).collect();
        assert_eq!(texts, vec![
            "Analysing the repository.\n".to_string(),
            "VERDICT: approve\nARTIFACT: artifacts/analyses/T-1-v1.md".to_string(),
        ]);
    }
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p runners feed_tags_text` → FAIL (type mismatch; `feed` returns `String`).

- [ ] **Step 3: Minimal impl — `feed`.** In `runners/src/stream_json.rs`, change `feed`'s signature and the `assistant` arm. Replace the `feed` body's `let mut delta = String::new();` with `let mut deltas: Vec<crate::output::LogDelta> = Vec::new();`, change the return type to `Result<Vec<crate::output::LogDelta>, RunnerError>`, and rewrite the `assistant` arm content loop:

```rust
            "assistant" => {
                if let Some(content) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        match block.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                    self.text.push_str(t);
                                    deltas.push(crate::output::LogDelta {
                                        kind: crate::output::LogKind::Output,
                                        text: t.to_string(),
                                    });
                                }
                            }
                            Some("thinking") => {
                                if let Some(t) = block.get("thinking").and_then(|t| t.as_str()) {
                                    // Thinking is forwarded for display ONLY — never
                                    // pushed into self.text (verdict/item parsing).
                                    deltas.push(crate::output::LogDelta {
                                        kind: crate::output::LogKind::Thinking,
                                        text: t.to_string(),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.add_usage(u);
                }
            }
```

(Delete the old `self.text.push_str(&delta);` line — text now accumulates inline above.) Change the final `Ok(delta)` to `Ok(deltas)`. Update the doc-comment on `feed` to say it returns the tagged deltas this event produced.

- [ ] **Step 4: Update `parse_stream_streaming`.** Change its callback param and forwarding:

```rust
pub fn parse_stream_streaming(
    raw: &str,
    model: &str,
    on_delta: &mut dyn FnMut(&crate::output::LogDelta),
) -> Result<RunnerOutput, RunnerError> {
    let mut acc = StreamAccumulator::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| RunnerError::Other(format!("bad stream-json line: {e}")))?;
        for d in acc.feed(&v)? {
            on_delta(&d);
        }
    }
    acc.finish(model)
}
```

In `parse_stream` (`:297`), change `let _ = acc.feed(&v)?;` stays valid (returns a Vec now; `let _ =` is fine).

- [ ] **Step 5: Update `claude_cli.rs`.** Change `run_once`'s `forward` param to `&mut dyn FnMut(&crate::output::LogDelta)` and pass it straight into `parse_stream_streaming`. Update `invoke` to `self.run_once(req, &mut |_d: &crate::output::LogDelta| {})` and `invoke_stream` to `let mut forward = |d: &crate::output::LogDelta| sink(d); self.run_once(req, &mut forward)`. In its two tests, change `Box::new(move |d: &str| ...)` sinks to `Box::new(move |d: &LogDelta| s.lock().unwrap().push(d.text.clone()))` and the noop to `Box::new(|_d: &LogDelta| {})`; add `use crate::output::{LogDelta, LogKind};` to the test module. The `invoke_stream_forwards_prose_deltas_and_returns_same_output` assertion stays `vec!["Analysing.\nVERDICT: approve".to_string()]` (it pushes `.text`).

- [ ] **Step 6: Update `fake.rs`.** In `invoke_stream` (`:66-70`) forward each scripted string as an Output delta:

```rust
        if let Some(call_deltas) = self.deltas.get(idx) {
            for d in call_deltas {
                sink(&crate::output::LogDelta {
                    kind: crate::output::LogKind::Output,
                    text: d.clone(),
                });
            }
        }
```

In the three fake.rs tests, change `Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()))` to `Box::new(move |d: &LogDelta| s.lock().unwrap().push(d.text.clone()))` and add `use crate::output::LogDelta;` per test.

- [ ] **Step 7: Update `output.rs` test sink.** At `:257`, change to `Box::new(move |d: &LogDelta| s.lock().unwrap().push(d.text.clone()))` and add `use` / reference `LogDelta` (already in `super::*`).

- [ ] **Step 8: Run it green.** `cd src-tauri && cargo test -p runners` → PASS. Then `cargo clippy -p runners --all-targets -- -D warnings` → clean.

- [ ] **Step 9: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/src/stream_json.rs src-tauri/runners/src/output.rs src-tauri/runners/src/fake.rs src-tauri/runners/src/claude_cli.rs
git commit -m "feat(runners): feed emits tagged LogDeltas; thinking excluded from parsed text"
```

### Task A3: Mirror the thinking capture in `llm_chat`

The llm_chat parser mirrors runners by design (separate ACL). The chat path uses its own `DeltaSink` (prose `&str`) for the terminal, which we keep — but the `feed` must still skip `thinking` blocks from `self.text` and not crash on them. The terminal already shows a thinking indicator (LF22), so chat does NOT need a tagged channel; the spec's mirror test only asserts thinking is excluded from the final reply text and that a thinking block does not break parsing.

**Files:**
- Modify: `src-tauri/llm_chat/src/stream_json.rs` (the `assistant` arm at `:54-67`)
- Test: same file

- [ ] **Step 1: Write the failing test.** Add to `llm_chat/src/stream_json.rs` tests:

```rust
    #[test]
    fn thinking_blocks_are_excluded_from_the_chat_reply_text() {
        let raw = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s","model":"m"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"reasoning"},{"type":"text","text":"hello"}],"usage":{"output_tokens":1}}}"#, "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"result":"hello","usage":{"output_tokens":1},"session_id":"s"}"#
        );
        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let (reply, _s) = parse_chat_stream_streaming(raw, "m", &mut |d: &str| seen.lock().unwrap().push(d.to_string())).unwrap();
        assert_eq!(reply.text, "hello");
        // only the visible prose was streamed to the terminal; reasoning excluded
        assert_eq!(*seen.lock().unwrap(), vec!["hello".to_string()]);
    }
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p llm_chat thinking_blocks_are_excluded` → currently this likely PASSES by accident (thinking has no `text` field so the existing loop ignores it). Verify: if it passes, the behavior is already correct — keep the test as a regression guard and skip Step 3. If it FAILS (thinking field leaked), proceed.

- [ ] **Step 3: Minimal impl (only if red).** The existing loop at `:60-66` already matches `Some("text")` only, so thinking is naturally dropped. No change needed unless red; if red, add an explicit `// thinking blocks are reasoning, never part of the chat reply` comment and ensure the match ignores non-text. (No tagged channel for chat — the terminal's prose `DeltaSink` is unchanged.)

- [ ] **Step 4: Run it green.** `cd src-tauri && cargo test -p llm_chat` → PASS.

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/llm_chat/src/stream_json.rs
git commit -m "test(llm_chat): regression guard — thinking blocks excluded from chat reply"
```

### Task A4: `make_task_log_sink` coalesces per kind and emits `task-log` with a `kind` field

The composition-root sink (`app/src/lib.rs:203` `make_task_log_sink`) currently takes `Fn(&str)` fragments into one `TaskLogBuffer` and emits `{task_id, delta}`. It must now accept `&LogDelta`, keep a buffer PER kind, and emit `{task_id, delta, kind: "output"|"thinking"}`. The existing `TaskLogBuffer` (`lib.rs:165-197`) coalesces by task id; we reuse it per kind.

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (`make_task_log_sink` at `:203-219`; reuse `TaskLogBuffer` at `:165`)
- Test: `src-tauri/app/src/lib.rs` (`#[cfg(test)]`, near the existing buffer tests at `:1552`)

- [ ] **Step 1: Write the failing test.** Add near the existing `buffer_coalesces_...` tests in `app/src/lib.rs`. We test the pure coalescing-per-kind helper (no Tauri handle in unit tests):

```rust
    #[test]
    fn per_kind_buffers_coalesce_independently_and_tag_their_kind() {
        use runners::output::{LogDelta, LogKind};
        // Two buffers keyed by kind; pushing into each and force-flushing yields
        // one emit per kind, each tagged.
        let mut out_buf = TaskLogBuffer::new("T-1".into());
        let mut think_buf = TaskLogBuffer::new("T-1".into());
        for d in [
            LogDelta { kind: LogKind::Thinking, text: "rea".into() },
            LogDelta { kind: LogKind::Thinking, text: "soning".into() },
            LogDelta { kind: LogKind::Output, text: "ans".into() },
        ] {
            match d.kind {
                LogKind::Output => out_buf.push(&d.text),
                LogKind::Thinking => think_buf.push(&d.text),
            }
        }
        let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let e = emitted.clone();
        out_buf.force_flush(&mut |t, d| e.lock().unwrap().push((t.into(), d.into())));
        let e2 = emitted.clone();
        think_buf.force_flush(&mut |t, d| e2.lock().unwrap().push((t.into(), d.into())));
        assert_eq!(
            *emitted.lock().unwrap(),
            vec![("T-1".into(), "ans".into()), ("T-1".into(), "reasoning".into())]
        );
    }
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p app per_kind_buffers_coalesce` → this exercises existing `TaskLogBuffer`; it should compile and PASS once `LogDelta` is importable (it is, from A1). If it passes immediately it confirms the buffer reuse; the real wiring change is verified by the next step's compile + the kind in the payload. Keep this as the coalescing regression.

- [ ] **Step 3: Rewrite `make_task_log_sink`.** Replace `lib.rs:203-219` with a per-kind pair of buffers and a `kind` field in the payload:

```rust
fn make_task_log_sink(handle: tauri::AppHandle) -> Arc<runtime::log_sink::LogSinkFactory> {
    use runners::output::{LogDelta, LogKind};
    Arc::new(move |task_id: &str| -> runners::output::LogSink {
        use std::sync::Mutex;
        // One coalescing buffer per kind so output and thinking never interleave
        // within a single emitted fragment; each emit carries its kind.
        let out_buf = Arc::new(Mutex::new(TaskLogBuffer::new(task_id.to_string())));
        let think_buf = Arc::new(Mutex::new(TaskLogBuffer::new(task_id.to_string())));
        let handle = handle.clone();
        Box::new(move |d: &LogDelta| {
            let (buf, kind_str) = match d.kind {
                LogKind::Output => (&out_buf, "output"),
                LogKind::Thinking => (&think_buf, "thinking"),
            };
            let mut b = buf.lock().unwrap();
            b.push(&d.text);
            let handle = handle.clone();
            b.flush_if_due(&mut |tid: &str, delta: &str| {
                let _ = handle.emit(
                    crate::events::TASK_LOG,
                    serde_json::json!({ "task_id": tid, "delta": delta, "kind": kind_str }),
                );
            });
        })
    })
}
```

(Note: `flush_if_due` only emits when the buffer crosses its threshold; that pre-existing throttling behavior is preserved per kind. The `task-log` event name is unchanged and remains dot-free.)

- [ ] **Step 4: Run it green + build.** `cd src-tauri && cargo test -p app per_kind_buffers_coalesce` → PASS; `cargo build -p app` → builds.

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): task-log sink coalesces per kind and emits a kind field"
```

---

## Section E — Card identity & content (the engine half; LF32)

Do E before B/C on the frontend so the synthetic-Task and card-rendering work can lean on `topic` being populated. The output contract gains `DESCRIPTION:`, `parse_items` captures it, and the engine writes it to the work-item `topic`.

### Task E1: `OutputItem` gains `description`; `parse_items` captures `DESCRIPTION:`

**Files:**
- Modify: `src-tauri/runners/src/stream_json.rs` (`OutputItem` at `:154-164`, `parse_items` at `:177-225`)
- Test: same file

- [ ] **Step 1: Write the failing test.** Add to `runners/src/stream_json.rs` tests:

```rust
    #[test]
    fn parse_items_captures_description_per_item() {
        let text = "\
KEY: lwv-a1-logdelta-seam
DESCRIPTION: Tag stream deltas as output or thinking
ARTIFACT: artifacts/specs/a1.md
KEY: lwv-a2-no-desc
ARTIFACT: artifacts/specs/a2.md";
        let items = parse_items(text);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].description.as_deref(), Some("Tag stream deltas as output or thinking"));
        assert_eq!(items[1].description, None);
    }
```

Also extend the existing `parse_items_reads_a_list_of_n_items` (`:411`) struct literals to include `description: None` for each `OutputItem { .. }` (and `parse_items_is_best_effort...` at `:449`).

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p runners parse_items_captures_description` → FAIL (no field `description`).

- [ ] **Step 3: Minimal impl.** Add the field to `OutputItem`:

```rust
pub struct OutputItem {
    pub key: String,
    pub artifact_path: Option<String>,
    pub verdict: Option<Verdict>,
    /// A short (<=~80 char) plain-language summary the agent wrote alongside the
    /// item (the `DESCRIPTION:` line). `None` when absent (legacy/transform items).
    pub description: Option<String>,
}
```

Update all three `OutputItem { .. }` constructions in `parse_items` (the `KEY:` arm at `:197`, the `ARTIFACT:` `get_or_insert_with` at `:204`, the `VERDICT:` one at `:213`) to add `description: None`. Add a new arm BEFORE the `ARTIFACT:` arm:

```rust
        } else if let Some(rest) = line.strip_prefix("DESCRIPTION:") {
            let d = rest.trim();
            let item = current.get_or_insert_with(|| OutputItem {
                key: String::new(),
                artifact_path: None,
                verdict: None,
                description: None,
            });
            if !d.is_empty() {
                item.description = Some(d.to_string());
            }
        }
```

- [ ] **Step 4: Run it green.** `cd src-tauri && cargo test -p runners parse_items` → PASS.

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/src/stream_json.rs
git commit -m "feat(runners): parse_items captures the DESCRIPTION line into OutputItem.description"
```

### Task E2: `output_contract` instructs the agent to emit `DESCRIPTION:`

**Files:**
- Modify: `src-tauri/runners/src/stream_json.rs` (`output_contract` at `:242-293`)
- Test: same file

- [ ] **Step 1: Write the failing test.** Add:

```rust
    #[test]
    fn output_contract_tells_every_role_to_emit_a_description() {
        for role in ["generator", "reviewer", "producer"] {
            let c = output_contract(role, "${project}/artifacts/x", &[]);
            assert!(c.contains("DESCRIPTION:"), "role {role} contract must request a DESCRIPTION line");
        }
    }
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p runners output_contract_tells_every_role` → FAIL.

- [ ] **Step 3: Minimal impl.** In `output_contract`, after the `KEY:` line push (`:250`) add:

```rust
    s.push_str("  DESCRIPTION: <a short (<=80 char) plain-language summary of this item>\n");
```

- [ ] **Step 4: Run it green.** `cd src-tauri && cargo test -p runners output_contract` → PASS (the existing role tests still pass — none assert the absence of `DESCRIPTION:`).

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runners/src/stream_json.rs
git commit -m "feat(runners): output contract requests a DESCRIPTION line per item"
```

### Task E3: A de-kebab fallback helper in the engine

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` (add a pure helper near `artifact_dir`/`role_str`; find them with `grep -n "fn artifact_dir\|fn role_str" src-tauri/runtime/src/engine.rs`)
- Test: `src-tauri/runtime/src/engine.rs`

- [ ] **Step 1: Write the failing test.** Add to the engine's `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn topic_for_item_prefers_description_then_dekebabs_the_key() {
        assert_eq!(topic_for_item(Some("Tag deltas by kind"), "lwv-a1-seam"), "Tag deltas by kind");
        assert_eq!(topic_for_item(None, "lwv-a1-logdelta-seam"), "Lwv a1 logdelta seam");
        assert_eq!(topic_for_item(Some("   "), "only-key"), "Only key");
        assert_eq!(topic_for_item(None, ""), "");
    }
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p runtime topic_for_item` → FAIL (not defined).

- [ ] **Step 3: Minimal impl.** Add to `runtime/src/engine.rs` (module level, near the other pure helpers):

```rust
/// The human-readable topic for a produced work-item (LF32): the agent's
/// `DESCRIPTION:` when present, else a de-kebabbed title derived from the slug
/// (`item_key`) — hyphens/underscores to spaces, first letter capitalised. Pure.
pub fn topic_for_item(description: Option<&str>, item_key: &str) -> String {
    if let Some(d) = description {
        let d = d.trim();
        if !d.is_empty() {
            return d.to_string();
        }
    }
    let spaced = item_key.replace(['-', '_'], " ");
    let spaced = spaced.trim();
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
```

- [ ] **Step 4: Run it green.** `cd src-tauri && cargo test -p runtime topic_for_item` → PASS.

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runtime/src/engine.rs
git commit -m "feat(runtime): topic_for_item — description-or-dekebabbed-slug helper"
```

### Task E4: The engine stores the topic on committed work-items

`Task::work_item` (`task.rs:139`) sets `topic: String::new()`. Rather than change its signature (it has many call sites), set `child.topic` after construction at the two commit sites that produce displayable items: the transformer commit (`engine.rs:375-391`) and the generator commit (`engine.rs:944-954`).

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` (`transform_once` commit at `:360-392`; `generate_once` commit loop at `:931-956`)
- Test: `src-tauri/runtime/src/engine.rs`

- [ ] **Step 1: Write the failing test.** The engine has existing FakeRunner+in-memory-pool integration tests (search `grep -n "async fn invoke_records_a_started" src-tauri/runtime/src/engine.rs` for the harness pattern). Add a generator test asserting the committed child carries the description as its topic. Model it on the existing generator tests (search `grep -n "generate_once" src-tauri/runtime/src/engine.rs` test module). Concretely add:

```rust
    #[tokio::test]
    async fn generated_work_item_carries_the_description_as_its_topic() {
        // A FakeRunner whose generator pass emits one item WITH a DESCRIPTION.
        let final_text = "KEY: lwv-x\nDESCRIPTION: Investigate the seam\nARTIFACT: artifacts/research/lwv-x.md";
        let out = RunnerOutput {
            verdict: Verdict::Approve,
            artifact_path: Some("artifacts/research/lwv-x.md".into()),
            final_text: final_text.into(),
            usage: Default::default(),
        };
        let ctx = /* build the in-memory EngineContext exactly as the neighbouring
                     generate_once tests do, with FakeRunner::always(out) */ unimplemented!();
        let source = /* the source team from the test pipeline */ unimplemented!();
        let outcome = generate_once(&ctx, &source).await.unwrap();
        assert!(matches!(outcome, StepOutcome::Generated { .. }));
        // the committed child task is queued downstream with topic = description
        let child = ctx.tasks.claim_next_for_stage(/* downstream stage */ "spec", now_unix()).await.unwrap().unwrap();
        assert_eq!(child.topic, "Investigate the seam");
        assert_eq!(child.item_key.as_deref(), Some("lwv-x"));
    }
```

NOTE to the implementer: replace the `unimplemented!()` placeholders by copying the exact `EngineContext`/test-pipeline setup from the nearest existing `generate_once` test in the same module (do not invent a new harness). The asserted behavior — `child.topic == "Investigate the seam"` — is the load-bearing part.

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p runtime generated_work_item_carries_the_description` → FAIL (topic empty).

- [ ] **Step 3: Minimal impl — generator.** In `generate_once`'s commit loop, the item is found at `:939-943`. Capture its description and set the topic on the child. Replace the child construction block (`:939-954`) with:

```rust
        let emitted = items.iter().find(|i| &i.key == key);
        let artifact = emitted
            .and_then(|i| i.artifact_path.clone())
            .or_else(|| Some(artifact_path(&source_team.id, key, 1)));
        let mut child = Task::work_item(
            run.project_id.clone(),
            ctx.pipeline.id.clone(),
            ctx.run_id.clone(),
            key.clone(),
            downstream.clone(),
            artifact,
            ctx.target_repo.as_ref().map(|p| p.to_string_lossy().into_owned()),
            now_unix(),
        );
        child.topic = topic_for_item(emitted.and_then(|i| i.description.as_deref()), key);
        ctx.tasks.insert(&child).await?;
        committed.push(key.clone());
```

- [ ] **Step 4: Minimal impl — transformer.** In `transform_once`, after building `child` (`:375-391`) and before `ctx.tasks.insert(&child).await?`, add:

```rust
        child.topic = topic_for_item(first.description.as_deref(), &produced_key);
```

(`child` is already `let mut child`. `first` is the `&items[0]` at `:360`; `produced_key` at `:361`.)

- [ ] **Step 5: Run it green.** `cd src-tauri && cargo test -p runtime` → PASS. `cargo clippy -p runtime --all-targets -- -D warnings` → clean.

- [ ] **Step 6: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runtime/src/engine.rs
git commit -m "feat(runtime): store the item description (or dekebabbed slug) on the work-item topic"
```

---

## Section C — Generator as a clickable worker (LF23/LF31)

The generator pass must run under a stable id `gen:<run_id>:<source-stage>` and report active/inactive so the loop can emit a dot-free `generator-status` event.

### Task C1: `generate_once` uses the `gen:` id for its invocation task_id

`generate_once` builds a transient `pass_task` (`engine.rs:894-903`) with a fresh `T-<uuid>` id, then calls `invoke(ctx, source_team, &pass_task, ...)` which uses `task.id.0` for the InvocationRequest `task_id` and the log-sink factory key. We must override that id to the stable `gen:` form so the live-log streams under it.

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` (`generate_once` pass_task at `:894-904`; add a stable-id helper)
- Test: `src-tauri/runtime/src/engine.rs`

- [ ] **Step 1: Write the failing test.** Add:

```rust
    #[test]
    fn generator_task_id_is_stable_per_run_and_source_stage() {
        assert_eq!(generator_task_id("R-7", "research"), "gen:R-7:research");
    }
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p runtime generator_task_id_is_stable` → FAIL.

- [ ] **Step 3: Minimal impl.** Add to `engine.rs`:

```rust
/// The stable invocation id a generator pass runs under (LF31): `gen:<run>:<stage>`.
/// The live-log sink keys on this, so the frontend's transient generator card can
/// open the same stream. Contains a `:` (Tauri-legal) but NO `.`. Pure.
pub fn generator_task_id(run_id: &str, source_stage: &str) -> String {
    format!("gen:{run_id}:{source_stage}")
}
```

Then in `generate_once`, after constructing `pass_task` (`:894-903`), override its id:

```rust
    let mut pass_task = Task::work_item(
        run.project_id.clone(),
        ctx.pipeline.id.clone(),
        ctx.run_id.clone(),
        String::new(),
        source_team.id.clone(),
        None,
        ctx.target_repo.as_ref().map(|p| p.to_string_lossy().into_owned()),
        now_unix(),
    );
    pass_task.id = crate::task::TaskId(generator_task_id(&ctx.run_id, &source_team.id));
    let items = invoke(ctx, source_team, &pass_task, system_prompt).await?;
```

(Confirm `TaskId`'s path with `grep -n "pub struct TaskId" src-tauri/runtime/src/task.rs`; it is a tuple newtype `TaskId(String)`.)

- [ ] **Step 4: Run it green.** `cd src-tauri && cargo test -p runtime` → PASS.

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runtime/src/engine.rs
git commit -m "feat(runtime): generator pass invokes under the stable gen:<run>:<stage> id"
```

### Task C2: `generate_once` reports whether a pass became active (engine returns status)

`runtime` stays Tauri-free: the engine cannot emit. The cleanest seam that does not disturb the existing `StepOutcome` consumers is a separate function the loop calls to learn the stable id + activity, OR returning the id alongside the outcome. Per the spec ("the engine returns the status; the loop emits"), expose a small helper the loop uses to derive the event payload from the outcome + ctx.

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` (add a pure classifier)
- Test: `src-tauri/runtime/src/engine.rs`

- [ ] **Step 1: Write the failing test.** Add:

```rust
    #[test]
    fn generator_pass_is_active_only_while_a_pass_is_in_flight() {
        // A pass that committed keys is no longer "in flight" once we observe the
        // outcome; activity is signalled by the loop BEFORE invoke and cleared
        // AFTER. The engine exposes the terminal (settled/dry) predicate so the
        // loop knows when to emit active:false.
        assert!(generator_pass_settled(&StepOutcome::Generated { keys: vec!["k".into()] }));
        assert!(generator_pass_settled(&StepOutcome::Retired));
        assert!(generator_pass_settled(&StepOutcome::Backpressure));
        assert!(generator_pass_settled(&StepOutcome::Braked));
    }
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p runtime generator_pass_is_active` → FAIL.

- [ ] **Step 3: Minimal impl.** Add to `engine.rs`:

```rust
/// Whether a generator step's outcome means the pass is no longer in flight (the
/// loop emits `active:false`). Every generator-step outcome is terminal for the
/// pass — the loop sets `active:true` before the step and clears it after, so this
/// is always true today; it exists as the single named predicate the loop reads
/// (keeps `runtime` Tauri-free — the loop does the emitting). Pure.
pub fn generator_pass_settled(_outcome: &StepOutcome) -> bool {
    true
}
```

- [ ] **Step 4: Run it green.** `cd src-tauri && cargo test -p runtime generator_pass_is_active` → PASS.

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runtime/src/engine.rs
git commit -m "feat(runtime): generator_pass_settled predicate for the loop's status emit"
```

### Task C3: Add the `generator-status` event name (Rust + TS contracts)

**Files:**
- Modify: `src-tauri/app/src/events.rs` (add `GENERATOR_STATUS`; include it in the charset guard array at the test)
- Modify: `src/ipc/events.ts` (add `generatorStatus`)
- Test: `src-tauri/app/src/events.rs` (existing `all_event_names_are_tauri_legal`); `src/ipc/events.test.ts` (existing guard)

- [ ] **Step 1: Write the failing test.** In `app/src/events.rs`, add `GENERATOR_STATUS` to the array in `all_event_names_are_tauri_legal` (`:30`). In `src/ipc/events.test.ts`, the existing guard iterates `EVENTS`; add nothing if it iterates values — instead assert the new name exists:

```ts
  it("exposes generator-status as a dot-free name", () => {
    expect(EVENTS.generatorStatus).toBe("generator-status");
    expect(EVENTS.generatorStatus).not.toContain(".");
  });
```

- [ ] **Step 2: Run it red.** `cd src-tauri && cargo test -p app all_event_names` → FAIL (const not defined); `cd /Users/tim/projects/agent-bus-app && npx vitest run src/ipc/events.test.ts` → FAIL.

- [ ] **Step 3: Minimal impl.** In `app/src/events.rs` add:

```rust
/// A generator (source) pass started or settled (LF31). Payload:
/// `{ run_id, stage, task_id, active }`. The frontend renders a transient
/// clickable source card while `active`.
pub const GENERATOR_STATUS: &str = "generator-status";
```

Add `GENERATOR_STATUS` to the test array. In `src/ipc/events.ts` add inside `EVENTS`:

```ts
  /// A generator (source) pass started/settled (LF31). Payload {run_id, stage, task_id, active}.
  generatorStatus: "generator-status",
```

- [ ] **Step 4: Run it green.** Both tests PASS.

- [ ] **Step 5: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/app/src/events.rs src/ipc/events.ts src/ipc/events.test.ts
git commit -m "feat: add the generator-status event to the Rust + TS event contracts"
```

### Task C4: The generator loop emits `generator-status` around the step

**Files:**
- Modify: `src-tauri/app/src/pipeline_activator.rs` (`spawn_generator_loop` at `:215-251`)
- Test: covered structurally by `cargo build` + the existing loop unit tests; no new Rust unit test (the loop is a spawned task — the emit is verified by the frontend hook test in C5 and manually by the operator).

- [ ] **Step 1: Wire the emit.** In `spawn_generator_loop`, the loop has `team`, `handle`, and builds `ctx` per poll. The `gen:` id is `engine::generator_task_id(&run.id, &team.id)`. Emit `active:true` before the step and `active:false` after observing the outcome. Replace the `match engine::generate_once(&ctx, &team).await { .. }` block (`:232-248`) with:

```rust
                let task_id = engine::generator_task_id(&run.id, &team.id);
                let _ = handle.emit(
                    crate::events::GENERATOR_STATUS,
                    serde_json::json!({ "run_id": run.id, "stage": team.id, "task_id": task_id, "active": true }),
                );
                let step = engine::generate_once(&ctx, &team).await;
                if let Ok(outcome) = &step {
                    if engine::generator_pass_settled(outcome) {
                        let _ = handle.emit(
                            crate::events::GENERATOR_STATUS,
                            serde_json::json!({ "run_id": run.id, "stage": team.id, "task_id": task_id, "active": false }),
                        );
                    }
                }
                match step {
                    Ok(StepOutcome::Generated { keys }) if !keys.is_empty() => {
                        let _ = handle.emit(crate::events::TASK_CHANGED, "generated");
                    }
                    Ok(_) => {
                        finish_and_emit(&ctx, &handle).await;
                        tokio::time::sleep(LOOP_IDLE_SLEEP).await;
                    }
                    Err(e) => {
                        eprintln!("app: generator loop step failed for `{}`: {e}", team.id);
                        // a failed step never leaves the card stuck "active":
                        let _ = handle.emit(
                            crate::events::GENERATOR_STATUS,
                            serde_json::json!({ "run_id": run.id, "stage": team.id, "task_id": engine::generator_task_id(&run.id, &team.id), "active": false }),
                        );
                        tokio::time::sleep(LOOP_IDLE_SLEEP).await;
                    }
                }
```

Add `use serde_json;` is unnecessary (the crate already uses `serde_json::json!` elsewhere). Confirm `engine::generator_task_id` / `engine::generator_pass_settled` are `pub` (C1/C2).

- [ ] **Step 2: Build.** `cd src-tauri && cargo build -p app` → builds. `cargo clippy -p app --all-targets -- -D warnings` → clean.

- [ ] **Step 3: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/app/src/pipeline_activator.rs
git commit -m "feat(app): generator loop emits generator-status around each pass"
```

### Task C5: `useActiveGenerators(runId)` hook

**Files:**
- Create: `src/hooks/useActiveGenerators.ts`
- Create: `src/hooks/useActiveGenerators.test.tsx`
- Modify: `src/ipc/runtime.ts` (add `GeneratorStatus` type + `onGeneratorStatus` listener, mirroring `onTaskLog` at `:163-171`)

- [ ] **Step 1: Write the failing test.** Create `src/hooks/useActiveGenerators.test.tsx`. Follow the existing hook-test pattern (search `ls src/hooks/*.test.*` for one; if none, use `@testing-library/react`'s `renderHook` + `act`). We drive it by mocking the listener. Concretely:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

// Capture the registered callback so the test can push events.
let cb: ((s: { run_id: string; stage: string; task_id: string; active: boolean }) => void) | null = null;
vi.mock("../ipc/runtime", () => ({
  onGeneratorStatus: (fn: typeof cb) => {
    cb = fn;
    return Promise.resolve(() => { cb = null; });
  },
}));

import { useActiveGenerators } from "./useActiveGenerators";

describe("useActiveGenerators", () => {
  beforeEach(() => { cb = null; });

  it("adds an active generator and clears it on active:false, scoped to the run", async () => {
    const { result } = renderHook(() => useActiveGenerators("R-1"));
    await waitFor(() => expect(cb).not.toBeNull());
    act(() => cb!({ run_id: "R-1", stage: "research", task_id: "gen:R-1:research", active: true }));
    await waitFor(() => expect(result.current).toHaveLength(1));
    expect(result.current[0]).toMatchObject({ stage: "research", task_id: "gen:R-1:research" });
    // an event for a different run is ignored
    act(() => cb!({ run_id: "R-2", stage: "research", task_id: "gen:R-2:research", active: true }));
    expect(result.current).toHaveLength(1);
    // active:false clears it
    act(() => cb!({ run_id: "R-1", stage: "research", task_id: "gen:R-1:research", active: false }));
    await waitFor(() => expect(result.current).toHaveLength(0));
  });

  it("returns nothing when no run is scoped", () => {
    const { result } = renderHook(() => useActiveGenerators(null));
    expect(result.current).toEqual([]);
  });
});
```

- [ ] **Step 2: Run it red.** `npx vitest run src/hooks/useActiveGenerators.test.tsx` → FAIL (module not found).

- [ ] **Step 3: Add the IPC listener.** In `src/ipc/runtime.ts`, after `onTaskLog` (`:171`), add:

```ts
/// A generator (source) pass started/settled (LF31). The transient board card
/// reads this. Mirrors the Rust `generator-status` payload.
export interface GeneratorStatus {
  run_id: string;
  stage: string;
  task_id: string;
  active: boolean;
}

export async function onGeneratorStatus(cb: (s: GeneratorStatus) => void): Promise<UnlistenFn> {
  return await listen<GeneratorStatus>(EVENTS.generatorStatus, (e) => cb(e.payload));
}
```

- [ ] **Step 4: Write the hook.** Create `src/hooks/useActiveGenerators.ts`:

```ts
import { useEffect, useState } from "react";
import { onGeneratorStatus, type GeneratorStatus } from "../ipc/runtime";

/// One currently-active generator pass — the transient source card the board shows.
export interface ActiveGenerator {
  stage: string;
  task_id: string;
}

/// Accumulates the generators currently mid-pass for `runId` (set on `active:true`,
/// cleared on `active:false`). Events for other runs are ignored. Returns `[]` when
/// no run is scoped. Keyed by `task_id` (the stable `gen:<run>:<stage>` id).
export function useActiveGenerators(runId: string | null): ActiveGenerator[] {
  const [active, setActive] = useState<Record<string, ActiveGenerator>>({});

  useEffect(() => {
    // A run switch resets the set so a stale generator never lingers.
    setActive({});
    if (!runId) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await onGeneratorStatus((s: GeneratorStatus) => {
        if (s.run_id !== runId) return;
        setActive((prev) => {
          const next = { ...prev };
          if (s.active) next[s.task_id] = { stage: s.stage, task_id: s.task_id };
          else delete next[s.task_id];
          return next;
        });
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, [runId]);

  return Object.values(active);
}
```

- [ ] **Step 5: Run it green.** `npx vitest run src/hooks/useActiveGenerators.test.tsx` → PASS. `npx tsc --noEmit` → clean.

- [ ] **Step 6: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/hooks/useActiveGenerators.ts src/hooks/useActiveGenerators.test.tsx src/ipc/runtime.ts
git commit -m "feat(ui): useActiveGenerators hook + onGeneratorStatus listener"
```

---

## Section B + E (frontend) — Live view & card identity

### Task B1: `TaskLog` gains `kind`; `useTaskLog` accumulates ordered tagged segments

**Files:**
- Modify: `src/ipc/runtime.ts` (`TaskLog` at `:163-166`)
- Modify: `src/hooks/useTaskLog.ts`
- Create/Modify: `src/hooks/useTaskLog.test.tsx` (check `ls src/hooks/useTaskLog.test.*`; create if absent)

- [ ] **Step 1: Write the failing test.** Create/extend `src/hooks/useTaskLog.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

let cb: ((l: { task_id: string; delta: string; kind: "output" | "thinking" }) => void) | null = null;
vi.mock("../ipc/runtime", () => ({
  onTaskLog: (fn: typeof cb) => { cb = fn; return Promise.resolve(() => { cb = null; }); },
}));

import { useTaskLog } from "./useTaskLog";

describe("useTaskLog tagged segments", () => {
  beforeEach(() => { cb = null; });

  it("accumulates ordered segments, coalescing consecutive same-kind deltas", async () => {
    const { result } = renderHook(() => useTaskLog());
    await waitFor(() => expect(cb).not.toBeNull());
    act(() => cb!({ task_id: "T-1", delta: "think a", kind: "thinking" }));
    act(() => cb!({ task_id: "T-1", delta: " think b", kind: "thinking" }));
    act(() => cb!({ task_id: "T-1", delta: "out a", kind: "output" }));
    act(() => cb!({ task_id: "T-1", delta: "think c", kind: "thinking" }));
    await waitFor(() => expect(result.current.segmentsFor("T-1")).toHaveLength(3));
    expect(result.current.segmentsFor("T-1")).toEqual([
      { kind: "thinking", text: "think a think b" },
      { kind: "output", text: "out a" },
      { kind: "thinking", text: "think c" },
    ]);
    // logFor still returns the OUTPUT-only flat text (back-compat for the state machine)
    expect(result.current.logFor("T-1")).toBe("out a");
    expect(result.current.segmentsFor("T-2")).toEqual([]);
  });
});
```

- [ ] **Step 2: Run it red.** `npx vitest run src/hooks/useTaskLog.test.tsx` → FAIL (no `segmentsFor`; `kind` missing).

- [ ] **Step 3: Update `TaskLog`.** In `src/ipc/runtime.ts`, change the interface:

```ts
export interface TaskLog {
  task_id: string;
  delta: string;
  /// Which channel the fragment belongs to (A): visible prose vs dimmed reasoning.
  kind: "output" | "thinking";
}
```

- [ ] **Step 4: Rewrite `useTaskLog`.** Replace `src/hooks/useTaskLog.ts`:

```ts
import { useCallback, useEffect, useRef, useState } from "react";
import { onTaskLog } from "../ipc/runtime";

/// One contiguous run of same-kind log text (B). An ordered list of these is what
/// the CardDrawer renders — output as prose, thinking dimmed/italic.
export interface LogSegment {
  kind: "output" | "thinking";
  text: string;
}

/// Accumulates display-only worker log fragments (`task-log`) into per-task ordered
/// tagged segments, coalescing consecutive same-kind deltas. `logFor` returns the
/// OUTPUT-only flat text (the live-log state machine keys off it); `segmentsFor`
/// returns the interleaved segments for rendering.
export function useTaskLog() {
  const buffers = useRef<Record<string, LogSegment[]>>({});
  const [, setVersion] = useState(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await onTaskLog((log) => {
        const segs = buffers.current[log.task_id] ?? (buffers.current[log.task_id] = []);
        const last = segs[segs.length - 1];
        if (last && last.kind === log.kind) last.text += log.delta;
        else segs.push({ kind: log.kind, text: log.delta });
        setVersion((v) => v + 1);
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const segmentsFor = useCallback((taskId: string): LogSegment[] => {
    return buffers.current[taskId] ?? [];
  }, []);

  const logFor = useCallback((taskId: string): string => {
    return (buffers.current[taskId] ?? [])
      .filter((s) => s.kind === "output")
      .map((s) => s.text)
      .join("");
  }, []);

  return { logFor, segmentsFor };
}
```

- [ ] **Step 5: Run it green.** `npx vitest run src/hooks/useTaskLog.test.tsx` → PASS. `npx tsc --noEmit` → clean (any other `onTaskLog` mock in the suite now needs `kind` — fix those if `tsc`/vitest flags them).

- [ ] **Step 6: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/ipc/runtime.ts src/hooks/useTaskLog.ts src/hooks/useTaskLog.test.tsx
git commit -m "feat(ui): useTaskLog accumulates ordered tagged log segments"
```

### Task B2: CardDrawer renders interleaved output/thinking; thinking dimmed/italic

**Files:**
- Modify: `src/components/CardDrawer.tsx` (props: add `logSegments`; the live-log render at `:241-268`; the `logState` derivation at `:122-132`)
- Modify: `src/App.tsx` (pass `logSegments={liveLog.segmentsFor(openTask.id)}` at `:413`)
- Test: `src/components/CardDrawer.test.tsx` (check `ls src/components/CardDrawer.test.*`; extend if present, else create)

- [ ] **Step 1: Write the failing test.** Add to the CardDrawer test suite (build a minimal `Task` per the existing tests' factory; reuse it):

```tsx
  it("renders output as prose and thinking dimmed with a marker", () => {
    render(
      <CardDrawer
        {...baseProps}
        task={{ ...baseTask, state: "running" }}
        logSegments={[
          { kind: "thinking", text: "let me reason" },
          { kind: "output", text: "the answer" },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /live log/i }));
    const out = screen.getByText("the answer");
    expect(out).toBeInTheDocument();
    const thinking = screen.getByText(/let me reason/);
    // thinking is tagged for distinct styling + carries the marker
    expect(thinking.closest("[data-log-kind='thinking']")).not.toBeNull();
    expect(screen.getByText(/thinking/i)).toBeInTheDocument();
  });
```

(Use the suite's existing `baseProps`/`baseTask`/`render`/`screen`/`fireEvent` imports; if the file is new, mirror another component test in `src/components/` for setup.)

- [ ] **Step 2: Run it red.** `npx vitest run src/components/CardDrawer.test.tsx` → FAIL (`logSegments` not a prop).

- [ ] **Step 3: Add the prop + types.** In `CardDrawerProps` add:

```ts
  /// Ordered tagged live-log segments (B). Output renders as prose, thinking
  /// dimmed/italic with a marker. When omitted, the drawer falls back to `logText`.
  logSegments?: { kind: "output" | "thinking"; text: string }[];
```

Destructure `logSegments = []` in the component signature.

- [ ] **Step 4: Derive state from segments; render them.** Keep `logText` back-compat: compute the output-only body from segments when present. Replace the `logBody` line (`:122`) with:

```ts
  const outputText = logSegments.length
    ? logSegments.filter((s) => s.kind === "output").map((s) => s.text).join("")
    : (logText ?? "");
  const logBody = outputText.trim();
```

The `logState` machine (`:123-132`) stays as-is (keyed on `logBody` + `task.state`). Replace the streaming/settled `<pre>` block (`:252-257`) so it renders segments inline when present:

```tsx
            {(logState === "streaming" || logState === "settled") && (
              <pre style={logBlock} data-testid="live-log-body">
                {logSegments.length
                  ? logSegments.map((s, i) =>
                      s.kind === "thinking" ? (
                        <span
                          key={i}
                          data-log-kind="thinking"
                          style={{ color: "var(--text-3)", fontStyle: "italic" }}
                        >
                          <span aria-hidden="true" style={{ opacity: 0.7 }}>thinking · </span>
                          {s.text}
                        </span>
                      ) : (
                        <span key={i} data-log-kind="output">{s.text}</span>
                      ),
                    )
                  : logBody}
                {logState === "streaming" && (
                  <span className="abp-pulse" style={{ color: "var(--running)" }}>▌</span>
                )}
              </pre>
            )}
```

(Tokenized colors only — `--text-3` for the dimmed thinking; no new colors. The `loading`/`error`/`empty` branches are unchanged, preserving those states.)

- [ ] **Step 5: Auto-scroll to the tail while streaming.** Add a ref + effect. Near the top of the component add `const logRef = useRef<HTMLPreElement>(null);` (import `useRef`), set `ref={logRef}` on the `<pre>`, and add:

```tsx
  useEffect(() => {
    if (tab === "live log" && logState === "streaming" && logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [tab, logState, logSegments]);
```

(Import `useEffect, useRef` from react — the file currently imports only `useState`.)

- [ ] **Step 6: Pass segments from App.** In `src/App.tsx` at the `<CardDrawer ... logText={liveLog.logFor(openTask.id)}` (`:413`), add:

```tsx
            logSegments={liveLog.segmentsFor(openTask.id)}
```

- [ ] **Step 7: Run it green.** `npx vitest run src/components/CardDrawer.test.tsx` → PASS. `npx tsc --noEmit` → clean.

- [ ] **Step 8: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/components/CardDrawer.tsx src/App.tsx src/components/CardDrawer.test.tsx
git commit -m "feat(ui): CardDrawer renders interleaved output/thinking with auto-scroll"
```

### Task E5: Card head + board card lead with description, show slug, drop raw task.id

**Files:**
- Modify: `src/components/CardDrawer.tsx` (head at `:157-166`)
- Modify: `src/components/BoardView.tsx` (`cardLabel` at `:24-26`)
- Modify: `src/components/ui/Card.tsx` (head row at `:73-85`)
- Test: `src/components/CardDrawer.test.tsx`, `src/components/BoardView.test.tsx` / `Card` test (check `ls src/components/*.test.* src/components/ui/*.test.*`)

- [ ] **Step 1: Write the failing tests.** CardDrawer head — the `T-…` id must be ABSENT from the head and the description + slug present:

```tsx
  it("leads the head with the description and shows the slug, not the raw task id", () => {
    render(
      <CardDrawer
        {...baseProps}
        task={{ ...baseTask, id: "T-abc-123", topic: "Investigate the seam", item_key: "lwv-a1-seam", current_stage: "spec", attempts: 2 }}
      />,
    );
    expect(screen.getByText("Investigate the seam")).toBeInTheDocument();
    expect(screen.getByText("lwv-a1-seam")).toBeInTheDocument();
    // the raw id is gone from the headline
    expect(screen.queryByText("T-abc-123")).not.toBeInTheDocument();
    // the stage · attempts line stays
    expect(screen.getByText(/spec · a2/)).toBeInTheDocument();
  });
```

For the board `cardLabel` (it currently returns `item_key || topic || id` — flip to topic-first):

```ts
  it("labels a card by its description (topic), falling back to the slug then id", () => {
    expect(cardLabel({ topic: "A clear title", item_key: "the-slug", id: "T-1" } as Task)).toBe("A clear title");
    expect(cardLabel({ topic: "", item_key: "the-slug", id: "T-1" } as Task)).toBe("the-slug");
    expect(cardLabel({ topic: "", item_key: "", id: "T-1" } as Task)).toBe("T-1");
  });
```

- [ ] **Step 2: Run them red.** `npx vitest run src/components/CardDrawer.test.tsx src/components/BoardView.test.tsx` → FAIL.

- [ ] **Step 3: CardDrawer head.** Replace the head's first inner rows (`:160-166`). Lead with the topic, show the slug as secondary mono metadata, drop `task.id`:

```tsx
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6, paddingRight: "var(--sp-7)" }}>
          <code style={{ color: "var(--text-3)", fontSize: 11, fontFamily: "var(--font-mono)" }}>
            {task.item_key ?? ""}
          </code>
          <span style={{ color: "var(--text-3)", fontSize: 11 }}>
            {task.current_stage} · a{task.attempts}
          </span>
        </div>
        <div style={{ fontSize: 14.5, color: "var(--text)" }}>
          {task.topic?.trim() || task.item_key?.trim() || ""}
        </div>
```

- [ ] **Step 4: BoardView `cardLabel`.** Replace `:24-26`:

```ts
export function cardLabel(task: Task): string {
  // LF32: lead with the human description (topic); fall back to the slug, then id.
  return task.topic?.trim() || task.item_key?.trim() || task.id;
}
```

- [ ] **Step 5: Card slug as secondary.** In `src/components/ui/Card.tsx`, the head shows `task.id` in accent (`:74-80`). Replace the `<code style={id}>…{task.id}</code>` with the slug (muted/mono), keeping the revising arrow:

```tsx
        <code style={{ color: "var(--text-3)", fontSize: 11, fontFamily: "var(--font-mono)" }}>
          {task.state === "revising" ? "↩ " : ""}
          {task.item_key ?? ""}
        </code>
```

(The `label` prop — the description — already renders as the `title` at `:81`. The `id` CSSProperties const at `:52` becomes unused; delete it to keep clippy/tsc/lint quiet.)

- [ ] **Step 6: Run green.** `npx vitest run src/components` → PASS. `npx tsc --noEmit` → clean.

- [ ] **Step 7: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/components/CardDrawer.tsx src/components/BoardView.tsx src/components/ui/Card.tsx src/components/CardDrawer.test.tsx src/components/BoardView.test.tsx
git commit -m "feat(ui): cards lead with the description, show the slug, drop the raw task id"
```

### Task C6: BoardView transient generator card; App resolves the synthetic Task

**Files:**
- Modify: `src/components/BoardView.tsx` (props: add `activeGenerators`; render a transient card in the source lane)
- Modify: `src/App.tsx` (call `useActiveGenerators(selectedRun?.id ?? null)`; pass to BoardView; resolve a synthetic `openTask` for a `gen:` id; default the drawer to the live-log tab for it)
- Modify: `src/components/CardDrawer.tsx` (accept an `initialTab` prop so a `gen:` card opens on live log; other tabs show their existing empty states)
- Test: `src/components/BoardView.test.tsx`, `src/components/CardDrawer.test.tsx`

- [ ] **Step 1: Write the failing test (BoardView).** Add:

```tsx
  it("shows a transient clickable generator card in the source lane while active", () => {
    const onOpenCard = vi.fn();
    render(
      <BoardView
        pipeline={pipelineWithSource}
        tasks={[]}
        hasRun
        onOpenCard={onOpenCard}
        activeGenerators={[{ stage: "research", task_id: "gen:R-1:research" }]}
      />,
    );
    const card = screen.getByText(/scanning/i);
    expect(card).toBeInTheDocument();
    fireEvent.click(card);
    expect(onOpenCard).toHaveBeenCalledWith("gen:R-1:research");
  });
```

(`pipelineWithSource` = a minimal pipeline whose source team id is `research`; mirror the existing BoardView test fixtures.)

- [ ] **Step 2: Run it red.** `npx vitest run src/components/BoardView.test.tsx` → FAIL.

- [ ] **Step 3: BoardView impl.** Add to `BoardViewProps`:

```ts
  /// Currently-active generator passes (LF31). The board renders a transient
  /// clickable card in each generator's source lane while its pass is in flight.
  activeGenerators?: { stage: string; task_id: string }[];
```

Destructure `activeGenerators = []`. Inside the lanes `.map`, for a `team` lane whose `l.id` matches an active generator's `stage`, render a transient card BEFORE its task cards:

```tsx
            {l.kind === "team" &&
              activeGenerators
                .filter((g) => g.stage === l.id)
                .map((g) => (
                  <div
                    key={g.task_id}
                    className="abp-card"
                    role="button"
                    onClick={() => onOpenCard(g.task_id)}
                    style={{
                      border: "1px solid var(--accent-bd)",
                      borderRadius: "var(--r-md)",
                      padding: "10px 12px",
                      marginBottom: 6,
                      cursor: "pointer",
                      background: "var(--accent-2)",
                      fontSize: 13,
                      color: "var(--text-2)",
                    }}
                  >
                    <span className="abp-pulse" aria-hidden="true">⟳ </span>
                    {l.label} · scanning…
                  </div>
                ))}
```

- [ ] **Step 4: Write the failing test (App synthetic Task) at the CardDrawer level.** Drive the smaller seam: CardDrawer should open on live log when `initialTab="live log"` and a synthetic running gen task is passed. Add to CardDrawer test:

```tsx
  it("opens on the live-log tab when initialTab is set (generator card)", () => {
    render(
      <CardDrawer
        {...baseProps}
        initialTab="live log"
        task={{ ...baseTask, id: "gen:R-1:research", state: "running", current_stage: "research" }}
        logSegments={[{ kind: "output", text: "scanning the repo" }]}
      />,
    );
    expect(screen.getByRole("tab", { name: /live log/i })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText("scanning the repo")).toBeInTheDocument();
  });
```

- [ ] **Step 5: Run it red.** `npx vitest run src/components/CardDrawer.test.tsx` → FAIL (`initialTab` not a prop).

- [ ] **Step 6: CardDrawer `initialTab`.** Add `initialTab?: Tab;` to props; destructure `initialTab`; change `const [tab, setTab] = useState<Tab>("artifact");` to `useState<Tab>(initialTab ?? "artifact")`.

- [ ] **Step 7: App synthetic Task + wiring.** In `src/App.tsx`:
  (a) Add `const activeGenerators = useActiveGenerators(selectedRun?.id ?? null);` near the `useTaskLog()` line (`:99`), importing the hook.
  (b) Pass `activeGenerators={activeGenerators}` to `<BoardView>` (`:397-404`).
  (c) Make `openTask` resolve a synthetic Task for a `gen:` id. Replace the `openTask` derivation (`:113-114`):

```tsx
  const openTask: Task | null = (() => {
    if (openTaskId == null) return null;
    const real = tasks.find((t) => t.id === openTaskId);
    if (real) return real;
    // A `gen:` id has no DB row — synthesise a minimal running source Task so the
    // drawer opens on the live-log tab; other tabs show their empty states.
    if (openTaskId.startsWith("gen:")) {
      const stage = openTaskId.split(":")[2] ?? "source";
      return {
        id: openTaskId,
        project_id: activeProject?.id ?? "",
        pipeline: pipeline?.id ?? "",
        topic: `${stage} · scanning…`,
        target_repo: null,
        target_scope: null,
        current_stage: stage,
        state: "running",
        attempts: 1,
        parent_artifact: null,
        review_artifact: null,
        created_at: 0,
        updated_at: 0,
        run_id: selectedRun?.id ?? null,
        item_key: null,
      } as Task;
    }
    return null;
  })();
```

  (d) On the `<CardDrawer>` (`:410`), pass `initialTab={openTask?.id.startsWith("gen:") ? "live log" : undefined}`.

- [ ] **Step 8: Run green.** `npx vitest run src/components src/App.test.tsx 2>/dev/null; npx vitest run` → PASS. `npx tsc --noEmit` → clean. (The synthetic task has no artifact/invocations, so the artifact/review/lineage/history tabs render their existing empty states — verified by reading CardDrawer's empty branches; no new code needed there.)

- [ ] **Step 9: Commit.**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/components/BoardView.tsx src/components/CardDrawer.tsx src/App.tsx src/components/BoardView.test.tsx src/components/CardDrawer.test.tsx
git commit -m "feat(ui): transient clickable generator card opens a synthetic live-log drawer"
```

---

## Final task: full verification gates

- [ ] **Step 1: Rust.** `cd src-tauri && cargo test` → all green. `cargo clippy --all-targets -- -D warnings` → clean. `cargo build` → builds.
- [ ] **Step 2: Frontend.** `cd /Users/tim/projects/agent-bus-app && npx vitest run` → all green. `npx tsc --noEmit` → clean.
- [ ] **Step 3: Tag (optional).** `git tag plan-live-worker-view`

---

## Spec coverage check

- **§A** capture thinking → A1 (LogDelta/LogKind, `LogSink = Box<dyn Fn(&LogDelta)>`), A2 (`feed` tags text→Output/thinking→Thinking; only Output accumulates), A3 (llm_chat mirror: thinking excluded from reply), A4 (`make_task_log_sink` coalesces per kind, emits `task-log {kind}`). ✓
- **§B** live view → B1 (`TaskLog.kind`, `useTaskLog` ordered tagged segments, coalesced), B2 (CardDrawer renders output prose + thinking dimmed/italic with marker via `--text-3`, auto-scroll, states preserved). ✓
- **§C** generator clickable → C1 (`gen:<run>:<stage>` id), C2 (`generator_pass_settled`), C3 (event name both contracts), C4 (loop emits `generator-status`), C5 (`useActiveGenerators`), C6 (BoardView transient card + App synthetic Task on the live-log tab). ✓
- **§E** card identity → E1 (`OutputItem.description`, `parse_items` `DESCRIPTION:`), E2 (contract requests it), E3 (`topic_for_item` fallback de-kebab), E4 (engine stores topic on transformer + generator commits), E5 (head/board lead with description, slug secondary, no raw `task.id`). ✓
- **§D** testing (no live claude) → every Rust task uses fixtures/FakeRunner; every frontend task uses vitest + mocked listeners. ✓
- Relationship/non-goals: persistence of finished logs is OUT of scope (running-worker live view only) — not implemented, as intended. ✓

## Risks & notes for the implementer
- **`feed` return-type change (A2) is a wide blast radius:** `String` → `Vec<LogDelta>` touches `parse_stream`, `parse_stream_streaming`, `claude_cli.rs`, `fake.rs`, `output.rs` tests. Do A2 in ONE commit (do not leave the crate half-broken between A1 and A2). The single test in A1 Step 4 is the only thing that compiles standalone.
- **Throttled `flush_if_due` (A4):** the per-kind buffers only emit when a buffer crosses its threshold; a short final fragment may not flush until `force_flush`. Confirm the live path force-flushes at stream end (check whether the existing R4 path force-flushes; if not, that pre-existing behavior is unchanged and acceptable — live `claude` is structural-only here).
- **Engine test harness (E4, C-tests):** the `generate_once`/`transform_once` integration tests need the exact in-memory `EngineContext` setup from the neighbouring tests. Copy it verbatim — the `unimplemented!()` placeholders in E4 Step 1 MUST be replaced with the real setup before running; do not invent a new harness.
- **`gen:` id collides with nothing:** `tasks.find` returns no real row for it, so the synthetic-Task branch is the only resolver. Confirm no code path tries to `listInvocations("gen:…")` and errors — App's invocations effect (`:147-165`) will call it; it should fail-soft to `[]` (it already catches). Acceptable (history tab shows its empty state).
- **`Card.tsx` unused `id` const:** deleting it (E5 Step 5) avoids a TS `noUnusedLocals`/lint failure — verify the project's tsconfig before assuming; if unused locals are allowed, leaving it is harmless but prefer deletion.
- **Other `onTaskLog` mocks:** B1's `TaskLog.kind` becomes required; any existing test that mocks `onTaskLog` with a payload lacking `kind` will fail `tsc`. Grep `grep -rn "onTaskLog" src/` and add `kind` to those payloads.
- **Run-scope reset (C5):** `useActiveGenerators` clears its set on `runId` change so a stale generator card never lingers across run switches.
