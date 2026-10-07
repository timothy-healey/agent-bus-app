# Real Utilization Meter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The usage meter and auto-brake read the subscription's real Utilization, polled for free from `claude`'s `get_usage` control request. Each team's Cost per 5-hour window is shown for information. The token estimate and transcript ingestion are removed.

**Architecture:**
- A kernel `UtilizationSource` trait. The Runners ACL implements it by spawning `claude -p --input-format stream-json …` and parsing the `get_usage` `control_response`.
- Usage Telemetry stores the latest reading in a one-row table and decides the brake from the session and weekly Limits. It assembles a new `UsageSnapshot`.
- The app replaces the 15s transcript sweep with a poller: every 60s, plus a notify after each settled worker step, at most once per 10s.
- Worker Cost flows from `result.total_cost_usd` through `RunnerUsage` and `UsageEvent` into `worker_usage_log`.

**Tech Stack:** Rust (Tauri 2, sqlx/SQLite, tokio), React 18 + TypeScript, vitest.

**Spec:** `docs/superpowers/specs/2026-10-07-real-utilization-design.md`

## Global Constraints

- Brake: set at `>= 0.95` and release at `< 0.85` of the stored fractions. Compare `utilization_pct / 100`. Watched Limits: `Session` and `Weekly` only. An expired Limit (`now >= resets_at`) counts as 0. No reading, or `available == false`, gives `NoChange`.
- Poll cadence: 60s, plus `poll_now`, ignored if less than 10s after the previous attempt. Each poll has a 10s process timeout.
- `get_usage` request line, verbatim: `{"type":"control_request","request_id":"u1","request":{"subtype":"get_usage"}}`
- `claude` args, verbatim: `-p --input-format stream-json --output-format stream-json --verbose --no-session-persistence`
- User-facing strings:
  - `— usage unavailable`
  - `usage not reported for this account`
  - `usage query timed out`
  - `cost this 5h (list price)`
  - `brakes new work at 95% of the session or weekly limit`
- **Plan-level ruling — Cost storage:** Cost is stored and carried as **integer micro-dollars** (`cost_micros: Option<u64>`), not `f64`. This keeps the existing `Eq` derives on `RunnerUsage`, `RunnerOutput` and `UsageEvent`. The snapshot converts to `cost_usd: f64` for display.
- **Plan-level ruling — migrations:** the spec's single migration 016 is split in two. Migration **016** adds `cost_micros` and `utilization_state`. Migration **017** drops `cc_usage_log`, in the task that deletes its code, so every task leaves a working app.
- **Plan-level ruling — per-team view:** per-team Cost rides on the existing `TeamSlice` (`{ team_id, tokens, cost_usd }`), rather than a separate `cost_by_team` list. The spec keeps per-team token usage, and one list serves both.
- Commands, from `Efforts/agent-bus-app`: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`, `npx vitest run`, `npx tsc --noEmit`.
- Code comments state the rule, never when or who decided it.

## Review Focus

1. **`resets_at` with fractional seconds and a `+00:00` offset** (the real `get_usage` form, e.g. `2026-10-07T13:49:59.971272+00:00`) parses to the right unix second. A non-UTC offset (`+10:00`) is honoured, not ignored. Pinned in Task 2.
2. **A `claude` that hangs** (stdin never read, no exit) is killed at the timeout, and the poll records `usage query timed out` rather than blocking the poller forever. Pinned in Task 2.
3. **A reading whose session window has already reset**, while the app sat idle past `resets_at`, must not hold the brake on. The session reads 0%, so an auto brake releases. Pinned in Task 3.
4. **A poll failure after a good reading** keeps the last good reading for display (with `available = false`) and never trips or releases the brake. Pinned in Task 3 (store) and Task 5 (snapshot).
5. **A result event without `total_cost_usd`** (an older CLI) stores `NULL`, and the team's Cost sums only the known values; the snapshot never fails. Pinned in Task 4 (parse) and Task 5 (snapshot sum).

---

### Task 1: Kernel utilization types + `UtilizationSource`

**Files:**
- Create: `src-tauri/agent_bus_core/src/utilization.rs`
- Modify: `src-tauri/agent_bus_core/src/lib.rs` (add `pub mod utilization;` and `pub use utilization::*;`)

**Interfaces:**
- Produces:
  - `LimitKind { Session, Weekly, ModelWeekly { model: String } }`
  - `LimitReading { kind, utilization_pct: f64, resets_at: i64 }`
  - `UtilizationReading { observed_at: i64, limits: Vec<LimitReading> }`, with Serialize, Deserialize, Clone, Debug and PartialEq.
  - `trait UtilizationSource: Send + Sync { fn fetch(&self) -> Result<UtilizationReading, String>; }`

- [ ] **Step 1: Write the failing test** in the new file `src-tauri/agent_bus_core/src/utilization.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_round_trips_through_json() {
        let r = UtilizationReading {
            observed_at: 100,
            limits: vec![
                LimitReading { kind: LimitKind::Session, utilization_pct: 41.0, resets_at: 200 },
                LimitReading { kind: LimitKind::Weekly, utilization_pct: 2.0, resets_at: 300 },
                LimitReading { kind: LimitKind::ModelWeekly { model: "Fable".into() }, utilization_pct: 0.0, resets_at: 400 },
            ],
        };
        let back: UtilizationReading = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn utilization_source_is_object_safe() {
        struct Fixed;
        impl UtilizationSource for Fixed {
            fn fetch(&self) -> Result<UtilizationReading, String> {
                Ok(UtilizationReading { observed_at: 1, limits: vec![] })
            }
        }
        let s: std::sync::Arc<dyn UtilizationSource> = std::sync::Arc::new(Fixed);
        assert_eq!(s.fetch().unwrap().observed_at, 1);
    }
}
```

Add `pub mod utilization;` and `pub use utilization::*;` to `lib.rs`.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p agent_bus_core utilization`
Expected: compile errors `cannot find type UtilizationReading`.

- [ ] **Step 3: Implement.** Put this above the tests module:

```rust
//! The account's plan Limits as Claude reports them. Usage Telemetry consumes a
//! `UtilizationSource`; the Runners ACL implements it, so neither depends on the
//! other's crate.

use serde::{Deserialize, Serialize};

/// Which plan Limit a reading is for. Only `Session` and `Weekly` drive the brake.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LimitKind {
    Session,
    Weekly,
    ModelWeekly { model: String },
}

/// One Limit's Utilization (0–100, as reported) and its reset time (unix seconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitReading {
    pub kind: LimitKind,
    pub utilization_pct: f64,
    pub resets_at: i64,
}

/// Every Limit reported at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UtilizationReading {
    pub observed_at: i64,
    pub limits: Vec<LimitReading>,
}

/// One fresh reading of the account's Limits, or why there isn't one.
pub trait UtilizationSource: Send + Sync {
    fn fetch(&self) -> Result<UtilizationReading, String>;
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p agent_bus_core`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/agent_bus_core/src/utilization.rs src-tauri/agent_bus_core/src/lib.rs
git commit -m "feat(core): UtilizationReading + UtilizationSource kernel seam"
```

---

### Task 2: Runners — `get_usage` parser + `ClaudeCliUtilizationSource`

**Files:**
- Create: `src-tauri/runners/src/usage_query.rs`
- Create: `src-tauri/runners/src/fixtures/get-usage-sample.jsonl`
- Modify: `src-tauri/runners/src/lib.rs` (add `pub mod usage_query;`)

**Interfaces:**
- Consumes: Task 1's kernel types.
- Produces:
  - `pub const GET_USAGE_REQUEST: &str`
  - `pub fn parse_rfc3339(s: &str) -> Option<i64>`
  - `pub fn parse_get_usage(stdout: &str, now: i64) -> Result<UtilizationReading, String>`
  - `pub struct ClaudeCliUtilizationSource`, with `new()` and `with_bin(bin: impl Into<String>, timeout: Duration)`; implements `UtilizationSource`.

- [ ] **Step 1: Write the fixture** `src-tauri/runners/src/fixtures/get-usage-sample.jsonl`. It is sanitised: no account, org or credit fields.

```
{"type":"system","subtype":"hook_started","hook_event":"SessionStart"}
{"type":"control_response","response":{"subtype":"success","request_id":"u1","response":{"rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":41,"resets_at":"2026-10-07T13:49:59.971272+00:00"},"seven_day":{"utilization":2,"resets_at":"2026-10-13T06:59:59.971295+00:00"},"model_scoped":[{"display_name":"Fable","utilization":0,"resets_at":"2026-10-13T07:00:00+00:00"}]}}}}
```

- [ ] **Step 2: Write the failing tests** in `src-tauri/runners/src/usage_query.rs`:

```rust
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
```

Add `pub mod usage_query;` to `src-tauri/runners/src/lib.rs`.

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p runners usage_query`
Expected: compile errors `cannot find function parse_get_usage` / `parse_rfc3339` / `ClaudeCliUtilizationSource`.

