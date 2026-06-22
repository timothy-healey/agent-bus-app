-- 003_runtime.sql — Plan 3 (Runtime + Review persistence). Tasks, workers,
-- comments per the design spec's SQLite schema. 001/002 are untouched.

CREATE TABLE IF NOT EXISTS tasks (
  id               TEXT PRIMARY KEY,
  project_id       TEXT NOT NULL,
  pipeline         TEXT NOT NULL,
  topic            TEXT NOT NULL,
  target_repo      TEXT,
  target_scope     TEXT,
  current_stage    TEXT NOT NULL,            -- team id or gate id
  state            TEXT NOT NULL,            -- queued|running|gated|revising|needs_human|done|braked
  attempts         INTEGER NOT NULL DEFAULT 1,
  parent_artifact  TEXT,
  review_artifact  TEXT,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  FOREIGN KEY(project_id) REFERENCES projects(id)
);

CREATE TABLE IF NOT EXISTS workers (
  id          TEXT PRIMARY KEY,
  team_id     TEXT NOT NULL,
  task_id     TEXT,                          -- null when idle
  pid         INTEGER,                        -- subprocess pid (null in v1: ephemeral)
  started_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS comments (
  id             TEXT PRIMARY KEY,
  task_id        TEXT NOT NULL,
  artifact_path  TEXT NOT NULL,
  anchor_text    TEXT,
  anchor_offset  INTEGER,
  note           TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  FOREIGN KEY(task_id) REFERENCES tasks(id)
);

CREATE INDEX IF NOT EXISTS idx_tasks_state ON tasks(state);
CREATE INDEX IF NOT EXISTS idx_tasks_stage ON tasks(current_stage);
CREATE INDEX IF NOT EXISTS idx_workers_team ON workers(team_id);
CREATE INDEX IF NOT EXISTS idx_comments_task ON comments(task_id);
