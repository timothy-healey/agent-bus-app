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
