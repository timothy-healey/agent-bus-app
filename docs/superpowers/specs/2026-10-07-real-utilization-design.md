# Spec — Real Utilization meter + per-team Cost

*Design doc. Wayfinder map: "Map: runner capabilities from the current Claude CLI" (#3). Ticket: "Spec: real Utilization meter + per-team Cost" (#4). The meter and the auto-brake read the subscription's real **Utilization** for each **Limit**, polled for free from the `claude` CLI, instead of estimating from token counts. Each team's **Cost** per window is shown for information. Terms are as defined in `DOMAIN.md` (Usage Telemetry).*

## Why this exists

The meter divides token counts (from Claude Code transcripts) by a calibrated budget. In the watched meal-planner run it read **83%** while claude.ai showed **41%** (LF35). Because the auto-brake fires at 95% of that estimate, it would stop a run with more than half the window unused. Token counts are not a stable proxy for plan usage: the model mix and caching change the ratio.

The CLI can report the real figure. Sending the SDK control request `get_usage` to `claude -p --input-format stream-json --output-format stream-json --verbose` returns the account's Utilization for every Limit, with reset times. It calls no model, costs nothing against the plan, and takes about 1.6s. Verified on `claude` 2.1.289.

## Decisions

1. **Source: poll `get_usage`.** Stream-json `rate_limit_event` parsing, and a paid probe call, are not used.
2. **Cadence:** every **60s** while the app runs, plus once after a worker invocation settles, debounced to at most one call per **10s**.
3. **The brake watches the session and weekly Limits.** It fires when either reaches **95%** and releases when both are below **85%**.
   - Model-scoped weekly Limits are shown in the tooltip but never braked on.
   - A Limit whose `resets_at` has passed counts as 0% until the next reading.
4. **Failure is visible, never guessed.** When a poll fails or the response has no rate limits, the meter shows "— usage unavailable" with the last good reading's age, and the brake keeps its current state. There is no token-estimate fallback.
5. **Per-team Cost.** Each worker invocation's `result.total_cost_usd` is stored on its usage row. The tooltip lists Cost per team for the **current 5-hour window**, labelled "list price". Cost never drives the brake.
6. **Removed:**
   - transcript ingestion (`ingest.rs`, `transcript.rs`, `cc_log.rs`, the `cc_usage_log` table and its 15s scan);
   - the token-estimate %, the window budget, burn rate and `est_brake_at`;
   - the `usage_set_budget` command, the budget field in Settings, and `setBudget` in the IPC.

   Kept: the brake thresholds, the auto-brake toggle, per-task token counts (shown on cards) and per-team token usage.

## Architecture

### Shared kernel (`agent_bus_core`)

```rust
pub enum LimitKind { Session, Weekly, ModelWeekly { model: String } }

pub struct LimitReading {
    pub kind: LimitKind,
    pub utilization_pct: f64,   // 0–100, as reported
    pub resets_at: i64,         // unix seconds
}

pub struct UtilizationReading {
    pub observed_at: i64,
    pub limits: Vec<LimitReading>,
}

pub trait UtilizationSource: Send + Sync {
    /// One fresh reading of the account's Limits, or why there isn't one.
    fn fetch(&self) -> Result<UtilizationReading, String>;
}
```

The trait lives in the kernel for the same reason `UsageSink` does: Usage Telemetry consumes it without depending on Runners.

`UsageEvent` gains `cost_usd: Option<f64>`.

### Runners (ACL) — the only code that knows `get_usage`

- **`runners::usage_query::parse_get_usage(stdout: &str, now: i64) -> Result<UtilizationReading, String>`.** Pure. It finds the `control_response` line and maps the fields as follows:

  | `get_usage` field | Becomes |
  |---|---|
  | `rate_limits.five_hour` | `Session` |
  | `rate_limits.seven_day` | `Weekly` |
  | each `rate_limits.model_scoped[]` entry | `ModelWeekly { model: display_name }` |

  `resets_at` is parsed from RFC 3339. The parse errs on `rate_limits_available: false`, a missing `rate_limits`, or an error `control_response`.
- **`ClaudeCliUtilizationSource`** implements `UtilizationSource`. It spawns `claude -p --input-format stream-json --output-format stream-json --verbose --no-session-persistence` and writes `{"type":"control_request","request_id":"u1","request":{"subtype":"get_usage"}}` to stdin before closing it. It has a **10s timeout**, then calls `parse_get_usage`.
- **Cost capture:** `StreamAccumulator` reads `result.total_cost_usd` into `RunnerUsage.cost_usd: Option<f64>`. The engine copies it into `UsageEvent.cost_usd`.

### Usage Telemetry

- **`utilization_state` (migration 016)** is a one-row table, `id = 1`, holding:
  - `reading_json`: the last good `UtilizationReading`;
  - `last_ok_at`;
  - `last_error`;
  - `last_attempt_at`.

  It survives restarts, so the meter has a value at boot.
- **`brake_policy::decide_utilization(reading: Option<&UtilizationReading>, available: bool, now, auto_on, on_pct, off_pct) -> BrakeDecision`.** Pure.
  - `watched` = the effective % of the Session and Weekly limits, where a Limit is 0 once `now >= resets_at`.
  - `SetOn` when `max(watched) >= on_pct`.
  - `Release` when `auto_on` and `max(watched) < off_pct`.
  - `NoChange` when there is no reading or `!available`.
  - Units: `usage_config` stores the thresholds as fractions (`0.95`, `0.85`), while `get_usage` reports percentages. Compare `utilization_pct / 100` against the stored fraction. `ThresholdBand::band` likewise takes the fraction.
- **`UsageSnapshot`** (replaces the token-window snapshot):
  ```rust
  pub struct LimitView { pub label: String, pub utilization_pct: f64, pub resets_in_secs: i64 }
  pub struct UsageSnapshot {
      pub available: bool,                 // last poll succeeded
      pub observed_at: Option<i64>,        // last good reading
      pub session: Option<LimitView>,
      pub weekly: Option<LimitView>,
      pub model_scoped: Vec<LimitView>,
      pub band: ThresholdBand,             // from max(session, weekly); braked overrides
      pub braked: bool,
      pub auto_meter_enabled: bool,
      pub cost_by_team: Vec<TeamCost>,     // current 5-hour window
      pub tokens_by_task: Vec<(String, u64)>,
  }
  pub struct TeamCost { pub team_id: String, pub cost_usd: f64 }
  ```
- **`worker_usage_log`** gains `cost_usd REAL NULL` (migration 016). `team_cost(since)` sums it per team.
- **`usage_config`** keeps `window_secs`, `brake_on_pct`, `brake_off_pct` and `auto_meter_enabled`. `window_budget` is no longer read. The column is left in place, because SQLite column drops add risk for no gain.

### Composition root (`app`)

- **The utilization poller** replaces the 15s transcript sweep. One spawned loop:
  1. wait 60s, or until a `poll_now` notify;
  2. `fetch` via `spawn_blocking`;
  3. store the result (success or error);
  4. if auto-brake is enabled, run `decide_utilization` and apply it exactly as the sweep does today (soft brake, `brake_store.save`, emit);
  5. emit `usage-changed`.

  It also polls once at boot, before the first wait.
- **Settle trigger:** the activator's task-settle path calls `poll_now`. The loop ignores triggers less than 10s after its last attempt.
- The `cc_*` wiring and the boot transcript backfill are deleted.

### Frontend

- **`UsageMeter`** headline: the session %. The bar uses the band colour.
  - A small "as of HH:MM" appears only when `available` is false, or the reading is more than 3 minutes old.
  - "—" appears when there has never been a reading.
- **Tooltip rows:**
  - Session (5h): % · resets in …
  - Weekly (7d): % · resets in …
  - one row per model-scoped Limit;
  - "cost this 5h (list price)" by team;
  - auto-brake on/off.
- **`SettingsView`:** the budget input and "currently X of Y used" are removed. The auto-brake checkbox stays, with its text: "brakes new work at 95% of the session or weekly limit".
- **`src/ipc/usage.ts`:** the types mirror the new snapshot; `setBudget` is removed.

## Data flow

```
boot ─┬─ poll ─► ClaudeCliUtilizationSource.fetch ─► parse_get_usage ─► utilization_state
      └─ every 60s / poll_now (≥10s apart) ─┘                │
                                                         decide_utilization ─► brake (soft)
worker invocation ─► result.total_cost_usd ─► UsageEvent.cost_usd ─► worker_usage_log
usage-changed ─► frontend usage_snapshot() ─► UsageMeter
```

## Error handling

| Case | Behaviour |
|---|---|
| `claude` missing, times out (10s) or exits non-zero | `last_error` set, `available = false`, meter "— usage unavailable", brake unchanged |
| `rate_limits_available: false` or `rate_limits` missing | Same as above, with error text "usage not reported for this account" |
| Older CLI without `get_usage` (error `control_response`) | Same as above, with the CLI's error text |
| Reading older than its `resets_at` | That Limit reads 0% until the next poll |
| `total_cost_usd` absent from a result | `cost_usd` NULL; that team's Cost excludes the invocation |

## Testing

- **`parse_get_usage`:** against a sanitised fixture of a real response (`runners/src/fixtures/get-usage-sample.jsonl`, with no account or org fields). It also covers `rate_limits_available: false`, an error `control_response`, and a missing `model_scoped`.
- **`decide_utilization`:** the session crosses 95%; the weekly crosses 95%; the release needs both below 85%; an expired `resets_at` counts as 0; no reading means `NoChange`; unavailable means `NoChange`; auto off means `NoChange`.
- **Snapshot assembly:** the band comes from max(session, weekly); `available` and `observed_at` pass through; `cost_by_team` covers only the current 5-hour window.
- **Poller:** with a fake `UtilizationSource`, a failure keeps the last good reading and sets `available = false`, and `poll_now` within 10s is ignored.
- **Cost capture:** a stream fixture with `total_cost_usd` lands in `RunnerUsage` and `UsageEvent`.
- **Migration 016:** the column and table are present, `cc_usage_log` is dropped, and the migration-count test is bumped.
- **Frontend:** the meter renders the headline, "as of" and "—" states; the tooltip shows both windows, model-scoped rows and per-team Cost; Settings has no budget input.

## Out of scope

- Braking on model-scoped Limits.
- Cost for god-terminal chat turns. Only worker invocations carry a team.
- Extra-usage and credit fields from `get_usage`.
