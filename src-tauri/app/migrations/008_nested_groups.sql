-- 008_nested_groups.sql — P1 (nested groups / non-linear lanes). Makes the
-- FanOutGroup aggregate RECURSIVE: a fork nested inside a lane spawns a CHILD
-- group whose completion settles its parent lane's verdict via the SAME
-- completes-once guard. Append-only; migrations 001–007 are never edited.
--
-- A root group (top-level fork) has both columns NULL. A child group carries the
-- enclosing group id + the enclosing lane name, so when the child barrier fires
-- the pool knows which parent lane to settle.
ALTER TABLE fanout_groups ADD COLUMN parent_group_id TEXT;  -- enclosing group, or NULL at the root
ALTER TABLE fanout_groups ADD COLUMN parent_lane     TEXT;  -- enclosing lane (entry team id), or NULL at the root

CREATE INDEX IF NOT EXISTS idx_fanout_groups_parent ON fanout_groups(parent_group_id);
