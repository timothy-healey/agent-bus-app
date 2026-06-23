-- S1: Git author identity used for commits workers make in worktrees.
-- Single-row config (id=1), mirrors usage_config. v1.1 persists; the
-- worktree-commit path that consumes it lands later.
CREATE TABLE IF NOT EXISTS git_config (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    author_name  TEXT NOT NULL DEFAULT '',
    author_email TEXT NOT NULL DEFAULT ''
);
INSERT OR IGNORE INTO git_config (id, author_name, author_email) VALUES (1, '', '');
