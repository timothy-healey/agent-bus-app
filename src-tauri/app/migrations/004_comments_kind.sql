-- Widen comments to distinguish inline anchored comments from the overall
-- direction note bundled with a revise. Existing rows default to 'inline'.
ALTER TABLE comments ADD COLUMN kind TEXT NOT NULL DEFAULT 'inline';
CREATE INDEX IF NOT EXISTS idx_comments_task_kind ON comments(task_id, kind);