- [ ] **Step 4: Implement.** Put this above the tests module:

```rust
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p runners`
Expected: all pass, including the 8 new tests.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runners/src/usage_query.rs src-tauri/runners/src/fixtures/get-usage-sample.jsonl src-tauri/runners/src/lib.rs
git commit -m "feat(runners): read plan Utilization via claude's get_usage control request"
```

---

### Task 3: Usage Telemetry — utilization store + brake decision

**Files:**
- Create: `src-tauri/app/migrations/016_utilization.sql`
- Create: `src-tauri/usage_telemetry/src/utilization_store.rs`
- Modify: `src-tauri/usage_telemetry/src/brake_policy.rs` (add `watched_fraction`, `decide_utilization`, tests)
- Modify: `src-tauri/usage_telemetry/src/lib.rs` (add `pub mod utilization_store;`)
- Modify: `src-tauri/app/src/lib.rs` (register migration 16 in **both** lists; bump the count test)

**Interfaces:**
- Consumes: Task 1's kernel types.
- Produces:
  - `UtilizationStore::new(pool)`, with `record_ok(&self, &UtilizationReading, now: i64)`, `record_err(&self, err: &str, now: i64)` and `load(&self) -> StoredUtilization`.
  - `StoredUtilization { reading: Option<UtilizationReading>, last_ok_at: Option<i64>, last_attempt_at: Option<i64>, last_error: Option<String> }`, with `fn available(&self) -> bool`.
  - `brake_policy::watched_fraction(&UtilizationReading, now: i64) -> Option<f64>`.
  - `brake_policy::decide_utilization(reading: Option<&UtilizationReading>, available: bool, now: i64, auto_on: bool, on: f64, off: f64) -> BrakeDecision`.

- [ ] **Step 1: Write the migration** `src-tauri/app/migrations/016_utilization.sql`:

```sql
-- 016_utilization.sql — real plan Utilization (polled from claude's get_usage)
-- and per-invocation Cost on worker rows (integer micro-dollars).
ALTER TABLE worker_usage_log ADD COLUMN cost_micros INTEGER;

CREATE TABLE IF NOT EXISTS utilization_state (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  reading_json    TEXT,
  last_ok_at      INTEGER,
  last_attempt_at INTEGER,
  last_error      TEXT
);
INSERT OR IGNORE INTO utilization_state (id) VALUES (1);
```

Register it in `src-tauri/app/src/lib.rs` in both places:
- `run_migrations`' `MIGRATIONS` array: `(16, include_str!("../migrations/016_utilization.sql")),`
- the plugin-sql `Migration` list:
  ```rust
  Migration { version: 16, description: "real plan utilization + worker cost", sql: include_str!("../migrations/016_utilization.sql"), kind: MigrationKind::Up },
  ```

Change the count test's `assert_eq!(version, 15, "all fifteen migrations recorded");` to `assert_eq!(version, 16, "all sixteen migrations recorded");`.

- [ ] **Step 2: Write the failing tests.**

In `src-tauri/usage_telemetry/src/utilization_store.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{LimitKind, LimitReading};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn store() -> UtilizationStore {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        sqlx::query("ALTER TABLE worker_usage_log ADD COLUMN cost_micros INTEGER").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE utilization_state (id INTEGER PRIMARY KEY CHECK (id = 1), reading_json TEXT, last_ok_at INTEGER, last_attempt_at INTEGER, last_error TEXT)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO utilization_state (id) VALUES (1)").execute(&pool).await.unwrap();
        UtilizationStore::new(pool)
    }

    fn reading(pct: f64) -> UtilizationReading {
        UtilizationReading { observed_at: 10, limits: vec![LimitReading { kind: LimitKind::Session, utilization_pct: pct, resets_at: 99 }] }
    }

    #[tokio::test]
    async fn empty_store_has_no_reading_and_is_unavailable() {
        let s = store().await.load().await;
        assert!(s.reading.is_none());
        assert!(!s.available());
    }

    #[tokio::test]
    async fn ok_then_err_keeps_the_last_good_reading_but_is_unavailable() {
        let st = store().await;
        st.record_ok(&reading(41.0), 10).await;
        assert!(st.load().await.available());
        st.record_err("usage query timed out", 20).await;
        let s = st.load().await;
        assert_eq!(s.reading, Some(reading(41.0)));
        assert_eq!(s.last_ok_at, Some(10));
        assert_eq!(s.last_attempt_at, Some(20));
        assert_eq!(s.last_error.as_deref(), Some("usage query timed out"));
        assert!(!s.available());
    }
}
```

Add to the end of the `tests` module in `src-tauri/usage_telemetry/src/brake_policy.rs` (create `#[cfg(test)] mod tests { use super::*; … }` if absent):

```rust
    use agent_bus_core::{LimitKind, LimitReading, UtilizationReading};

    fn r(session: f64, weekly: f64, fable: f64, resets_at: i64) -> UtilizationReading {
        UtilizationReading { observed_at: 0, limits: vec![
            LimitReading { kind: LimitKind::Session, utilization_pct: session, resets_at },
            LimitReading { kind: LimitKind::Weekly, utilization_pct: weekly, resets_at: 10_000 },
            LimitReading { kind: LimitKind::ModelWeekly { model: "Fable".into() }, utilization_pct: fable, resets_at: 10_000 },
        ]}
    }

    #[test]
    fn session_at_95_sets_the_brake() {
        assert_eq!(decide_utilization(Some(&r(95.0, 2.0, 0.0, 5_000)), true, 100, false, 0.95, 0.85), BrakeDecision::SetOn(AUTO_METER_REASON.into()));
    }

    #[test]
    fn weekly_at_95_sets_the_brake() {
        assert_eq!(decide_utilization(Some(&r(10.0, 96.0, 0.0, 5_000)), true, 100, false, 0.95, 0.85), BrakeDecision::SetOn(AUTO_METER_REASON.into()));
    }

    #[test]
    fn model_scoped_limits_never_brake() {
        assert_eq!(decide_utilization(Some(&r(10.0, 2.0, 100.0, 5_000)), true, 100, false, 0.95, 0.85), BrakeDecision::NoChange);
    }

    #[test]
    fn release_needs_both_watched_limits_below_85() {
        assert_eq!(decide_utilization(Some(&r(80.0, 90.0, 0.0, 5_000)), true, 100, true, 0.95, 0.85), BrakeDecision::NoChange);
        assert_eq!(decide_utilization(Some(&r(80.0, 84.0, 0.0, 5_000)), true, 100, true, 0.95, 0.85), BrakeDecision::Release);
    }

    #[test]
    fn an_expired_session_counts_as_zero_and_releases() {
        // session read 99% but its window reset at t=50; now is t=100.
        assert_eq!(decide_utilization(Some(&r(99.0, 2.0, 0.0, 50)), true, 100, true, 0.95, 0.85), BrakeDecision::Release);
    }

    #[test]
    fn no_reading_or_unavailable_changes_nothing() {
        assert_eq!(decide_utilization(None, true, 100, false, 0.95, 0.85), BrakeDecision::NoChange);
        assert_eq!(decide_utilization(Some(&r(99.0, 2.0, 0.0, 5_000)), false, 100, false, 0.95, 0.85), BrakeDecision::NoChange);
        assert_eq!(decide_utilization(Some(&r(10.0, 2.0, 0.0, 5_000)), false, 100, true, 0.95, 0.85), BrakeDecision::NoChange);
    }
```

