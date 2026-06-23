-- 006_fanout.sql — Sub-project 2 (parallel flow: fork/join). Adds the Task lane
-- columns and the fan-out group barrier tables. Append-only; migrations 001–005
-- are never edited. The fanout_groups.completed flag is the single-row guard for
-- the *completes-exactly-once* barrier invariant (FanOutGroup aggregate).

-- Lane membership on a task. NULL for ordinary linear tasks; set on the sibling
-- tasks a fork spawns.
ALTER TABLE tasks ADD COLUMN group_id    TEXT;   -- the fan-out group this task belongs to
ALTER TABLE tasks ADD COLUMN lane        TEXT;   -- which lane (entry team id)
ALTER TABLE tasks ADD COLUMN join_target TEXT;   -- the join node this lane reports to

-- The fan-out group: the consistency boundary for the barrier. One row per fork
-- expansion. `completed` is the conditional-UPDATE guard.
CREATE TABLE IF NOT EXISTS fanout_groups (
  id           TEXT PRIMARY KEY,
  pipeline     TEXT NOT NULL,
  join_target  TEXT NOT NULL,          -- the join id all lanes report to
  downstream   TEXT NOT NULL,          -- where an all-approve continuation goes
  completed    INTEGER NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL DEFAULT 0
);

-- One row per lane in a group, carrying that lane's settled verdict. Unique on
-- (group_id, lane) so re-recording a lane (crash recovery) is idempotent.
CREATE TABLE IF NOT EXISTS fanout_lanes (
  group_id  TEXT NOT NULL,
  lane      TEXT NOT NULL,
  verdict   TEXT NOT NULL,             -- 'approve' | 'reject' | 'pending'
  PRIMARY KEY (group_id, lane),
  FOREIGN KEY (group_id) REFERENCES fanout_groups(id)
);

CREATE INDEX IF NOT EXISTS idx_tasks_group ON tasks(group_id);
CREATE INDEX IF NOT EXISTS idx_fanout_lanes_group ON fanout_lanes(group_id);
