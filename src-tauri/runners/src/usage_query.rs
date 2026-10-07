//! The `get_usage` control request: the account's real plan Utilization, read
//! from the `claude` CLI without calling a model. The only code that knows the
//! request and response shape.
//!
//! The poll runs `claude` with `--setting-sources=` and `--strict-mcp-config`
//! from the system temp dir, so it never loads the operator's settings, hooks
//! or MCP servers (a poll fires every minute). Valued flags use the `=` form so
//! an empty value cannot swallow the next argument.

use agent_bus_core::{LimitKind, LimitReading, UtilizationReading, UtilizationSource};
use serde_json::Value;
use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
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
    "--setting-sources=",
    "--strict-mcp-config",
];

/// RFC 3339 → unix seconds. Accepts optional fractional seconds and either `Z`
/// or a `±HH:MM` offset (fractions are truncated).
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    // Helper: parse ASCII digit range and validate it's all digits
    let num_validated = |a: usize, z: usize, min: i64, max: i64| {
        let slice = s.get(a..z)?;
        // Ensure all chars are ASCII digits
        if !slice.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let val = slice.parse::<i64>().ok()?;
        if val < min || val > max {
            return None;
        }
        Some(val)
    };
    let y = num_validated(0, 4, 1970, 2100)?;
    let mo = num_validated(5, 7, 1, 12)?;
    let d = num_validated(8, 10, 1, 31)?;
    let h = num_validated(11, 13, 0, 23)?;
    let mi = num_validated(14, 16, 0, 59)?;
    let se = num_validated(17, 19, 0, 60)?;

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
    } else if rest.len() == 6 {
        let rest_bytes = rest.as_bytes();
        let sign_byte = rest_bytes[0];
        if (sign_byte != b'+' && sign_byte != b'-') || rest_bytes[3] != b':' {
            return None;
        }
        // Get offset hours and minutes from rest (indices 1-2 and 4-5)
        let oh_str = rest.get(1..3)?;
        let om_str = rest.get(4..6)?;
        // Validate offset hours and minutes
        if !oh_str.bytes().all(|c| c.is_ascii_digit()) || !om_str.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let oh = oh_str.parse::<i64>().ok()?;
        let om = om_str.parse::<i64>().ok()?;
        if oh > 23 || om > 59 {
            return None;
        }
        let sign = if sign_byte == b'-' { -1 } else { 1 };
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
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start claude: {e}"))?;

        // Take stdout immediately and read it on a separate thread to avoid pipe-fill deadlock.
        let stdout = child.stdout.take();
        let (tx, rx) = mpsc::channel();
        let _reader_thread = if let Some(stdout_handle) = stdout {
            Some(thread::spawn(move || {
                let mut out = String::new();
                let mut reader = BufReader::new(stdout_handle);
                let _ = reader.read_to_string(&mut out);
                let _ = tx.send(out);
            }))
        } else {
            None
        };

        // Send request to stdin and drop it to signal EOF.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(GET_USAGE_REQUEST.as_bytes());
        }

        // Helper: reap child with bounded try_wait up to deadline.
        let bounded_reap = |child: &mut std::process::Child, deadline: Instant| {
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return, // Child exited
                    Ok(None) if Instant::now() >= deadline => {
                        // Deadline passed; force kill and wait (killed process reaps promptly).
                        let _ = child.kill();
                        let _ = child.wait();
                        return;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(30)),
                    Err(_) => return, // Error reaping; give up
                }
            }
        };

        // Wait for stdout data or timeout, bounded by the overall timeout.
        let deadline = Instant::now() + self.timeout;
        let out = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining) {
                Ok(data) => break data,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    bounded_reap(&mut child, deadline);
                    return Err("usage query timed out".into());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Reader thread panicked or closed; bounded reap then break.
                    bounded_reap(&mut child, deadline);
                    break String::new();
                }
            }
        };

        // Reap the child with bounded try_wait (child may still be running if it closed stdout early).
        bounded_reap(&mut child, deadline);

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
    fn rfc3339_strict_validation() {
        // Negative offset
        assert_eq!(parse_rfc3339("2026-10-07T03:49:59-10:00"), Some(1_791_380_999));
        // Z with fraction
        assert_eq!(parse_rfc3339("2026-10-07T13:49:59.123456Z"), Some(1_791_380_999));
        // Out-of-range month
        assert_eq!(parse_rfc3339("2026-13-07T13:49:59Z"), None);
        // Out-of-range day
        assert_eq!(parse_rfc3339("2026-10-32T13:49:59Z"), None);
        // Out-of-range hour
        assert_eq!(parse_rfc3339("2026-10-07T24:49:59Z"), None);
        // Out-of-range minute
        assert_eq!(parse_rfc3339("2026-10-07T13:60:59Z"), None);
        // Out-of-range second
        assert_eq!(parse_rfc3339("2026-10-07T13:49:61Z"), None);
        // Out-of-range offset hour
        assert_eq!(parse_rfc3339("2026-10-07T13:49:59+24:00"), None);
        // Out-of-range offset minute
        assert_eq!(parse_rfc3339("2026-10-07T13:49:59+00:60"), None);
        // Signed digit field (+ in year)
        assert_eq!(parse_rfc3339("+026-10-07T13:49:59Z"), None);
        // Signed digit field (- in month, not offset)
        assert_eq!(parse_rfc3339("2026--1-07T13:49:59Z"), None);
    }

    #[test]
    fn rfc3339_no_panic_on_multibyte_tail() {
        // Multibyte UTF-8 at the end should not panic, just return None
        assert_eq!(parse_rfc3339("2026-10-07T13:49:59🔥"), None);
        assert_eq!(parse_rfc3339("2026-10-07T13:49:59.123🔥"), None);
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
    fn cli_source_skips_operator_settings_and_runs_outside_the_app_cwd() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/get-usage-sample.jsonl");
        let (dir, bin) = script("");
        let record = dir.join("record");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do echo \"arg:$a\"; done > '{r}'\necho \"cwd:$(pwd -P)\" >> '{r}'\ncat > /dev/null\ncat '{f}'\n",
                r = record.display(),
                f = fixture.display()
            ),
        )
        .unwrap();
        let src = ClaudeCliUtilizationSource::with_bin(bin, std::time::Duration::from_secs(5));
        src.fetch().unwrap();
        let rec = std::fs::read_to_string(&record).unwrap();
        let args: Vec<&str> = rec.lines().filter_map(|l| l.strip_prefix("arg:")).collect();
        assert_eq!(
            args,
            [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--no-session-persistence",
                "--setting-sources=",
                "--strict-mcp-config",
            ]
        );
        let cwd = rec.lines().find_map(|l| l.strip_prefix("cwd:")).unwrap();
        assert_eq!(std::path::Path::new(cwd), std::env::temp_dir().canonicalize().unwrap());
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

    #[cfg(unix)]
    #[test]
    fn cli_source_handles_large_stdout() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/get-usage-sample.jsonl");
        // Write >128KB of data then the fixture to test pipe-fill deadlock fix
        let (dir, bin) = script(&format!(
            "cat > /dev/null\nyes x | head -c 200000\ncat '{}'",
            fixture.display()
        ));
        let src = ClaudeCliUtilizationSource::with_bin(bin, std::time::Duration::from_secs(5));
        let r = src.fetch().unwrap();
        assert_eq!(r.limits[0].utilization_pct, 41.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn cli_source_bounded_when_grandchild_holds_stdout() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/get-usage-sample.jsonl");
        // Spawn a long-lived grandchild that holds stdout open after printing the fixture
        let (dir, bin) = script(&format!(
            "cat > /dev/null\ncat '{}'\nsleep 30 &",
            fixture.display()
        ));
        let src = ClaudeCliUtilizationSource::with_bin(bin, std::time::Duration::from_secs(2));
        let started = std::time::Instant::now();
        // Either succeeds (fixture read before grandchild blocks) or times out (bounded to ~3s)
        let _ = src.fetch();
        let elapsed = started.elapsed();
        assert!(elapsed < std::time::Duration::from_secs(3), "fetch took too long: {:?}", elapsed);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn cli_source_child_reap_bounded_by_deadline() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/get-usage-sample.jsonl");
        // Binary prints fixture, closes stdout (exec 1>&-), then sleeps 30s
        // The read completes when stdout closes, but the child keeps running.
        // Bounded reap must not block indefinitely.
        let (dir, bin) = script(&format!(
            "cat > /dev/null\ncat '{}'\nexec 1>&-\nsleep 30",
            fixture.display()
        ));
        let src = ClaudeCliUtilizationSource::with_bin(bin, std::time::Duration::from_secs(1));
        let started = std::time::Instant::now();
        let r = src.fetch();
        let elapsed = started.elapsed();
        // Must return Ok (data was complete) and finish within ~3s despite child still running
        assert!(r.is_ok(), "fetch should succeed: {:?}", r);
        assert_eq!(r.unwrap().limits[0].utilization_pct, 41.0);
        assert!(elapsed < std::time::Duration::from_secs(3), "fetch reap took too long: {:?}", elapsed);
        let _ = std::fs::remove_dir_all(dir);
    }
}