Add `pub mod utilization_store;` to `usage_telemetry/src/lib.rs`.

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p usage_telemetry`
Expected: compile errors `cannot find function decide_utilization` / `cannot find type UtilizationStore`.

- [ ] **Step 4: Implement.**

Append to `brake_policy.rs` (above its tests):

```rust
use agent_bus_core::{LimitKind, UtilizationReading};

/// The highest Utilization (as a fraction) across the Limits the brake watches:
/// the session and weekly Limits. A Limit whose window has reset counts as 0.
/// None when the reading has neither Limit.
pub fn watched_fraction(reading: &UtilizationReading, now: i64) -> Option<f64> {
    reading
        .limits
        .iter()
        .filter(|l| matches!(l.kind, LimitKind::Session | LimitKind::Weekly))
        .map(|l| if now >= l.resets_at { 0.0 } else { l.utilization_pct / 100.0 })
        .fold(None, |acc: Option<f64>, x| Some(acc.map_or(x, |a| a.max(x))))
}

/// The auto-brake decision from a real reading. Without a reading, or when the
/// last poll failed, the brake is left as it is.
pub fn decide_utilization(
    reading: Option<&UtilizationReading>,
    available: bool,
    now: i64,
    auto_on: bool,
    brake_on_pct: f64,
    brake_off_pct: f64,
) -> BrakeDecision {
    if !available {
        return BrakeDecision::NoChange;
    }
    match reading.and_then(|r| watched_fraction(r, now)) {
        Some(f) => decide(f, auto_on, brake_on_pct, brake_off_pct),
        None => BrakeDecision::NoChange,
    }
}
```

`utilization_store.rs` (above its tests):

```rust
//! The latest plan Utilization reading, persisted in the one-row
//! `utilization_state` table so the meter has a value across restarts.

use agent_bus_core::UtilizationReading;
use sqlx::SqlitePool;

/// What the last poll left behind. `reading` is the last *good* reading; a
/// failed poll keeps it and records the error.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StoredUtilization {
    pub reading: Option<UtilizationReading>,
    pub last_ok_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub last_error: Option<String>,
}

impl StoredUtilization {
    /// The last poll succeeded.
    pub fn available(&self) -> bool {
        self.last_attempt_at.is_some() && self.last_error.is_none()
    }
}

pub struct UtilizationStore {
    pool: SqlitePool,
}

impl UtilizationStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn record_ok(&self, reading: &UtilizationReading, now: i64) {
        let json = serde_json::to_string(reading).unwrap_or_default();
        if let Err(e) = sqlx::query(
            "UPDATE utilization_state SET reading_json = ?, last_ok_at = ?, last_attempt_at = ?, last_error = NULL WHERE id = 1",
        )
        .bind(json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        {
            eprintln!("usage_telemetry: failed to store utilization: {e}");
        }
    }

    pub async fn record_err(&self, err: &str, now: i64) {
        if let Err(e) = sqlx::query("UPDATE utilization_state SET last_attempt_at = ?, last_error = ? WHERE id = 1")
            .bind(now)
            .bind(err)
            .execute(&self.pool)
            .await
        {
            eprintln!("usage_telemetry: failed to store utilization error: {e}");
        }
    }

