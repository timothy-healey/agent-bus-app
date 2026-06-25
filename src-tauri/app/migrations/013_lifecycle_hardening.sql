-- 013_lifecycle_hardening.sql — Lifecycle hardening (LH4 + LH6).
-- Append-only; migrations 001–012 are never edited. Additive only.

-- LH4: durable record of a spawned `claude` process group, so an UNCLEAN app
-- death (the in-memory ProcessRegistry is lost) can still be reaped on the next
-- boot before recovery re-queues the task. Deregister deletes the row; a clean
-- exit therefore leaves none. started_ts is the PID-reuse guard: on boot we only
-- signal a pgid whose process start-time still matches (a recycled pid won't).
CREATE TABLE IF NOT EXISTS live_processes (
  pgid        INTEGER PRIMARY KEY,
  run_id      TEXT,
  task_id     TEXT,
  started_ts  INTEGER NOT NULL
);

-- LH6: durable brake state, so an explicit Stop survives app exit/reboot. One
-- row. The runtime Brake aggregate stays persistence-unaware; the root writes
-- this on every set_on/set_off and reads it (reason-aware) on boot.
CREATE TABLE IF NOT EXISTS brake_state (
  id      INTEGER PRIMARY KEY CHECK (id = 1),
  on_flag INTEGER NOT NULL DEFAULT 0,
  reason  TEXT,
  ts      INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO brake_state (id, on_flag, reason, ts) VALUES (1, 0, NULL, 0);
