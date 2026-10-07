//! The `get_usage` control request: the account's real plan Utilization, read
//! from the `claude` CLI without calling a model. The only code that knows the
//! request and response shape.

use agent_bus_core::{LimitKind, LimitReading, UtilizationReading, UtilizationSource};
use serde_json::Value;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The control request line written to `claude`'s stdin.
pub const GET_USAGE_REQUEST: &str =
    "{\"type\":\"control_request\",\"request_id\":\"u1\",\"request\":{\"subtype\":\"get_usage\"}}\n";

const ARGS: &[&str] = &[
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--no-session-persistence",
];

/// RFC 3339 → unix seconds. Accepts optional fractional seconds and either `Z`
/// or a `±HH:MM` offset (fractions are truncated).
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        rest = &frac[digits..];
    }
    let offset = if rest == "Z" {
        0
    } else if rest.len() == 6 && (rest.starts_with('+') || rest.starts_with('-')) && &rest[3..4] == ":" {
        let oh = rest.get(1..3)?.parse::<i64>().ok()?;
        let om = rest.get(4..6)?.parse::<i64>().ok()?;
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        sign * (oh * 3600 + om * 60)
    } else {
        return None;
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// Days since the unix epoch for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn limit(v: &Value, kind: LimitKind) -> Result<Option<LimitReading>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let utilization_pct = v["utilization"].as_f64().ok_or("usage response missing utilization")?;
    let resets = v["resets_at"].as_str().ok_or("usage response missing resets_at")?;
    let resets_at = parse_rfc3339(resets).ok_or_else(|| format!("unreadable resets_at: {resets}"))?;
    Ok(Some(LimitReading { kind, utilization_pct, resets_at }))
}

/// Parse `claude`'s stdout for the `get_usage` control response.
pub fn parse_get_usage(stdout: &str, now: i64) -> Result<UtilizationReading, String> {
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["type"] != "control_response" {
            continue;
        }
        let resp = &v["response"];
        if resp["subtype"] == "error" {
            return Err(resp["error"].as_str().unwrap_or("usage query failed").to_string());
        }
        let body = &resp["response"];
        let rl = &body["rate_limits"];
        if body["rate_limits_available"] == Value::Bool(false) || !rl.is_object() {
            return Err("usage not reported for this account".into());
        }
        let mut limits = Vec::new();
        if let Some(l) = limit(&rl["five_hour"], LimitKind::Session)? {
            limits.push(l);
        }
        if let Some(l) = limit(&rl["seven_day"], LimitKind::Weekly)? {
            limits.push(l);
        }
        if let Some(scoped) = rl["model_scoped"].as_array() {
            for m in scoped {
                let model = m["display_name"].as_str().unwrap_or("model").to_string();
                if let Some(l) = limit(m, LimitKind::ModelWeekly { model })? {
                    limits.push(l);
                }
            }
        }
        return Ok(UtilizationReading { observed_at: now, limits });
    }
    Err("no usage response from claude".into())
}

/// Production source: spawns `claude` and sends `get_usage`.
pub struct ClaudeCliUtilizationSource {
    bin: String,
    timeout: Duration,
}

impl ClaudeCliUtilizationSource {
    pub fn new() -> Self {
        Self::with_bin(crate::command::CLAUDE_BIN, Duration::from_secs(10))
    }

    pub fn with_bin(bin: impl Into<String>, timeout: Duration) -> Self {
        Self { bin: bin.into(), timeout }
    }
}

impl Default for ClaudeCliUtilizationSource {
    fn default() -> Self {
        Self::new()
    }
}

impl UtilizationSource for ClaudeCliUtilizationSource {
    fn fetch(&self) -> Result<UtilizationReading, String> {
        let mut child = Command::new(&self.bin)
            .args(ARGS)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start claude: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            // Dropping stdin closes it, which tells claude the input is complete.
            let _ = stdin.write_all(GET_USAGE_REQUEST.as_bytes());
        }
        let deadline = Instant::now() + self.timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("usage query timed out".into());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(format!("usage query failed: {e}")),
            }
        }
        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout.read_to_string(&mut out).map_err(|e| format!("usage query failed: {e}"))?;
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        parse_get_usage(&out, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::LimitKind;

    const SAMPLE: &str = include_str!("fixtures/get-usage-sample.jsonl");

    #[test]
    fn rfc3339_handles_fraction_and_offsets() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-07T13:49:59.971272+00:00"), Some(1_791_380_999));
        // +10:00 is ten hours ahead of UTC, so the same wall time is earlier in UTC.
        assert_eq!(parse_rfc3339("2026-10-07T23:49:59+10:00"), Some(1_791_380_999));
        assert_eq!(parse_rfc3339("not a date"), None);
    }

    #[test]
    fn parses_session_weekly_and_model_scoped_limits() {
        let r = parse_get_usage(SAMPLE, 1000).unwrap();
        assert_eq!(r.observed_at, 1000);
        assert_eq!(r.limits.len(), 3);
        assert_eq!(r.limits[0].kind, LimitKind::Session);
        assert_eq!(r.limits[0].utilization_pct, 41.0);
        assert_eq!(r.limits[0].resets_at, 1_791_380_999);
        assert_eq!(r.limits[1].kind, LimitKind::Weekly);
        assert_eq!(r.limits[2].kind, LimitKind::ModelWeekly { model: "Fable".into() });
    }

    #[test]
    fn unavailable_rate_limits_is_an_error() {
        let s = r#"{"type":"control_response","response":{"subtype":"success","request_id":"u1","response":{"rate_limits_available":false}}}"#;
        assert_eq!(parse_get_usage(s, 0).unwrap_err(), "usage not reported for this account");
    }

    #[test]
    fn error_control_response_surfaces_the_cli_message() {
        let s = r#"{"type":"control_response","response":{"subtype":"error","request_id":"u1","error":"Unsupported control request subtype: get_usage"}}"#;
        assert_eq!(parse_get_usage(s, 0).unwrap_err(), "Unsupported control request subtype: get_usage");
    }

    #[test]
    fn missing_model_scoped_is_fine() {
        let s = r#"{"type":"control_response","response":{"subtype":"success","request_id":"u1","response":{"rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":5,"resets_at":"2026-10-07T13:49:59Z"},"seven_day":null}}}}"#;
        let r = parse_get_usage(s, 0).unwrap();
        assert_eq!(r.limits.len(), 1);
        assert_eq!(r.limits[0].kind, LimitKind::Session);
    }

    #[test]
    fn no_control_response_is_an_error() {
        assert_eq!(parse_get_usage("", 0).unwrap_err(), "no usage response from claude");
    }

    #[cfg(unix)]
    fn script(body: &str) -> (std::path::PathBuf, String) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("abp-usage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake-claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path.to_string_lossy().into_owned())
    }

    #[cfg(unix)]
    #[test]
    fn cli_source_reads_the_fake_binarys_response() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/get-usage-sample.jsonl");
        let (dir, bin) = script(&format!("cat > /dev/null\ncat '{}'", fixture.display()));
        let src = ClaudeCliUtilizationSource::with_bin(bin, std::time::Duration::from_secs(5));
        let r = src.fetch().unwrap();
        assert_eq!(r.limits[0].utilization_pct, 41.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn cli_source_times_out_a_hung_binary() {
        let (dir, bin) = script("sleep 30");
        let src = ClaudeCliUtilizationSource::with_bin(bin, std::time::Duration::from_millis(300));
        let started = std::time::Instant::now();
        assert_eq!(src.fetch().unwrap_err(), "usage query timed out");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(dir);
    }
}