    pub async fn load(&self) -> StoredUtilization {
        let row: Option<(Option<String>, Option<i64>, Option<i64>, Option<String>)> = sqlx::query_as(
            "SELECT reading_json, last_ok_at, last_attempt_at, last_error FROM utilization_state WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        match row {
            Some((json, last_ok_at, last_attempt_at, last_error)) => StoredUtilization {
                reading: json.and_then(|j| serde_json::from_str(&j).ok()),
                last_ok_at,
                last_attempt_at,
                last_error,
            },
            None => StoredUtilization::default(),
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
Expected: all pass, including the migration count test at 16.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/migrations/016_utilization.sql src-tauri/usage_telemetry/src/utilization_store.rs src-tauri/usage_telemetry/src/brake_policy.rs src-tauri/usage_telemetry/src/lib.rs src-tauri/app/src/lib.rs
git commit -m "feat(usage): store the latest Utilization reading + decide the brake from it"
```

---

### Task 4: Capture Cost from worker results

**Files:**
- Modify: `src-tauri/agent_bus_core/src/usage.rs` (`UsageEvent.cost_micros`)
- Modify: `src-tauri/usage_telemetry/src/worker_log.rs` (`insert` writes `cost_micros`; test pool adds the column)
- Modify: `src-tauri/runners/src/output.rs` (`RunnerUsage.cost_micros`)
- Modify: `src-tauri/runners/src/stream_json.rs` (read `total_cost_usd` in the `result` branch)
- Modify: `src-tauri/runtime/src/engine.rs:1419` (copy into `UsageEvent`)
- Modify: `RunnerUsage` literals at `runners/src/anthropic_api.rs:120`, `runners/src/output.rs:212`, `runtime/src/engine.rs:2993` and `runtime/src/engine.rs:3057`. Add `cost_micros: None` to each, or `..Default::default()` where they already spread.

**Interfaces:**
- Consumes: migration 016's `worker_usage_log.cost_micros` column (Task 3).
- Produces: `RunnerUsage.cost_micros: Option<u64>` and `UsageEvent.cost_micros: Option<u64>`, where micro-dollars are `(total_cost_usd * 1_000_000).round()`; `WorkerUsageStore::insert` persists it.

- [ ] **Step 1: Write the failing tests** in `runners/src/stream_json.rs`'s tests module:

```rust
    #[test]
    fn result_total_cost_usd_becomes_cost_micros() {
        let raw = concat!(
            r#"{"type":"system","subtype":"init","model":"claude-opus-5-5"}"#, "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"result":"VERDICT: approve","total_cost_usd":0.004249,"usage":{"input_tokens":10,"output_tokens":2,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#, "\n",
        );
        let out = parse_stream(raw, "claude-opus-5-5").unwrap();
        assert_eq!(out.usage.cost_micros, Some(4_249));
    }

    #[test]
    fn a_result_without_total_cost_usd_leaves_cost_unknown() {
        let raw = concat!(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"VERDICT: approve","usage":{"input_tokens":1,"output_tokens":1}}"#, "\n",
        );
        assert_eq!(parse_stream(raw, "m").unwrap().usage.cost_micros, None);
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p runners cost`
Expected: compile error `no field cost_micros on RunnerUsage`.

- [ ] **Step 3: Implement.**
- `output.rs`: add `pub cost_micros: Option<u64>,` to `RunnerUsage`. `Default` gives `None`.
- `usage.rs` (kernel): add `pub cost_micros: Option<u64>,` to `UsageEvent`, documented as `/// List-price Cost of the invocation in micro-dollars, when Claude reported it.`. Add `cost_micros: None` to every existing `UsageEvent { … }` literal: `agent_bus_core/src/usage.rs:53` and `:71` (tests), `usage_telemetry/src/snapshot.rs:150` (the old snapshot test, rewritten in Task 5) and `usage_telemetry/src/worker_log.rs:122` (`ev`).
- `stream_json.rs`, `result` branch: after the usage block, add:

```rust
                if let Some(c) = v.get("total_cost_usd").and_then(|c| c.as_f64()) {
                    self.usage.cost_micros = Some((c * 1_000_000.0).round() as u64);
                }
```

- `engine.rs:1419`: add `cost_micros: output.usage.cost_micros,` to the `UsageEvent` literal.
- `worker_log.rs`: change the `insert` statement to `(ts, team_id, task_id, model, runner, input_tokens, output_tokens, cache_creation, cache_read, cost_micros) VALUES (?,?,?,?,?,?,?,?,?,?)`, with `.bind(e.cost_micros.map(|c| c as i64))` last. Its tests' `fresh_pool()` adds the column after the 005 query: `sqlx::query("ALTER TABLE worker_usage_log ADD COLUMN cost_micros INTEGER").execute(&pool).await.unwrap();`, and its `ev(...)` helper gains `cost_micros: None`.
- Add `cost_micros: None` to the four `RunnerUsage { … }` literals listed above that don't use `..Default::default()`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A src-tauri
git commit -m "feat(usage): record each worker invocation's list-price Cost"
```

---

### Task 5: Snapshot from Utilization; remove the token estimate; app poller

This task swaps the whole pipeline over in one commit, because removing `CcUsageStore` breaks the app's wiring until the poller replaces it.

**Files:**
- Modify: `src-tauri/usage_telemetry/src/snapshot.rs` (rewrite)
- Modify: `src-tauri/usage_telemetry/src/window.rs` (keep only `ThresholdBand` + `band`; delete `window_pct`, `burn_per_min`, `reset_in_secs`, `est_brake_at` and their tests)
- Modify: `src-tauri/usage_telemetry/src/api.rs` (`UsageState`, `load_config`, delete `usage_set_budget` + `SetBudgetArgs`, update `tools()`)
- Modify: `src-tauri/usage_telemetry/src/contract_tests.rs` (rewrite to the new shape)
- Modify: `src-tauri/usage_telemetry/src/worker_log.rs` (`team_breakdown` adds cost; delete `api_window_tokens`)
- Delete: `src-tauri/usage_telemetry/src/cc_log.rs`, `ingest.rs`, `transcript.rs` (and their `pub mod` lines in `lib.rs`)
- Create: `src-tauri/app/migrations/017_drop_cc_usage_log.sql`
- Modify: `src-tauri/app/src/lib.rs` (usage wiring, poller replacing sweep + boot backfill, delete `cc_claude_projects_dir`, `usage_snapshot` tool dispatch, remove `usage_set_budget` from `generate_handler!`, register migration 17 in both lists, count test → 17)
- Modify: `src-tauri/app/src/pipeline_activator.rs` (`WorkerDeps.usage_poll`; notify on settle)

**Interfaces:**
- Consumes:
  - `UsageEvent.cost_micros` and `worker_usage_log.cost_micros` (Task 4);
  - `UtilizationStore` / `StoredUtilization`, `decide_utilization` and `watched_fraction` (Task 3);
  - `ClaudeCliUtilizationSource` (Task 2);
  - `UtilizationSource` (Task 1).
- Produces:
  - `snapshot::compute_snapshot(worker: &WorkerUsageStore, util: &UtilizationStore, cfg: &UsageConfig, braked: bool, now: i64) -> Result<UsageSnapshot, SnapshotError>`;
  - `UsageSnapshot { available, observed_at, session, weekly, model_scoped, band, braked, auto_meter_enabled, by_team, tokens_by_task }`;
  - `LimitView { label, utilization_pct, resets_in_secs }`;
  - `TeamSlice { team_id, tokens, cost_usd }`;
  - `UsageState { worker, util, pool, is_braked }`;
  - `WorkerDeps.usage_poll: Option<Arc<tokio::sync::Notify>>`.

- [ ] **Step 1: Write the failing tests.** Replace the tests module in `snapshot.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utilization_store::UtilizationStore;
    use crate::worker_log::WorkerUsageStore;
    use agent_bus_core::{LimitKind, LimitReading, TaskId, TeamId, UsageEvent, UtilizationReading};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        sqlx::query("ALTER TABLE worker_usage_log ADD COLUMN cost_micros INTEGER").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE utilization_state (id INTEGER PRIMARY KEY CHECK (id = 1), reading_json TEXT, last_ok_at INTEGER, last_attempt_at INTEGER, last_error TEXT)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO utilization_state (id) VALUES (1)").execute(&pool).await.unwrap();
        pool
    }

    fn reading(session: f64, weekly: f64) -> UtilizationReading {
        UtilizationReading { observed_at: 900, limits: vec![
            LimitReading { kind: LimitKind::Session, utilization_pct: session, resets_at: 1_000 + 3_600 },
            LimitReading { kind: LimitKind::Weekly, utilization_pct: weekly, resets_at: 1_000 + 86_400 },
            LimitReading { kind: LimitKind::ModelWeekly { model: "Fable".into() }, utilization_pct: 0.0, resets_at: 1_000 + 86_400 },
        ]}
    }

    fn ev(ts: i64, team: &str, cost_micros: Option<u64>) -> UsageEvent {
        UsageEvent { ts, team_id: TeamId(team.into()), task_id: Some(TaskId("T-1".into())), model: "m".into(),
            input_tokens: 100, output_tokens: 20, cache_creation: 0, cache_read: 0, cost_micros }
    }

    #[tokio::test]
    async fn never_polled_is_unavailable_with_no_limits() {
        let p = pool().await;
        let snap = compute_snapshot(&WorkerUsageStore::new(p.clone()), &UtilizationStore::new(p), &UsageConfig::default(), false, 1_000).await.unwrap();
        assert!(!snap.available);
        assert!(snap.session.is_none() && snap.weekly.is_none());
        assert_eq!(snap.band, ThresholdBand::Safe);
    }

    #[tokio::test]
    async fn band_comes_from_the_higher_watched_limit() {
        let p = pool().await;
        let util = UtilizationStore::new(p.clone());
        util.record_ok(&reading(41.0, 88.0), 900).await;
        let snap = compute_snapshot(&WorkerUsageStore::new(p), &util, &UsageConfig::default(), false, 1_000).await.unwrap();
        assert!(snap.available);
        assert_eq!(snap.observed_at, Some(900));
        assert_eq!(snap.session.as_ref().unwrap().utilization_pct, 41.0);
        assert_eq!(snap.session.as_ref().unwrap().resets_in_secs, 3_600);
        assert_eq!(snap.weekly.as_ref().unwrap().label, "weekly (7d)");
        assert_eq!(snap.model_scoped.len(), 1);
        assert_eq!(snap.model_scoped[0].label, "Fable weekly");
        assert_eq!(snap.band, ThresholdBand::Hot);
    }

    #[tokio::test]
    async fn a_failed_poll_keeps_the_last_reading_but_reports_unavailable() {
        let p = pool().await;
        let util = UtilizationStore::new(p.clone());
        util.record_ok(&reading(41.0, 2.0), 900).await;
        util.record_err("usage query timed out", 950).await;
        let snap = compute_snapshot(&WorkerUsageStore::new(p), &util, &UsageConfig::default(), false, 1_000).await.unwrap();
        assert!(!snap.available);
        assert_eq!(snap.session.unwrap().utilization_pct, 41.0);
    }

    #[tokio::test]
    async fn by_team_sums_cost_in_the_window_and_tolerates_null_cost() {
        let p = pool().await;
        let worker = WorkerUsageStore::new(p.clone());
        worker.insert(&ev(900, "research", Some(250_000))).await.unwrap();
        worker.insert(&ev(950, "research", None)).await.unwrap();
        worker.insert(&ev(10, "research", Some(9_000_000))).await.unwrap(); // outside the 5h window ending at 20_000
        let cfg = UsageConfig::default();
        let snap = compute_snapshot(&worker, &UtilizationStore::new(p), &cfg, false, 18_500).await.unwrap();
        let research = snap.by_team.iter().find(|t| t.team_id == "research").unwrap();
        assert_eq!(research.cost_usd, 0.25);
        assert_eq!(research.tokens, 240);
    }
}
```

Replace `contract_tests.rs` with:

```rust
//! Serde contract regression tests for the Usage Telemetry IPC types. Lock the
//! JSON key sets + enum strings against `src/ipc/usage.ts`.

#![cfg(test)]

use crate::snapshot::{LimitView, TeamSlice, UsageSnapshot};
use crate::window::ThresholdBand;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn full_snapshot() -> UsageSnapshot {
    let mut tokens_by_task = HashMap::new();
    tokens_by_task.insert("T-1".to_string(), 120u64);
    let lv = |label: &str| LimitView { label: label.into(), utilization_pct: 41.0, resets_in_secs: 600 };
    UsageSnapshot {
        available: true,
        observed_at: Some(1_700_000_000),
        session: Some(lv("session (5h)")),
        weekly: Some(lv("weekly (7d)")),
        model_scoped: vec![lv("Fable weekly")],
        band: ThresholdBand::Warn,
        braked: false,
        auto_meter_enabled: false,
        by_team: vec![TeamSlice { team_id: "research".into(), tokens: 120, cost_usd: 0.25 }],
        tokens_by_task,
    }
}

#[test]
fn usage_snapshot_key_set_matches_ts() {
    let v = serde_json::to_value(full_snapshot()).unwrap();
    assert_eq!(keys(&v), set(&[
        "available", "observed_at", "session", "weekly", "model_scoped", "band",
        "braked", "auto_meter_enabled", "by_team", "tokens_by_task",
    ]));
    assert_eq!(v["band"], Value::String("warn".into()));
    assert!(v["tokens_by_task"].is_object());
}

#[test]
fn absent_limits_are_present_and_null() {
    let mut s = full_snapshot();
    s.session = None;
    s.observed_at = None;
    let v = serde_json::to_value(&s).unwrap();
    assert!(v.as_object().unwrap().contains_key("session"));
    assert!(v["session"].is_null());
    assert!(v["observed_at"].is_null());
}

#[test]
fn limit_view_and_team_slice_key_sets_match_ts() {
    let lv = serde_json::to_value(LimitView { label: "x".into(), utilization_pct: 1.0, resets_in_secs: 2 }).unwrap();
    assert_eq!(keys(&lv), set(&["label", "utilization_pct", "resets_in_secs"]));
    let ts = serde_json::to_value(TeamSlice { team_id: "r".into(), tokens: 1, cost_usd: 0.5 }).unwrap();
    assert_eq!(keys(&ts), set(&["team_id", "tokens", "cost_usd"]));
}

#[test]
fn threshold_band_matches_ts_string_union() {
    for (variant, s) in [(ThresholdBand::Safe, "safe"), (ThresholdBand::Warn, "warn"), (ThresholdBand::Hot, "hot"), (ThresholdBand::Braked, "braked")] {
        assert_eq!(serde_json::to_value(variant).unwrap(), Value::String(s.into()));
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p usage_telemetry`
Expected: compile errors `cannot find type LimitView` / mismatched `compute_snapshot` arguments.

- [ ] **Step 3: Implement the Usage Telemetry side.**

`snapshot.rs`: replace everything above the tests with:

```rust
//! UsageSnapshot — the single value the meter renders. Built from the latest
//! Utilization reading (the meter's headline + band) and the worker log
//! (per-team tokens and Cost in the 5-hour window, per-task tokens).

use crate::utilization_store::UtilizationStore;
use crate::window::{band, ThresholdBand};
use crate::brake_policy::watched_fraction;
use crate::worker_log::{WorkerUsageError, WorkerUsageStore};
use agent_bus_core::{LimitKind, LimitReading};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error(transparent)]
    Worker(#[from] WorkerUsageError),
}

/// Meter config, loaded from `usage_config` (row id=1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsageConfig {
    /// The per-team Cost window (the session Limit's length).
    pub window_secs: i64,
    pub brake_on_pct: f64,
    pub brake_off_pct: f64,
    pub auto_meter_enabled: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self { window_secs: 18_000, brake_on_pct: 0.95, brake_off_pct: 0.85, auto_meter_enabled: false }
    }
}

