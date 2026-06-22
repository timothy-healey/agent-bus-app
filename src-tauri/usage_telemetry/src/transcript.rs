//! Claude Code transcript ingestion. The relationship is Conformist: we adapt to
//! Anthropic's `~/.claude/projects/**/*.jsonl` shape; they don't care about us.
//! The PARSE + DEDUP logic is pure and fixture-tested; the live FSEvents/poll
//! loop (PollingTranscriptSource) is a thin adapter behind the TranscriptSource
//! seam (D4) — mirroring runners' SpawnFn — so the whole context reaches green
//! with zero real transcript files.

use serde_json::Value;

/// One parsed CC usage record, keyed by message_id for dedup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcUsageRecord {
    pub message_id: String,
    pub ts: i64,
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

/// Parse one transcript JSONL line into a usage record, or `None` if the line is
/// not an assistant-message-with-usage (or is unparseable). Lenient by design
/// (Conformist + the AI/Engineer slipperiness tension): never panics; skips
/// anything that doesn't carry `message.id` + `message.usage`.
pub fn parse_transcript_line(line: &str) -> Option<CcUsageRecord> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let msg = v.get("message")?;
    let message_id = msg.get("id").and_then(|x| x.as_str())?.to_string();
    let usage = msg.get("usage")?;
    let g = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let ts = v
        .get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(parse_rfc3339_secs)
        .unwrap_or(0);
    Some(CcUsageRecord {
        message_id,
        ts,
        model: msg.get("model").and_then(|m| m.as_str()).map(String::from),
        input_tokens: g("input_tokens"),
        output_tokens: g("output_tokens"),
        cache_creation: g("cache_creation_input_tokens"),
        cache_read: g("cache_read_input_tokens"),
    })
}

/// Parse all usage records from a transcript blob (newline-delimited). Skips
/// non-usage / malformed lines. Does NOT dedup — the store's UNIQUE(message_id)
/// is the dedup authority; this just extracts candidates.
pub fn parse_transcript(blob: &str) -> Vec<CcUsageRecord> {
    blob.lines().filter_map(parse_transcript_line).collect()
}

/// Minimal RFC3339 → unix-seconds (no chrono dep). Accepts the
/// `YYYY-MM-DDТHH:MM:SSZ` form CC writes. Returns None on any deviation; the
/// caller defaults ts to 0 (out-of-window, harmless) rather than failing.
fn parse_rfc3339_secs(s: &str) -> Option<i64> {
    // s = "2026-06-22T11:14:00Z"
    let bytes = s.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[10] != b'T' || !s.ends_with('Z') {
        return None;
    }
    let p = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (p(0, 4)?, p(5, 7)?, p(8, 10)?);
    let (h, mi, se) = (p(11, 13)?, p(14, 16)?, p(17, 19)?);
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se)
}

/// Days since unix epoch for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The I/O seam (D4). The production impl reads new lines off the JSONL files;
/// tests pass a canned blob. Object-safe; held behind Box/Arc by the root.
pub trait TranscriptSource: Send + Sync {
    /// Return any transcript content available since the last poll. The caller
    /// feeds the result through `parse_transcript` and into the dedup store.
    fn poll(&self) -> String;
}

/// A canned source for tests + the empty default (no transcript dir yet).
pub struct StaticTranscriptSource(pub String);
impl TranscriptSource for StaticTranscriptSource {
    fn poll(&self) -> String {
        self.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("fixtures/transcript-sample.jsonl");
    const MALFORMED: &str = include_str!("fixtures/transcript-malformed.jsonl");

    #[test]
    fn parses_sample_records_with_usage_and_timestamp() {
        let recs = parse_transcript(SAMPLE);
        assert_eq!(recs.len(), 3); // dedup is the store's job, not the parser's
        assert_eq!(recs[0].message_id, "msg_aaa");
        assert_eq!(recs[0].input_tokens, 1200);
        assert_eq!(recs[0].output_tokens, 300);
        assert_eq!(recs[0].cache_creation, 40);
        assert_eq!(recs[0].cache_read, 10);
        assert_eq!(recs[0].model.as_deref(), Some("claude-opus-4-7"));
        // timestamp parsed to a positive unix-seconds value
        assert!(recs[0].ts > 1_700_000_000);
        assert_eq!(recs[1].message_id, "msg_bbb");
        assert_eq!(recs[2].message_id, "msg_aaa"); // duplicate kept here
    }

    #[test]
    fn skips_non_usage_and_malformed_lines() {
        let recs = parse_transcript(MALFORMED);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].message_id, "msg_ccc");
        assert_eq!(recs[0].input_tokens, 700);
        // cache fields absent in the fixture default to 0
        assert_eq!(recs[0].cache_creation, 0);
    }

    #[test]
    fn line_without_usage_is_none() {
        assert!(parse_transcript_line(r#"{"type":"user","message":{"id":"x"}}"#).is_none());
        assert!(parse_transcript_line("garbage").is_none());
        assert!(parse_transcript_line("").is_none());
    }

    #[test]
    fn static_source_returns_its_blob() {
        let s = StaticTranscriptSource("hello".into());
        assert_eq!(s.poll(), "hello");
    }
}
