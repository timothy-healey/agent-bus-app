//! stream-json parsing — the inbound half of the ACL. Turns Claude's
//! line-delimited JSON into our RunnerOutput. A worker's result is the
//! `structured_output` object on the final `result` event, which the CLI has
//! already validated against the `--json-schema` it was given. Assistant prose
//! is kept for the live log and never parsed for a result.

use crate::output::{RunnerError, RunnerOutput, RunnerUsage};
use agent_bus_core::{DenialSource, OutputKind, PermissionDenial, PermissionMode};
use serde_json::Value;

/// Accumulator fed one parsed JSON line at a time.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    text: String,
    usage: RunnerUsage,
    saw_result: bool,
    /// The `result` event's `structured_output`, when it carried one.
    structured: Option<Value>,
    /// The permission mode the invocation asked for; `None` skips the check.
    requested_mode: Option<PermissionMode>,
    /// The last `permissionMode` any `system` event reported. `init` reports
    /// the requested mode; a following `system/status` reports a fallback.
    reported_mode: Option<String>,
    /// `tool_use_id` → `decision_reason_type` from `system/permission_denied`.
    denial_reasons: Vec<(String, String)>,
    /// The `result` event's `permission_denials`, as given.
    raw_denials: Vec<Value>,
}

impl StreamAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// An accumulator that fails the run when the CLI reports a different
    /// permission mode than `mode`.
    pub fn expecting_mode(mode: PermissionMode) -> Self {
        Self { requested_mode: Some(mode), ..Self::default() }
    }

    /// The denials seen so far: every `result.permission_denials` entry, tagged
    /// `classifier` when its `system/permission_denied` event says so and
    /// `rule` otherwise (a denied file edit emits no system event at all).
    pub fn denials(&self) -> Vec<PermissionDenial> {
        self.raw_denials
            .iter()
            .map(|d| {
                let id = d.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                let classifier = self.denial_reasons.iter().any(|(i, r)| i == id && r == "classifier");
                PermissionDenial {
                    tool_name: d.get("tool_name").and_then(Value::as_str).unwrap_or("").to_string(),
                    tool_input: d.get("tool_input").cloned().unwrap_or(Value::Null),
                    source: if classifier { DenialSource::Classifier } else { DenialSource::Rule },
                }
            })
            .collect()
    }

    /// Feed one stream-json line (already a parsed Value). Returns the tagged
    /// `LogDelta`s this event produced (empty for non-prose events) so a streaming
    /// caller can forward them; the accumulator keeps the running prose for the
    /// final RunnerOutput (Output blocks only — thinking is display-only and never
    /// accumulated). Returns Err only on a recognised rate-limit error event.
    /// (Mirrors `llm_chat::stream_json`'s `feed`, by design — separate ACL; vet F3.)
    pub fn feed(&mut self, v: &Value) -> Result<Vec<crate::output::LogDelta>, RunnerError> {
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let mut deltas: Vec<crate::output::LogDelta> = Vec::new();

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
                                    // Thinking is forwarded for display only, never
                                    // kept in self.text.
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
            "result" => {
                self.saw_result = true;
                if let Some(u) = v.get("usage") {
                    // The result usage is authoritative for totals — replace,
                    // but keep the model already captured from the system line.
                    let model = std::mem::take(&mut self.usage.model);
                    self.usage = RunnerUsage { model, ..Default::default() };
                    self.add_usage(u);
                }
                if let Some(c) = v.get("total_cost_usd").and_then(|c| c.as_f64()) {
                    self.usage.cost_micros = Some((c * 1_000_000.0).round() as u64);
                }
                if let Some(d) = v.get("permission_denials").and_then(Value::as_array) {
                    self.raw_denials = d.clone();
                }
                // An absent key and an explicit null both mean "no structured output".
                self.structured = v.get("structured_output").filter(|s| !s.is_null()).cloned();
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
                if let Some(mode) = v.get("permissionMode").and_then(Value::as_str) {
                    self.reported_mode = Some(mode.to_string());
                }
                if v.get("subtype").and_then(Value::as_str) == Some("permission_denied") {
                    let id = v.get("tool_use_id").and_then(Value::as_str).unwrap_or("").to_string();
                    let reason = v.get("decision_reason_type").and_then(Value::as_str).unwrap_or("").to_string();
                    self.denial_reasons.push((id, reason));
                }
            }
            _ => {}
        }
        Ok(deltas)
    }

    fn add_usage(&mut self, u: &Value) {
        let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        self.usage.input_tokens += g("input_tokens");
        self.usage.output_tokens += g("output_tokens");
        self.usage.cache_creation += g("cache_creation_input_tokens");
        self.usage.cache_read += g("cache_read_input_tokens");
    }

    /// Finish: produce a RunnerOutput whose result is the `structured_output`
    /// read as `kind`, with the run's denials. An empty stream is `NoResult`; a
    /// run in a different permission mode than requested is
    /// `PermissionModeMismatch`; a run that ended without a structured output,
    /// or with one of the wrong shape, is `NoStructuredOutput`.
    pub fn finish(self, model: &str, kind: OutputKind) -> Result<RunnerOutput, RunnerError> {
        if !self.saw_result && self.text.is_empty() {
            return Err(RunnerError::NoResult);
        }
        let denials = self.denials();
        let mut usage = self.usage;
        if usage.model.is_empty() {
            usage.model = model.to_string();
        }
        if let (Some(requested), Some(actual)) = (self.requested_mode, &self.reported_mode) {
            if requested.as_cli() != actual {
                return Err(RunnerError::PermissionModeMismatch {
                    requested: requested.as_cli().to_string(),
                    actual: actual.clone(),
                    usage,
                    denials,
                });
            }
        }
        let Some(structured) = &self.structured else {
            return Err(RunnerError::NoStructuredOutput {
                detail: "the run ended without a structured_output".into(),
                usage,
                denials,
            });
        };
        let result = match kind.parse(structured) {
            Ok(r) => r,
            Err(e) => {
                return Err(RunnerError::NoStructuredOutput { detail: format!("{kind:?} shape: {e}"), usage, denials })
            }
        };
        Ok(RunnerOutput { result, final_text: self.text, usage, permission_denials: denials })
    }
}