/// One Limit as the meter shows it. An expired Limit reads 0%.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitView {
    pub label: String,
    pub utilization_pct: f64,
    pub resets_in_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamSlice {
    pub team_id: String,
    pub tokens: u64,
    /// List-price Cost in the window. Informational only.
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    /// The last poll succeeded.
    pub available: bool,
    /// When the last good reading was taken (unix seconds).
    pub observed_at: Option<i64>,
    pub session: Option<LimitView>,
    pub weekly: Option<LimitView>,
    pub model_scoped: Vec<LimitView>,
    pub band: ThresholdBand,
    pub braked: bool,
    pub auto_meter_enabled: bool,
    pub by_team: Vec<TeamSlice>,
    /// task_id -> lifetime tokens, for the board cards.
    pub tokens_by_task: HashMap<String, u64>,
}

fn view(l: &LimitReading, now: i64) -> LimitView {
    let label = match &l.kind {
        LimitKind::Session => "session (5h)".to_string(),
        LimitKind::Weekly => "weekly (7d)".to_string(),
        LimitKind::ModelWeekly { model } => format!("{model} weekly"),
    };
    let expired = now >= l.resets_at;
    LimitView {
        label,
        utilization_pct: if expired { 0.0 } else { l.utilization_pct },
        resets_in_secs: (l.resets_at - now).max(0),
    }
}

/// Assemble a snapshot. `braked` is Runtime's brake state; `now` is injected.
pub async fn compute_snapshot(
    worker: &WorkerUsageStore,
    util: &UtilizationStore,
    cfg: &UsageConfig,
    braked: bool,
    now: i64,
) -> Result<UsageSnapshot, SnapshotError> {
    let stored = util.load().await;
    let (mut session, mut weekly, mut model_scoped) = (None, None, Vec::new());
    if let Some(r) = &stored.reading {
        for l in &r.limits {
            match l.kind {
                LimitKind::Session => session = Some(view(l, now)),
                LimitKind::Weekly => weekly = Some(view(l, now)),
                LimitKind::ModelWeekly { .. } => model_scoped.push(view(l, now)),
            }
        }
    }
    let fraction = stored.reading.as_ref().and_then(|r| watched_fraction(r, now)).unwrap_or(0.0);
    let by_team = worker
        .team_breakdown(now - cfg.window_secs)
        .await?
        .into_iter()
        .map(|t| TeamSlice { team_id: t.team_id, tokens: t.tokens, cost_usd: t.cost_micros as f64 / 1_000_000.0 })
        .collect();
    let tokens_by_task = worker.tokens_by_task().await?.into_iter().collect();
    Ok(UsageSnapshot {
        available: stored.available(),
        observed_at: stored.reading.as_ref().map(|r| r.observed_at),
        session,
        weekly,
        model_scoped,
        band: band(fraction, braked),
        braked,
        auto_meter_enabled: cfg.auto_meter_enabled,
        by_team,
        tokens_by_task,
    })
}
```

`worker_log.rs`:
- Delete `api_window_tokens`.
- Change `TeamUsage` to `{ pub team_id: String, pub tokens: u64, pub cost_micros: u64 }`.
- Replace `team_breakdown`'s query and map with:

```rust
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT team_id, COALESCE(SUM(input_tokens + output_tokens), 0), COALESCE(SUM(cost_micros), 0)
             FROM worker_usage_log WHERE ts > ?
             GROUP BY team_id ORDER BY 2 DESC",
        )
        .bind(since_ts)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(team_id, t, c)| TeamUsage { team_id, tokens: t as u64, cost_micros: c as u64 }).collect())
