-- 007_invocation_audit.sql — R3 (per-invocation persistence / audit). Adds a
-- durable, append-only audit trail: one row per Invocation (DOMAIN.md → Runners
-- "Invocation": one Claude call). Written by Runtime's WorkerPool at invoke-start
-- (outcome NULL = in-flight) and updated at settle with the verdict/error class +
-- usage. Standalone root — never joined into the tasks UPDATE; no cross-aggregate
-- transaction. Migrations 001–006 are never edited.
--
-- An in-flight row (outcome IS NULL) left by a Runtime crash is the intended
-- "started but never completed" signal; crash *recovery* stays Task-claim release
-- (release_orphaned_running). v1.1 adds no reconciliation (a viewer is out of scope).
CREATE TABLE IF NOT EXISTS invocation_audit (
  invocation_id   TEXT PRIMARY KEY,             -- fresh uuid per invocation
  task_id         TEXT NOT NULL,
  team_id         TEXT NOT NULL,
  model           TEXT NOT NULL,
  attempts        INTEGER NOT NULL DEFAULT 0,    -- the task's attempt no. at invoke time
  started_at      INTEGER NOT NULL,
  settled_at      INTEGER,                       -- NULL while in-flight
  outcome_kind    TEXT,                          -- NULL | 'verdict' | 'error'
  outcome         TEXT,                          -- NULL | 'approve'|'revise'|'reject' | 'error:<class>'
  input_tokens    INTEGER NOT NULL DEFAULT 0,
  output_tokens   INTEGER NOT NULL DEFAULT 0,
  cache_creation  INTEGER NOT NULL DEFAULT 0,
  cache_read      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_invocation_audit_task ON invocation_audit(task_id);
CREATE INDEX IF NOT EXISTS idx_invocation_audit_started ON invocation_audit(started_at);
