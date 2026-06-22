-- 001_initial.sql — Plan 1 schema (Workspace + Conversation tables only;
-- other tables land in their respective context's plan).

CREATE TABLE IF NOT EXISTS projects (
  id                  TEXT PRIMARY KEY,
  name                TEXT NOT NULL,
  root_path           TEXT NOT NULL,
  active_pipeline_id  TEXT,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS conversations (
  id                          TEXT PRIMARY KEY,
  project_id                  TEXT NOT NULL,
  started_at                  INTEGER NOT NULL,
  last_message_at             INTEGER NOT NULL,
  history_json                TEXT NOT NULL,
  summary_of_prior_sessions   TEXT,
  FOREIGN KEY(project_id) REFERENCES projects(id)
);

CREATE INDEX IF NOT EXISTS idx_conversations_project ON conversations(project_id);