```

`window.rs`: keep the module doc, `ThresholdBand` and `band`. Delete `window_pct`, `burn_per_min`, `reset_in_secs`, `est_brake_at` and every test of them.

`lib.rs` (`usage_telemetry`): remove `pub mod cc_log;`, `pub mod ingest;` and `pub mod transcript;`. Then `git rm src-tauri/usage_telemetry/src/{cc_log,ingest,transcript}.rs`.

`api.rs`:
- `UsageState` becomes `{ pub worker: Arc<WorkerUsageStore>, pub util: Arc<UtilizationStore>, pub pool: SqlitePool, pub is_braked: Arc<dyn Fn() -> bool + Send + Sync> }`. Imports: drop `cc_log`; add `use crate::utilization_store::UtilizationStore;`.
- `load_config` selects `window_secs, brake_on_pct, brake_off_pct, auto_meter_enabled` (four columns, no `window_budget`), maps into the new `UsageConfig`, and falls back to `UsageConfig::default()`.
- `usage_snapshot` calls `compute_snapshot(&state.worker, &state.util, &cfg, braked, now_unix())`.
- Delete `usage_set_budget` and `args::SetBudgetArgs`.
- In `tools()`:
  - delete the `usage_set_budget` `ToolSpec`;
  - set the `usage_snapshot` description to `"Return the current plan usage meter: session and weekly Utilization with reset times, per-team tokens and list-price Cost."`;
  - set the `usage_set_auto_meter` description to `"Enable or disable the auto-brake (brakes new work at 95% of the session or weekly limit)."`.
- Remove the `usage_set_budget` tests in `api.rs` and update `load_config` test expectations to the four-field config.

- [ ] **Step 4: Implement the app side.**

Create `src-tauri/app/migrations/017_drop_cc_usage_log.sql`:

```sql
-- 017_drop_cc_usage_log.sql — the meter reads real Utilization; transcript
-- token mirroring is gone.
DROP TABLE IF EXISTS cc_usage_log;
```

Register it in both migration lists:
- `(17, include_str!("../migrations/017_drop_cc_usage_log.sql")),`
- `Migration { version: 17, description: "drop transcript usage mirror", sql: include_str!("../migrations/017_drop_cc_usage_log.sql"), kind: MigrationKind::Up },`

Change the count test to `assert_eq!(version, 17, "all seventeen migrations recorded");`.

`pipeline_activator.rs`:
- Add to `WorkerDeps`:

```rust
    /// Wakes the utilization poller after a worker step settles.
    pub usage_poll: Option<Arc<tokio::sync::Notify>>,
```

- In `spawn_transformer_loop`, next to `let handle = self.handle.clone();`, add `let usage_poll = self.deps.usage_poll.clone();`.
- In the `if settled { … }` block, after the `USAGE_CHANGED` emit, add `if let Some(n) = &usage_poll { n.notify_one(); }`.

`lib.rs` (app), usage wiring block (the one that builds `cc_store`, `worker_usage` and `ingestor`). Replace it with:

```rust
                use usage_telemetry::utilization_store::UtilizationStore;
                use usage_telemetry::worker_log::WorkerUsageStore;
                let worker_usage = Arc::new(WorkerUsageStore::new(pool.clone()));
                let util_store = Arc::new(UtilizationStore::new(pool.clone()));
                let usage_poll = Arc::new(tokio::sync::Notify::new());
                let usage_sink: Arc<dyn agent_bus_core::UsageSink> = worker_usage.clone();
                let usage_state_arc = Arc::new(usage_telemetry::api::UsageState {
                    worker: worker_usage.clone(), util: util_store.clone(), pool: pool.clone(),
                    is_braked: Arc::new({ let b = brake.clone(); move || b.is_on() }),
                });
                handle.manage(usage_telemetry::api::UsageState {
                    worker: worker_usage.clone(),
                    util: util_store.clone(),
                    pool: pool.clone(),
                    is_braked: Arc::new({ let b = brake.clone(); move || b.is_on() }),
                });
```

Next:
- In the `WorkerDeps { … }` literal, add `usage_poll: Some(usage_poll.clone()),`.
- In the tool dispatcher's `"usage_snapshot"` arm, call `compute_snapshot(&self.usage.worker, &self.usage.util, &cfg, braked, now_unix())`.
- Delete `fn cc_claude_projects_dir()`.
- Remove `usage_telemetry::api::usage_set_budget,` from `generate_handler!`.

Replace **both** the "Boot backfill" block and the "Auto-meter sweep" block with the poller:

```rust
                // Utilization poller: reads the account's real plan usage from
                // claude (no model call), stores it, and applies the auto-brake.
                // Runs at boot, every 60s, and when a worker step settles (at
                // most once per 10s).
                {
                    let util = util_store.clone();
                    let brake = brake.clone();
                    let pool = pool.clone();
                    let handle = handle.clone();
                    let brake_store = brake_store.clone();
                    let usage_poll = usage_poll.clone();
                    let source: Arc<dyn agent_bus_core::UtilizationSource> =
                        Arc::new(runners::usage_query::ClaudeCliUtilizationSource::new());
                    tauri::async_runtime::spawn(async move {
                        use usage_telemetry::api::load_config;
                        use usage_telemetry::brake_policy::{decide_utilization, BrakeDecision, AUTO_METER_REASON};
                        let mut last_attempt: Option<std::time::Instant> = None;
                        loop {
                            if let Some(t) = last_attempt {
                                if t.elapsed() < std::time::Duration::from_secs(10) {
                                    tokio::time::sleep(std::time::Duration::from_secs(10) - t.elapsed()).await;
                                }
                            }
                            last_attempt = Some(std::time::Instant::now());
                            let now = now_unix();
                            let src = source.clone();
                            match tokio::task::spawn_blocking(move || src.fetch()).await {
                                Ok(Ok(reading)) => util.record_ok(&reading, now).await,
                                Ok(Err(e)) => util.record_err(&e, now).await,
                                Err(e) => util.record_err(&format!("usage query failed: {e}"), now).await,
                            }
                            let cfg = load_config(&pool).await;
                            if cfg.auto_meter_enabled {
                                let stored = util.load().await;
                                let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
                                match decide_utilization(stored.reading.as_ref(), stored.available(), now, auto_on, cfg.brake_on_pct, cfg.brake_off_pct) {
                                    BrakeDecision::SetOn(reason) => {
                                        // A soft brake: blocks new claims, never kills in-flight work.
                                        brake.set_on(reason.as_str());
                                        let _ = brake_store.save(true, Some(&reason), now).await;
                                    }
                                    BrakeDecision::Release => {
                                        brake.set_off();
                                        let _ = brake_store.save(false, None, now).await;
                                    }
                                    BrakeDecision::NoChange => {}
                                }
                            }
                            let _ = handle.emit(crate::events::USAGE_CHANGED, ());
                            tokio::select! {
                                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
                                _ = usage_poll.notified() => {}
                            }
                        }
                    });
                }
