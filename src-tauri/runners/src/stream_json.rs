//! stream-json parsing — the inbound half of the ACL. Turns Claude's
//! line-delimited JSON into our RunnerOutput. The verdict + artifact are parsed
//! from the model's text using a simple, lenient convention (VERDICT: / ARTIFACT:)
//! — the AI-Engineer-vs-Engineer tension (DOMAIN.md) is resolved by keeping the
//! parser lenient and defaulting to a safe verdict, never panicking on shape.

use crate::output::{RunnerError, RunnerOutput, RunnerUsage};
use agent_bus_core::Verdict;
use serde_json::Value;

/// Accumulator fed one parsed JSON line at a time.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    text: String,
    usage: RunnerUsage,
    saw_result: bool,
}

impl StreamAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one stream-json line (already a parsed Value). Returns the prose text
    /// *this* event added (empty for non-prose events) so a streaming caller can
    /// forward it; the accumulator keeps the running full text for the final
    /// RunnerOutput. Returns Err only on a recognised rate-limit error event.
    /// (Mirrors `llm_chat::stream_json`'s `feed`, by design — separate ACL; vet F3.)
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

    fn add_usage(&mut self, u: &Value) {
        let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        self.usage.input_tokens += g("input_tokens");
        self.usage.output_tokens += g("output_tokens");
        self.usage.cache_creation += g("cache_creation_input_tokens");
        self.usage.cache_read += g("cache_read_input_tokens");
    }

    /// Finish: produce a RunnerOutput. Errors if nothing was accumulated.
    pub fn finish(self, model: &str) -> Result<RunnerOutput, RunnerError> {
        if !self.saw_result && self.text.is_empty() {
            return Err(RunnerError::NoResult);
        }
        let verdict = parse_verdict(&self.text);
        let artifact_path = parse_artifact(&self.text);
        let mut usage = self.usage;
        if usage.model.is_empty() {
            usage.model = model.to_string();
        }
        Ok(RunnerOutput { verdict, artifact_path, final_text: self.text, usage })
    }
}

/// Parse a verdict from assistant text. Convention: a line `VERDICT: <word>`.
/// Lenient: defaults to Revise (the safe "send back for another look") when no
/// verdict is found, so a malformed model response never auto-approves.
pub fn parse_verdict(text: &str) -> Verdict {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("VERDICT:") {
            return match rest.trim().to_lowercase().as_str() {
                "approve" => Verdict::Approve,
                "reject" => Verdict::Reject,
                _ => Verdict::Revise,
            };
        }
    }
    Verdict::Revise
}

/// Parse an artifact path from assistant text. Convention: `ARTIFACT: <path>`.
pub fn parse_artifact(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ARTIFACT:") {
            let p = rest.trim();
            if !p.is_empty() {
                return Some(p.to_string());
            }
        }
    }
    None
}

/// One item in the list-of-items output contract (Runtime redesign ④b / L1).
///
/// The new bounded-buffer engine asks an agent to emit a *list* of items rather
/// than the legacy single `VERDICT:`/`ARTIFACT:` pair: a generator emits one item
/// per NEW candidate key, a transformer emits one item per output it produced, and
/// a reviewer adds a `VERDICT:` per item. This is the dual to `parse_verdict`/
/// `parse_artifact` and lives beside them in the Runners ACL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputItem {
    /// The work-item's stable candidate key (lineage + dedup identity). Empty
    /// (`""`) for a legacy single-item response with no `KEY:` line.
    pub key: String,
    /// Path of the artifact the agent wrote, if any.
    pub artifact_path: Option<String>,
    /// The reviewer verdict for this item, if the agent emitted one. `None` for
    /// producer/generator items (which imply forward).
    pub verdict: Option<Verdict>,
}

