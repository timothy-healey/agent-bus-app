-- 002_pipeline_activation.sql — Plan 2.
-- projects.active_pipeline_id already exists (migration 001). This migration
-- adds an index to make "find projects on pipeline X" cheap and marks the
-- Plan 2 boundary. No destructive changes; 001 is untouched.

CREATE INDEX IF NOT EXISTS idx_projects_active_pipeline
  ON projects(active_pipeline_id);