```

If the app crate does not yet depend on `runners` directly, it does: `GitCliWorktreeProvider` and `runners::probe` are used in `lib.rs`. If `tokio::select!` needs the `macros` feature and the build says so, add it to the app crate's tokio features, and record a ruling.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
Expected: all pass. The snapshot tests, the contract tests and the migration count test (17) are included. `cargo build` emits no unused-import warnings from the removed modules.

- [ ] **Step 6: Commit**

```bash
git add -A src-tauri
git commit -m "feat(usage): meter + auto-brake read real Utilization; remove the token estimate"
```

---

### Task 6: Frontend — meter, Settings, IPC

**Files:**
- Modify: `src/ipc/usage.ts`, `src/ipc/usage.test.ts`
- Modify: `src/lib/usageMeter.ts`, `src/lib/usageMeter.test.ts`
- Modify: `src/components/UsageMeter.tsx`, `src/components/UsageMeter.test.tsx`
- Modify: `src/components/SettingsView.tsx`, `src/components/SettingsView.test.tsx`
- Modify: `src/App.tsx` (drop `setBudget` / `onSetBudget`)
- Modify: `src/components/Topbar.test.tsx`, `src/hooks/useUsage.test.ts` (fixtures)
- Modify: `DOMAIN.md` (delete the two *(retiring)* entries, Budget and Burn rate)

**Interfaces:**
- Consumes: Task 5's snapshot JSON.
- Produces:
  - TS `UsageSnapshot`, `LimitView` and `TeamSlice`, mirroring Task 5;
  - `usageMeter.ts`: `bandColorVar`, `resetCountdown`, `isStale(observedAt, now)`, `asOf(observedAt)`, `formatUsd(n)`.

The shared fixture used by every test below (paste it into each test file that needs one):

```ts
const snap = (over: Partial<UsageSnapshot> = {}): UsageSnapshot => ({
  available: true,
  observed_at: 1_000,
  session: { label: "session (5h)", utilization_pct: 41, resets_in_secs: 3_600 },
  weekly: { label: "weekly (7d)", utilization_pct: 2, resets_in_secs: 86_400 },
  model_scoped: [{ label: "Fable weekly", utilization_pct: 0, resets_in_secs: 86_400 }],
  band: "safe",
  braked: false,
  auto_meter_enabled: false,
  by_team: [{ team_id: "research", tokens: 240, cost_usd: 0.25 }],
  tokens_by_task: {},
  ...over,
});
```

(`import type { UsageSnapshot } from "<relative>/ipc/usage";` where the file doesn't already import it.)

- [ ] **Step 1: Write the failing tests.**

`src/lib/usageMeter.test.ts`:
- Delete the `brakeEta`, `windowLine` and `formatBurn` tests.
- Keep the `bandColorVar` and `resetCountdown` tests.
- Add:

```ts
import { isStale, asOf, formatUsd } from "./usageMeter";

describe("isStale", () => {
  it("is false within 3 minutes and true after", () => {
    expect(isStale(1_000, 1_000 + 180)).toBe(false);
    expect(isStale(1_000, 1_000 + 181)).toBe(true);
  });
  it("is true when there has never been a reading", () => {
    expect(isStale(null, 1_000)).toBe(true);
  });
});

describe("asOf", () => {
  it("formats the reading time as HH:MM local", () => {
    const d = new Date(2026, 9, 7, 14, 32);
    expect(asOf(Math.floor(d.getTime() / 1000))).toBe("as of 14:32");
  });
});

describe("formatUsd", () => {
  it("shows cents, and <$0.01 for tiny non-zero cost", () => {
    expect(formatUsd(0.25)).toBe("$0.25");
    expect(formatUsd(0.004)).toBe("<$0.01");
    expect(formatUsd(0)).toBe("$0.00");
  });
});
```

Replace the body of `src/components/UsageMeter.test.tsx` (keep its imports, adding `UsageSnapshot`), using the fixture above:

```tsx
describe("UsageMeter", () => {
  it("headlines the session Utilization", () => {
    render(<UsageMeter snapshot={snap()} now={1_010} />);
    expect(screen.getByRole("group", { name: "usage 41% of session limit" })).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "41");
  });

  it("shows a dash when there has never been a reading", () => {
    render(<UsageMeter snapshot={snap({ observed_at: null, session: null, weekly: null, model_scoped: [], available: false })} now={1_010} />);
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("says usage unavailable and shows the reading's age when the last poll failed", () => {
    render(<UsageMeter snapshot={snap({ available: false })} now={1_010} />);
    expect(screen.getByText(/— usage unavailable/)).toBeInTheDocument();
    expect(screen.getAllByText(/as of/).length).toBeGreaterThan(0);
  });

  it("shows 'as of' once a good reading is over 3 minutes old", () => {
    render(<UsageMeter snapshot={snap()} now={1_000 + 600} />);
    expect(screen.getAllByText(/as of/).length).toBeGreaterThan(0);
  });

  it("tooltip lists both windows, model-scoped limits and per-team cost", () => {
    render(<UsageMeter snapshot={snap()} now={1_010} />);
    expect(screen.getByText("session (5h)")).toBeInTheDocument();
    expect(screen.getByText("weekly (7d)")).toBeInTheDocument();
    expect(screen.getByText("Fable weekly")).toBeInTheDocument();
    expect(screen.getByText("cost this 5h (list price)")).toBeInTheDocument();
    expect(screen.getByText(/\$0\.25/)).toBeInTheDocument();
  });
});
```

`src/components/SettingsView.test.tsx`:
- Replace the old fixture with `snap()`.
- Delete the two budget tests ("pre-fills the recalibrated default budget…" and "calls onSetBudget…").
- Remove `onSetBudget` from `baseProps`.
- Add:

```tsx
  it("has no budget setting and describes the real-usage auto-brake", () => {
    render(<SettingsView {...baseProps({})} />);
    expect(screen.queryByLabelText(/window budget/i)).not.toBeInTheDocument();
    expect(screen.getByText(/brakes new work at 95% of the session or weekly limit/)).toBeInTheDocument();
  });
```

`src/ipc/usage.test.ts`: delete the `setBudget` test and its import, and make the `usageSnapshot` mock resolve `snap()`.

`src/components/Topbar.test.tsx` and `src/hooks/useUsage.test.ts`: replace their fixture literals with `snap()`.
- In `useUsage.test.ts`, the helper `snap(pct)` becomes `(pct: number) => snap({ session: { label: "session (5h)", utilization_pct: pct * 100, resets_in_secs: 1 } })`; rename the shared fixture `base` there to avoid the name clash.
- Its assertions `snapshot?.window_pct).toBe(0.5)` become `snapshot?.session?.utilization_pct).toBe(50)`, and likewise for `0.2` → `20` and `0.8` → `80`.

- [ ] **Step 2: Run them to verify they fail**

Run: `npx vitest run src/lib/usageMeter.test.ts src/components/UsageMeter.test.tsx src/components/SettingsView.test.tsx src/ipc/usage.test.ts src/components/Topbar.test.tsx src/hooks/useUsage.test.ts`
Expected: failures. Examples:
- `isStale is not a function`;
- no group named `usage 41% of session limit`;
- `— usage unavailable` not found;
- the Settings budget field still present.

- [ ] **Step 3: Implement.**

`src/ipc/usage.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";

export type ThresholdBand = "safe" | "warn" | "hot" | "braked";

/// One plan Limit as the meter shows it.
export interface LimitView {
  label: string;
  utilization_pct: number;
  resets_in_secs: number;
}

export interface TeamSlice {
  team_id: string;
  tokens: number;
  /// List-price Cost in the current 5-hour window. Informational only.
  cost_usd: number;
}

export interface UsageSnapshot {
  available: boolean;
  observed_at: number | null;
  session: LimitView | null;
  weekly: LimitView | null;
  model_scoped: LimitView[];
  band: ThresholdBand;
  braked: boolean;
  auto_meter_enabled: boolean;
  by_team: TeamSlice[];
  tokens_by_task: Record<string, number>;
}

export async function usageSnapshot(): Promise<UsageSnapshot> {
  return await invoke<UsageSnapshot>("usage_snapshot");
}

export async function setAutoMeter(enabled: boolean): Promise<UsageSnapshot> {
  return await invoke<UsageSnapshot>("usage_set_auto_meter", { enabled });
}
```

`src/lib/usageMeter.ts`: keep `bandColorVar` and `resetCountdown`, delete `windowLine`, `formatBurn` and `brakeEta`, and add:

```ts
/// A reading older than this shows its age.
const STALE_SECS = 180;

