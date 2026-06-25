-- Worktree isolation (WT1): the per-work-item git worktree a task runs in.
-- Nullable, append-only; read-only stages and legacy tasks carry NULL.
ALTER TABLE tasks ADD COLUMN worktree_path TEXT;
