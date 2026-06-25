-- 005_usage.sql — Plan 5 (Usage Telemetry). The two usage logs per the design
-- spec's SQLite schema, plus a one-row budget config. Migrations 001–004 are
-- never edited.

-- Usage attributed to our own worker invocations (per-team breakdown +
-- per-task card cost source). `runner` distinguishes the CLI tail (v1) from the
-- API tail (v1.1) so the window formula can include only API rows from here.
CREATE TABLE IF NOT EXISTS worker_usage_log (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  ts              INTEGER NOT NULL,
  team_id         TEXT NOT NULL,
  task_id         TEXT,
  model           TEXT NOT NULL,
  runner          TEXT NOT NULL DEFAULT 'cli',   -- 'cli' (v1) | 'api' (v1.1)
  input_tokens    INTEGER NOT NULL DEFAULT 0,
  output_tokens   INTEGER NOT NULL DEFAULT 0,
  cache_creation  INTEGER NOT NULL DEFAULT 0,
  cache_read      INTEGER NOT NULL DEFAULT 0
);

-- Mirror of Claude Code's transcript usage (window-total source). message_id is
-- UNIQUE so a re-read of the same transcript line is deduplicated.
CREATE TABLE IF NOT EXISTS cc_usage_log (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  ts              INTEGER NOT NULL,
  message_id      TEXT NOT NULL UNIQUE,
  model           TEXT,
  input_tokens    INTEGER NOT NULL DEFAULT 0,
  output_tokens   INTEGER NOT NULL DEFAULT 0,
  cache_creation  INTEGER NOT NULL DEFAULT 0,
  cache_read      INTEGER NOT NULL DEFAULT 0
);

-- Single-row meter configuration (budget denominator + thresholds). This gives
-- the meter a denominator and the brake policy its thresholds. Seeded with the
-- all-tokens calibration (LF34): the window counts input+output+cache_creation+
-- cache_read, and cache_read dominates real throughput — ~67.2M live ≈ 35% on
-- claude.ai ⟹ ~192M, rounded to 190_000_000. Tunable estimate, NOT an exact
-- claude.ai mirror (G6); the operator adjusts it in Settings → Usage.
CREATE TABLE IF NOT EXISTS usage_config (
  id                  INTEGER PRIMARY KEY CHECK (id = 1),
  window_budget       INTEGER NOT NULL DEFAULT 190000000,
  window_secs         INTEGER NOT NULL DEFAULT 18000,    -- 5h
  brake_on_pct        REAL NOT NULL DEFAULT 0.95,
  brake_off_pct       REAL NOT NULL DEFAULT 0.85,
  auto_meter_enabled  INTEGER NOT NULL DEFAULT 0          -- v1: OFF (D8)
);
INSERT OR IGNORE INTO usage_config (id) VALUES (1);

CREATE INDEX IF NOT EXISTS idx_worker_usage_ts ON worker_usage_log(ts);
CREATE INDEX IF NOT EXISTS idx_worker_usage_task ON worker_usage_log(task_id);
CREATE INDEX IF NOT EXISTS idx_cc_usage_ts ON cc_usage_log(ts);