/// The output-contract block the engine appends to a worker's system prompt.
/// It carries the guidance the schema cannot: where to write files, which keys
/// a generator has already found, and how long a description may be. The shape
/// of the result itself is the `--json-schema` the runner passes. Pure.
pub fn output_contract(kind: OutputKind, artifact_dir: &str, already_found: &[String]) -> String {
    let mut s = String::from("## Output contract\n\n");
    s.push_str(&format!(
        "Write any file you produce under `{artifact_dir}`. You have write access there.\n"
    ));
    match kind {
        OutputKind::Generator => {
            s.push_str(
                "\nYou are the GENERATOR (source) stage. Scan the work and return one item per \
                 NEW candidate you find, each with a stable, unique key and a description of 80 \
                 characters or fewer. Use the key in any file name so items stay traceable. Do \
                 NOT return a key that was already found. Return an empty list when you find \
                 nothing new.\n",
            );
            if already_found.is_empty() {
                s.push_str("Already-found keys: none yet (this is the first pass).\n");
            } else {
                s.push_str("Already-found keys (do NOT return these):\n");
                for k in already_found {
                    s.push_str(&format!("  - {k}\n"));
                }
            }
        }
        OutputKind::Reviewer => {
            s.push_str(
                "\nYou are a REVIEWER. Assess your input and give a verdict: approve (it moves \
                 on), revise (it goes back to its producer) or reject (it goes to a human). \
                 Always give a reason; on revise the producer acts on it. If you write a \
                 critique file, return its path as the artifact.\n",
            );
        }
        OutputKind::Producer => {
            s.push_str(
                "\nYou are a PRODUCER. Transform your input into one output file, return its \
                 path as the artifact, and a description of 80 characters or fewer.\n",
            );
        }
    }
    s.push_str("\nReport your result by calling the StructuredOutput tool.\n");
    s
}