/// Parse the agent's emitted item list (Runtime redesign ④b / L1 output contract).
///
/// CONVENTION (the dual to `output_contract`): each item is a block of marked
/// lines — `KEY: <key>`, `ARTIFACT: <path>`, `VERDICT: <word>` — and a new
/// `KEY:` line starts a new item. The parse is lenient and NEVER panics:
///   * Lines are matched by prefix after trimming; unknown lines are ignored.
///   * A `VERDICT:`/`ARTIFACT:` seen before any `KEY:` opens an implicit
///     `key=""` item (the legacy single-item shape — one item, no key).
///   * A blank `VERDICT:` value parses as `Verdict::Revise` (the same safe
///     default `parse_verdict` uses), so a malformed verdict never auto-approves.
///   * Malformed / empty input → an empty Vec (best-effort, the caller decides).
pub fn parse_items(text: &str) -> Vec<OutputItem> {
    let mut items: Vec<OutputItem> = Vec::new();
    // The item currently being built (None until the first KEY/ARTIFACT/VERDICT).
    let mut current: Option<OutputItem> = None;

    fn verdict_of(rest: &str) -> Verdict {
        match rest.trim().to_lowercase().as_str() {
            "approve" => Verdict::Approve,
            "reject" => Verdict::Reject,
            _ => Verdict::Revise,
        }
    }

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("KEY:") {
            // A new KEY always starts a fresh item; flush the previous one.
            if let Some(item) = current.take() {
                items.push(item);
            }
            current = Some(OutputItem {
                key: rest.trim().to_string(),
                artifact_path: None,
                verdict: None,
            });
        } else if let Some(rest) = line.strip_prefix("ARTIFACT:") {
            let p = rest.trim();
            let item = current.get_or_insert_with(|| OutputItem {
                key: String::new(),
                artifact_path: None,
                verdict: None,
            });
            if !p.is_empty() {
                item.artifact_path = Some(p.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("VERDICT:") {
            let item = current.get_or_insert_with(|| OutputItem {
                key: String::new(),
                artifact_path: None,
                verdict: None,
            });
            item.verdict = Some(verdict_of(rest));
        }
    }
    if let Some(item) = current.take() {
        items.push(item);
    }
    items
}

/// The output-contract system-prompt preamble (Runtime redesign ④b / the L1 fix).
///
/// This is the text that finally TELLS the agent to emit the item list — the live
/// gap L1 named (the convention lived only in the parser + tests; nothing
/// instructed the agent). The engine composes the returned block into the system
/// prompt, where `role`, the `artifact_dir`, and the `already_found` key set are
/// known. Pure (no I/O); it never panics.
///
/// * `role` — `"generator"` (emit only NEW keys not in `already_found`),
///   `"reviewer"` (add a `VERDICT:` per item), or any producer role (emit items
///   with keys + artifacts, implicit forward).
/// * `artifact_dir` — the directory the agent must write artifacts under (the
///   worker grants write access to it, fixing the L1 write-access gap).
/// * `already_found` — keys the generator has already produced this run; only
///   meaningful for the generator role.
pub fn output_contract(role: &str, artifact_dir: &str, already_found: &[String]) -> String {
    let role_lc = role.to_lowercase();
    let mut s = String::new();
    s.push_str("## Output contract\n\n");
    s.push_str(
        "When you finish, emit a list of work-items. Write each item as a block of \
         marked lines:\n\n",
    );
    s.push_str("  KEY: <a stable, unique candidate key for this item>\n");
    s.push_str(&format!(
        "  ARTIFACT: {artifact_dir}/<key>... (the file you wrote for this item)\n"
    ));
    if role_lc == "reviewer" {
        s.push_str("  VERDICT: approve | revise | reject\n");
    }
    s.push('\n');
    s.push_str(&format!(
        "Write every artifact file under `{artifact_dir}` — you have write access \
         there. Use the exact KEY as part of the path so items stay traceable.\n",
    ));

    match role_lc.as_str() {
        "generator" => {
            s.push_str(
                "\nYou are the GENERATOR (source) stage. Scan the work and emit ONE item \
                 per NEW candidate you find — assign each a stable KEY. Do NOT re-emit any \
                 key that already exists. Emit nothing when you find no new candidates.\n",
            );
            if already_found.is_empty() {
                s.push_str("Already-found keys: (none yet — this is the first pass).\n");
            } else {
                s.push_str("Already-found keys (do NOT re-emit these):\n");
                for k in already_found {
                    s.push_str(&format!("  - {k}\n"));
                }
            }
        }
        "reviewer" => {
            s.push_str(
                "\nYou are a REVIEWER. For every item you assess, emit a VERDICT \
                 (approve / revise / reject) alongside its KEY.\n",
            );
        }
        _ => {
            s.push_str(
                "\nYou are a PRODUCER. Transform your input into one output item, emit its \
                 KEY and ARTIFACT; it is forwarded downstream implicitly (no verdict needed).\n",
            );
        }
    }
    s
}

/// Parse a full stream from raw text (newline-delimited JSON), feeding each
/// non-blank line. Convenience used by ClaudeCliRunner + tests.
pub fn parse_stream(raw: &str, model: &str) -> Result<RunnerOutput, RunnerError> {
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
    acc.finish(model)
}

/// Streaming variant of `parse_stream`. Parses the same newline-delimited JSON
/// but invokes `on_delta` with each assistant *prose fragment* as it is parsed
/// (display-only), then returns the identical final RunnerOutput. The result
/// line's authoritative prose is NOT forwarded as a delta — it has already been
/// streamed via the assistant events. Mirrors
/// `llm_chat::stream_json::parse_chat_stream_streaming` (separate ACL crate, by
/// design — vet F3; do not merge the two parsers).
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("fixtures/stream-json-sample.txt");

    #[test]
    fn parses_sample_into_approve_with_artifact_and_usage() {
        let out = parse_stream(SAMPLE, "fallback-model").unwrap();
        assert_eq!(out.verdict, Verdict::Approve);
        assert_eq!(out.artifact_path.as_deref(), Some("artifacts/analyses/T-1-v1.md"));
        // result usage is authoritative (1200/32/300/50), not the sum of deltas.
        assert_eq!(out.usage.input_tokens, 1200);
        assert_eq!(out.usage.output_tokens, 32);
        assert_eq!(out.usage.cache_creation, 300);
        assert_eq!(out.usage.cache_read, 50);
        // model came from the system init line.
        assert_eq!(out.usage.model, "claude-opus-4-7");
    }

    #[test]
    fn verdict_defaults_to_revise_when_absent() {
        assert_eq!(parse_verdict("no verdict here"), Verdict::Revise);
        assert_eq!(parse_verdict("VERDICT: approve"), Verdict::Approve);
        assert_eq!(parse_verdict("VERDICT: reject"), Verdict::Reject);
        assert_eq!(parse_verdict("VERDICT: nonsense"), Verdict::Revise);
    }

    #[test]
    fn artifact_is_optional() {
        assert_eq!(parse_artifact("VERDICT: approve"), None);
        assert_eq!(parse_artifact("ARTIFACT: a/b.md"), Some("a/b.md".to_string()));
    }

    #[test]
    fn rate_limit_event_is_an_error() {
        let raw = r#"{"type":"error","error":{"message":"Rate limit exceeded (429)"}}"#;
        let err = parse_stream(raw, "m").unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[test]
    fn empty_stream_is_no_result() {
        let err = parse_stream("\n  \n", "m").unwrap_err();
        assert!(matches!(err, RunnerError::NoResult));
    }

    #[test]
    fn malformed_line_is_other_error() {
        let err = parse_stream("not json", "m").unwrap_err();
        assert!(matches!(err, RunnerError::Other(_)));
    }

    #[test]
    fn feed_returns_prose_delta_for_assistant_event_only() {
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
    fn parse_items_reads_a_list_of_n_items() {
        let text = "\
KEY: alpha
ARTIFACT: artifacts/specs/alpha.md
VERDICT: approve
KEY: beta
ARTIFACT: artifacts/specs/beta.md
VERDICT: revise
KEY: gamma
ARTIFACT: artifacts/specs/gamma.md";
        let items = parse_items(text);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], OutputItem { key: "alpha".into(), artifact_path: Some("artifacts/specs/alpha.md".into()), verdict: Some(Verdict::Approve) });
        assert_eq!(items[1], OutputItem { key: "beta".into(), artifact_path: Some("artifacts/specs/beta.md".into()), verdict: Some(Verdict::Revise) });
        assert_eq!(items[2], OutputItem { key: "gamma".into(), artifact_path: Some("artifacts/specs/gamma.md".into()), verdict: None });
    }

    #[test]
    fn parse_items_handles_legacy_single_item_with_no_key() {
        // A legacy VERDICT:/ARTIFACT: with no KEY: => one item, key="".
        let text = "Some prose.\nARTIFACT: artifacts/analyses/a.md\nVERDICT: approve";
        let items = parse_items(text);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "");
        assert_eq!(items[0].artifact_path.as_deref(), Some("artifacts/analyses/a.md"));
        assert_eq!(items[0].verdict, Some(Verdict::Approve));
    }

    #[test]
    fn parse_items_is_best_effort_on_malformed_and_empty() {
        assert!(parse_items("").is_empty());
        assert!(parse_items("just prose, no markers").is_empty());
        // a blank verdict value defaults to the safe Revise (never auto-approve)
        let items = parse_items("KEY: x\nVERDICT:");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].verdict, Some(Verdict::Revise));
        // a KEY with no artifact/verdict is still an item
        let items = parse_items("KEY: only");
        assert_eq!(items, vec![OutputItem { key: "only".into(), artifact_path: None, verdict: None }]);
    }

    #[test]
    fn output_contract_tells_generator_to_emit_new_keys() {
        let found = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
        let c = output_contract("generator", "${project}/artifacts/research", &found);
        assert!(c.contains("KEY:"));
        assert!(c.contains("ARTIFACT:"));
        assert!(c.contains("${project}/artifacts/research"));
        assert!(c.to_lowercase().contains("generator"));
        // the already-found set is embedded so the agent dedups
        assert!(c.contains("src/a.rs"));
        assert!(c.contains("src/b.rs"));
        // a generator is not told to emit a verdict
        assert!(!c.contains("VERDICT:"));
    }

    #[test]
    fn output_contract_tells_reviewer_to_emit_a_verdict() {
        let c = output_contract("reviewer", "${project}/artifacts/specs", &[]);
        assert!(c.contains("VERDICT:"));
        assert!(c.to_lowercase().contains("reviewer"));
    }

    #[test]
    fn output_contract_producer_has_no_verdict_and_grants_write_access() {
        let c = output_contract("producer", "${project}/artifacts/plans", &[]);
        assert!(!c.contains("VERDICT:"));
        assert!(c.contains("${project}/artifacts/plans"));
        assert!(c.to_lowercase().contains("write access"));
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
}