export function isStale(observedAt: number | null, now: number): boolean {
  return observedAt == null || now - observedAt > STALE_SECS;
}

export function asOf(observedAt: number): string {
  const d = new Date(observedAt * 1000);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `as of ${hh}:${mm}`;
}

export function formatUsd(n: number): string {
  if (n > 0 && n < 0.01) return "<$0.01";
  return `$${n.toFixed(2)}`;
}
```

Remove the now-unused `formatTokens` import from `usageMeter.ts` if nothing else in it uses it.

`src/components/UsageMeter.tsx`: replace the component body with:

```tsx
import type { LimitView, UsageSnapshot } from "../ipc/usage";
import { formatTokens } from "../lib/cost";
import { asOf, bandColorVar, formatUsd, isStale, resetCountdown } from "../lib/usageMeter";

export interface UsageMeterProps {
  snapshot: UsageSnapshot | null;
  /// Epoch seconds reference for staleness. Defaults to the wall clock; tests pass a fixed value.
  now?: number;
}

// The one sanctioned gradient (DESIGN.md §Usage meter): --running -> --warn ->
// --danger at 0% / 60% / 90% of the 100px track.
const GRADIENT =
  "linear-gradient(90deg, var(--running) 0%, var(--warn) 60%, var(--danger) 90%)";

export function UsageMeter({ snapshot, now = Math.floor(Date.now() / 1000) }: UsageMeterProps) {
  const session = snapshot?.session ?? null;
  const pct = session ? Math.round(session.utilization_pct) : 0;
  const band = snapshot?.band ?? "safe";
  const color = bandColorVar(band);
  const braked = snapshot?.braked ?? false;
  const observedAt = snapshot?.observed_at ?? null;
  const unavailable = snapshot != null && !snapshot.available;
  const showAge = observedAt != null && (unavailable || isStale(observedAt, now));

  return (
    <div
      data-testid="usage-meter"
      tabIndex={0}
      role="group"
      aria-label={`usage ${pct}% of session limit`}
      style={{
        position: "relative", display: "flex", alignItems: "center", gap: "var(--sp-3)",
        padding: "4px 12px", background: "var(--bg-2)",
        border: `1px solid ${band === "hot" || braked ? "var(--danger)" : "var(--border)"}`,
        borderRadius: "var(--r-md)", opacity: braked ? 0.6 : 1, cursor: "default",
      }}
      className="usage-meter-hoverable"
    >
      <div data-testid="usage-bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}
        style={{ width: 100, height: 6, background: "var(--surface-3)", borderRadius: 3, overflow: "hidden" }}>
        <div style={{ height: "100%", width: `${Math.min(pct, 100)}%`, overflow: "hidden" }}>
          <div data-testid="usage-bar-fill" style={{ height: "100%", width: 100, background: GRADIENT }} />
        </div>
      </div>

      <div style={{ fontSize: 11, color: "var(--text-2)", display: "flex", alignItems: "center", gap: 6, fontVariantNumeric: "tabular-nums" }}>
        {session ? <span style={{ color, fontWeight: 500 }}>{pct}%</span> : <span style={{ color: "var(--text-3)" }}>—</span>}
        {unavailable && session && <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>— usage unavailable</span>}
        {showAge && observedAt != null && <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>· {asOf(observedAt)}</span>}
        {!showAge && session && (
          <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>· {resetCountdown(session.resets_in_secs)}</span>
        )}
      </div>

      {snapshot && (
        <div className="usage-tooltip" style={{
          position: "absolute", top: "calc(100% + 8px)", right: 0, minWidth: 260,
          background: "var(--surface-3)", border: "1px solid var(--border-2)", borderRadius: "var(--r-md)",
          padding: "12px 14px", boxShadow: "var(--shadow-card)", zIndex: 20, textAlign: "left",
        }}>
          <div style={{ fontSize: 11.5, color: "var(--text)", fontWeight: 500, marginBottom: 10 }}>claude plan usage</div>
          {[snapshot.session, snapshot.weekly, ...snapshot.model_scoped]
            .filter((l): l is LimitView => l != null)
            .map((l) => (
              <Row key={l.label} label={l.label} value={`${Math.round(l.utilization_pct)}% · ${resetCountdown(l.resets_in_secs)}`} />
            ))}
          {observedAt != null && <Row label="reading" value={unavailable ? `${asOf(observedAt)} · unavailable` : asOf(observedAt)} />}
          <Row label="auto-brake" value={snapshot.auto_meter_enabled ? "on" : "off"} />
          <div style={{ marginTop: 10, paddingTop: 10, borderTop: "1px solid var(--border)" }}>
            <div style={{ color: "var(--text-3)", fontSize: 10.5, marginBottom: 6 }}>cost this 5h (list price)</div>
            {snapshot.by_team.length === 0 ? (
              <div style={{ color: "var(--text-4)", fontSize: 11 }}>no usage yet</div>
            ) : (
              snapshot.by_team.map((t) => (
                <div key={t.team_id} style={{ display: "flex", justifyContent: "space-between", padding: "2px 0", fontSize: 11, color: "var(--text-3)" }}>
                  <span>{t.team_id}</span>
                  <span style={{ color: "var(--text-2)" }}>{formatUsd(t.cost_usd)} · {formatTokens(t.tokens)} tok</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: "var(--text-3)", padding: "3px 0" }}>
      <span>{label}</span>
      <span style={{ color: "var(--text-2)", fontVariantNumeric: "tabular-nums" }}>{value}</span>
    </div>
  );
}
```

`src/components/SettingsView.tsx`:
- Remove `onSetBudget` from the props interface and the destructure.
- Remove `budgetInput` / `setBudgetInput`, `saving` / `setSaving` (if used only by the budget) and the `saveBudget` function.
- In the usage section, delete everything from the `window budget` label through the `currently … used this window.` block.
- Change the checkbox label text to `auto-brake on plan usage`, and the hint under it to `brakes new work at 95% of the session or weekly limit; releases below 85%. manual and rate-limit braking stay on either way.`.
- Remove the `formatTokens` import if it is now unused.

`src/App.tsx`: change the import to `import { setAutoMeter } from "./ipc/usage";` and delete the `onSetBudget={setBudget}` prop.

`DOMAIN.md`: delete the two lines starting `- **Budget** *(retiring)*` and `- **Burn rate** *(retiring)*`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npx vitest run && npx tsc --noEmit`
Expected: all test files pass; `tsc` prints nothing.

- [ ] **Step 5: Commit**

```bash
git add -A src DOMAIN.md
git commit -m "feat(ui): usage meter shows real session/weekly Utilization and per-team Cost"
```

---

### Task 7: Whole-branch verification + findings log

**Files:**
- Modify: `docs/live-test-findings-2026-06-25.md` (LF35 status)

- [ ] **Step 1: Full suites**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --workspace && npx vitest run && npx tsc --noEmit`
Expected: every test passes; no type errors.

- [ ] **Step 2: Real `get_usage` smoke** (real CLI, no model call, no app):

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p runners usage_query
printf '%s\n' '{"type":"control_request","request_id":"u1","request":{"subtype":"get_usage"}}' | claude -p --input-format stream-json --output-format stream-json --verbose --no-session-persistence | grep -c '"control_response"'
```

Expected: tests pass; the count is `1`.

- [ ] **Step 3: Update LF35.** In `docs/live-test-findings-2026-06-25.md`, change the LF35 heading's status from `` `open` `` to `` `fixed (branch spec/real-utilization)` ``.

- [ ] **Step 4: Commit**

```bash
git add docs/live-test-findings-2026-06-25.md
git commit -m "docs(findings): LF35 fixed by the real Utilization meter"
```

The in-app check (meter matches claude.ai within a point or two; auto-brake behaviour) is the resumed meal-planner spike's first step, not part of this plan.