/// Parse a full stream from raw text (newline-delimited JSON), feeding each
/// non-blank line. Convenience used by ClaudeCliRunner + tests.
pub fn parse_stream(raw: &str, model: &str, kind: OutputKind) -> Result<RunnerOutput, RunnerError> {
    let mut acc = StreamAccumulator::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| RunnerError::Other(format!("bad stream-json line: {e}")))?;
        let _ = acc.feed(&v)?;
    }
    acc.finish(model, kind)
}

/// Streaming variant of `parse_stream`. Parses the same newline-delimited JSON
/// but invokes `on_delta` with each assistant *prose fragment* as it is parsed
/// (display-only), then returns the identical final RunnerOutput. The result
/// line's prose is NOT forwarded as a delta — it has already been streamed via
/// the assistant events. Mirrors
/// `llm_chat::stream_json::parse_chat_stream_streaming` (separate ACL crate, by
/// design — vet F3; do not merge the two parsers).
pub fn parse_stream_streaming(
    raw: &str,
    model: &str,
    kind: OutputKind,
    requested_mode: Option<PermissionMode>,
    on_delta: &mut dyn FnMut(&crate::output::LogDelta),
) -> Result<RunnerOutput, RunnerError> {
    let mut acc = match requested_mode {
        Some(m) => StreamAccumulator::expecting_mode(m),
        None => StreamAccumulator::new(),
    };
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
    acc.finish(model, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{GeneratedItem, OutputKind, ProducerOutput, Verdict, WorkerResult};

    // Real `claude --print --output-format stream-json --verbose --json-schema=...`
    // runs (haiku), trimmed of account, path and session detail.
    const GENERATOR: &str = include_str!("fixtures/structured-generator.jsonl");
    const PRODUCER: &str = include_str!("fixtures/structured-producer.jsonl");
    const REVIEWER: &str = include_str!("fixtures/structured-reviewer.jsonl");
    // The model could not satisfy the schema, was nudged once, and gave up in
    // prose: `subtype: success`, exit 0, and no `structured_output` key.
    const MISSING: &str = include_str!("fixtures/structured-missing.jsonl");
    // Real sonnet runs under `--permission-mode auto`: a Write into a denied
    // read path plus a force push (both refused by rule), and a push refused by
    // the classifier. Neither passed `--json-schema`.
    const DENIED_RULE: &str = include_str!("fixtures/permission-denied-rule.jsonl");
    const DENIED_CLASSIFIER: &str = include_str!("fixtures/permission-denied-classifier.jsonl");
    // A real haiku run asked for auto: `init` says auto, the following
    // `system/status` says default.
    const MODE_FALLBACK: &str = include_str!("fixtures/permission-mode-fallback.jsonl");

    fn finish_with(raw: &str, mode: Option<PermissionMode>) -> Result<RunnerOutput, RunnerError> {
        parse_stream_streaming(raw, "m", OutputKind::Producer, mode, &mut |_d| {})
    }

    #[test]
    fn rule_denials_are_tagged_rule_with_their_input() {
        let err = finish_with(DENIED_RULE, Some(PermissionMode::Auto)).unwrap_err();
        let d = err.denials();
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!(d[0].tool_name, "Write");
        assert_eq!(d[0].source, DenialSource::Rule);
        assert!(d[0].tool_input["file_path"].as_str().unwrap().ends_with("/readonly/out.txt"));
        assert_eq!(d[1].tool_name, "Bash");
        assert_eq!(d[1].source, DenialSource::Rule);
        assert_eq!(d[1].tool_input["command"], "git push --force origin feature-x");
        assert!(matches!(err, RunnerError::NoStructuredOutput { .. }), "{err:?}");
    }

    #[test]
    fn a_classifier_block_is_tagged_classifier() {
        let err = finish_with(DENIED_CLASSIFIER, Some(PermissionMode::Auto)).unwrap_err();
        let d = err.denials();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].source, DenialSource::Classifier);
        assert_eq!(d[0].tool_input["command"], "git push origin feature-x");
    }

    #[test]
    fn a_fallback_to_default_when_auto_was_requested_is_a_mismatch() {
        match finish_with(MODE_FALLBACK, Some(PermissionMode::Auto)).unwrap_err() {
            RunnerError::PermissionModeMismatch { requested, actual, usage, denials } => {
                assert_eq!(requested, "auto");
                assert_eq!(actual, "default");
                assert!(usage.output_tokens > 0);
                assert!(denials.is_empty());
            }
            other => panic!("expected PermissionModeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_run_in_the_requested_mode_is_not_a_mismatch() {
        let err = finish_with(DENIED_CLASSIFIER, Some(PermissionMode::Auto)).unwrap_err();
        assert!(!matches!(err, RunnerError::PermissionModeMismatch { .. }));
        let err = finish_with(DENIED_CLASSIFIER, Some(PermissionMode::AcceptEdits)).unwrap_err();
        assert!(matches!(err, RunnerError::PermissionModeMismatch { .. }), "{err:?}");
        // No requested mode: no check.
        let err = finish_with(MODE_FALLBACK, None).unwrap_err();
        assert!(matches!(err, RunnerError::NoStructuredOutput { .. }), "{err:?}");
    }

    #[test]
    fn denials_ride_on_a_successful_output() {
        let raw = r#"{"type":"system","subtype":"init","permissionMode":"acceptEdits"}
{"type":"system","subtype":"permission_denied","tool_name":"Bash","tool_use_id":"t1","decision_reason_type":"classifier"}
{"type":"result","subtype":"success","is_error":false,"result":"ok","structured_output":{"artifact":"a.md"},"permission_denials":[{"tool_name":"Bash","tool_use_id":"t1","tool_input":{"command":"gh pr create"}},{"tool_name":"Edit","tool_use_id":"t2","tool_input":{"file_path":"/r/x"}}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
        let out = finish_with(raw, Some(PermissionMode::AcceptEdits)).unwrap();
        assert_eq!(out.permission_denials.len(), 2);
        assert_eq!(out.permission_denials[0].source, DenialSource::Classifier);
        assert_eq!(out.permission_denials[1].source, DenialSource::Rule);
    }

    #[test]
    fn denial_fixtures_are_redacted() {
        for f in [DENIED_RULE, DENIED_CLASSIFIER, MODE_FALLBACK] {
            assert!(!f.contains("/Users/"), "home path in a fixture");
            assert!(!f.contains("\"email\""), "an account email in a fixture");
            assert!(!f.contains("messaging_socket_path") && !f.contains("memory_paths"));
            for line in f.lines() {
                let v: Value = serde_json::from_str(line).unwrap();
                if let Some(id) = v.get("session_id").and_then(Value::as_str) {
                    assert_eq!(id, "00000000-0000-0000-0000-000000000000");
                }
            }
        }
    }

    #[test]
    fn a_real_reviewer_run_gives_its_verdict_and_reason() {
        let out = parse_stream(REVIEWER, "fallback", OutputKind::Reviewer).unwrap();
        match out.result {
            WorkerResult::Reviewer(r) => {
                assert_eq!(r.verdict, Verdict::Approve);
                assert!(r.reason.contains("2+2=4"), "{}", r.reason);
                assert_eq!(r.artifact, None);
            }
            other => panic!("expected a reviewer result, got {other:?}"),
        }
        assert_eq!(out.usage.model, "claude-haiku-4-5-20251001");
        assert!(out.usage.cost_micros.unwrap() > 0);
    }

    #[test]
    fn a_real_producer_run_gives_its_artifact_and_description() {
        let out = parse_stream(PRODUCER, "fallback", OutputKind::Producer).unwrap();
        assert_eq!(
            out.result,
            WorkerResult::Producer(ProducerOutput {
                artifact: Some("artifacts/notes/greeting-v1.md".into()),
                description: Some("A short greeting note.".into()),
            })
        );
    }

    #[test]
    fn a_real_generator_run_gives_its_items() {
        let out = parse_stream(GENERATOR, "fallback", OutputKind::Generator).unwrap();
        match out.result {
            WorkerResult::Generator(g) => assert_eq!(
                g.items,
                vec![
                    GeneratedItem { key: "apple".into(), description: "a red fruit".into(), artifact: None },
                    GeneratedItem { key: "pear".into(), description: "a green fruit".into(), artifact: None },
                ]
            ),
            other => panic!("expected a generator result, got {other:?}"),
        }
    }

    #[test]
    fn a_success_result_without_structured_output_is_no_structured_output() {
        let err = parse_stream(MISSING, "m", OutputKind::Producer).unwrap_err();
        assert!(matches!(err, RunnerError::NoStructuredOutput { .. }), "got {err:?}");
    }

    #[test]
    fn a_run_without_structured_output_still_reports_what_it_cost() {
        match parse_stream(MISSING, "m", OutputKind::Producer).unwrap_err() {
            RunnerError::NoStructuredOutput { usage, .. } => {
                assert_eq!(usage.model, "claude-haiku-4-5-20251001");
                assert!(usage.output_tokens > 0);
                assert!(usage.cost_micros.unwrap() > 0);
            }
            other => panic!("expected NoStructuredOutput, got {other:?}"),
        }
    }

    #[test]
    fn the_assistant_prose_is_kept_as_final_text_for_the_log() {
        // The reviewer run wrote prose before calling StructuredOutput.
        let out = parse_stream(REVIEWER, "m", OutputKind::Reviewer).unwrap();
        assert!(out.final_text.contains("2+2=4"));
    }

    #[test]
    fn an_object_of_the_wrong_shape_is_no_structured_output() {
        let raw = r#"{"type":"result","subtype":"success","is_error":false,"result":"{}","structured_output":{"verdict":"maybe"}}"#;
        let err = parse_stream(raw, "m", OutputKind::Reviewer).unwrap_err();
        match err {
            RunnerError::NoStructuredOutput { detail, .. } => assert!(!detail.is_empty()),
            other => panic!("expected NoStructuredOutput, got {other:?}"),
        }
    }

    #[test]
    fn a_reviewer_object_handed_to_a_generator_is_no_structured_output() {
        let raw = r#"{"type":"result","subtype":"success","is_error":false,"structured_output":{"verdict":"approve","reason":"ok"}}"#;
        assert!(matches!(
            parse_stream(raw, "m", OutputKind::Generator).unwrap_err(),
            RunnerError::NoStructuredOutput { .. }
        ));
    }

    #[test]
    fn rate_limit_event_is_an_error() {
        let raw = r#"{"type":"error","error":{"message":"Rate limit exceeded (429)"}}"#;
        let err = parse_stream(raw, "m", OutputKind::Producer).unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[test]
    fn empty_stream_is_no_result() {
        let err = parse_stream("\n  \n", "m", OutputKind::Producer).unwrap_err();
        assert!(matches!(err, RunnerError::NoResult));
    }

    #[test]
    fn malformed_line_is_other_error() {
        let err = parse_stream("not json", "m", OutputKind::Producer).unwrap_err();
        assert!(matches!(err, RunnerError::Other(_)));
    }

    #[test]
    fn result_usage_is_authoritative_and_model_comes_from_init() {
        let raw = concat!(
            r#"{"type":"system","subtype":"init","model":"claude-opus-4-7"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":9,"output_tokens":9}}}"#, "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"structured_output":{},"usage":{"input_tokens":1200,"output_tokens":32,"cache_creation_input_tokens":300,"cache_read_input_tokens":50}}"#,
        );
        let out = parse_stream(raw, "fallback", OutputKind::Producer).unwrap();
        assert_eq!(out.usage.input_tokens, 1200);
        assert_eq!(out.usage.output_tokens, 32);
        assert_eq!(out.usage.cache_creation, 300);
        assert_eq!(out.usage.cache_read, 50);
        assert_eq!(out.usage.model, "claude-opus-4-7");
    }

    #[test]
    fn feed_tags_text_as_output_and_thinking_as_thinking_and_excludes_thinking_from_final() {
        use crate::output::{LogDelta, LogKind};
        let raw = concat!(
            r#"{"type":"system","subtype":"init","model":"m"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"let me reason"},{"type":"text","text":"Looks right."}]}}"#, "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"result":"{}","structured_output":{"verdict":"approve","reason":"fine"},"usage":{"input_tokens":1,"output_tokens":1}}"#
        );
        let mut seen: Vec<LogDelta> = vec![];
        let out = parse_stream_streaming(raw, "m", OutputKind::Reviewer, None, &mut |d: &LogDelta| seen.push(d.clone())).unwrap();
        assert_eq!(seen, vec![
            LogDelta { kind: LogKind::Thinking, text: "let me reason".into() },
            LogDelta { kind: LogKind::Output, text: "Looks right.".into() },
        ]);
        assert!(!out.final_text.contains("let me reason"));
        assert_eq!(out.final_text, "Looks right.");
    }

    #[test]
    fn streaming_parse_returns_the_same_output_as_the_whole_buffer_parse() {
        let mut n = 0;
        let streamed = parse_stream_streaming(REVIEWER, "m", OutputKind::Reviewer, None, &mut |_d| n += 1).unwrap();
        assert_eq!(streamed, parse_stream(REVIEWER, "m", OutputKind::Reviewer).unwrap());
        assert!(n > 0, "the run's prose was forwarded");
    }

    #[test]
    fn result_total_cost_usd_becomes_cost_micros() {
        let raw = r#"{"type":"result","subtype":"success","is_error":false,"structured_output":{},"total_cost_usd":0.004249,"usage":{"input_tokens":10,"output_tokens":2}}"#;
        assert_eq!(parse_stream(raw, "m", OutputKind::Producer).unwrap().usage.cost_micros, Some(4_249));
    }

    #[test]
    fn a_result_without_total_cost_usd_leaves_cost_unknown() {
        let raw = r#"{"type":"result","subtype":"success","is_error":false,"structured_output":{},"usage":{"input_tokens":1,"output_tokens":1}}"#;
        assert_eq!(parse_stream(raw, "m", OutputKind::Producer).unwrap().usage.cost_micros, None);
    }

    #[test]
    fn every_contract_says_to_call_the_structured_output_tool_and_has_no_markers() {
        for kind in [OutputKind::Generator, OutputKind::Producer, OutputKind::Reviewer] {
            let c = output_contract(kind, "/data/artifacts/x", &[]);
            assert!(c.contains("Report your result by calling the StructuredOutput tool."), "{kind:?}");
            for marker in ["KEY:", "VERDICT:", "ARTIFACT:", "DESCRIPTION:"] {
                assert!(!c.contains(marker), "{kind:?} contract still has {marker}");
            }
            assert!(c.contains("/data/artifacts/x"), "{kind:?} names the artifact folder");
        }
    }

    #[test]
    fn the_generator_contract_lists_the_already_found_keys() {
        let found = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
        let c = output_contract(OutputKind::Generator, "/d", &found);
        assert!(c.contains("src/a.rs") && c.contains("src/b.rs"));
        assert!(output_contract(OutputKind::Generator, "/d", &[]).contains("none yet"));
    }

    #[test]
    fn contracts_with_a_description_state_its_length() {
        for kind in [OutputKind::Generator, OutputKind::Producer] {
            assert!(output_contract(kind, "/d", &[]).contains("80 characters"), "{kind:?}");
        }
    }

    #[test]
    fn the_reviewer_contract_asks_for_a_verdict_with_a_reason() {
        let c = output_contract(OutputKind::Reviewer, "/d", &[]);
        assert!(c.contains("approve") && c.contains("revise") && c.contains("reject"));
        assert!(c.to_lowercase().contains("reason"));
    }

    #[test]
    fn the_producer_contract_grants_write_access_to_the_artifact_folder() {
        let c = output_contract(OutputKind::Producer, "/data/artifacts/plans", &[]);
        assert!(c.contains("/data/artifacts/plans"));
        assert!(c.to_lowercase().contains("write access"));
    }
}
