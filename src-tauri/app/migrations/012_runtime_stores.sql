-- 012_runtime_stores.sql — Runtime redesign ④a (bounded-buffer foundation).
-- Append-only; migrations 001–011 are never edited. Additive only: these tables
-- and columns back the new Store / Run / generator-ledger aggregates and the
-- work-item identity; nothing here is wired into the existing single-task pool yet
-- (that is ④b). The single conditional-UPDATE guards live in the aggregates:
--   stores.occupancy < capacity  (Store::reserve — occupancy <= capacity invariant)
--   runs.completed = 0            (Run::try_complete — completes-exactly-once)

-- A Run: one execution of a pipeline (the tree of work-items from a single Start).
-- Aggregate root; owns the completes-exactly-once invariant via `completed`.
CREATE TABLE IF NOT EXISTS runs (
  id             TEXT PRIMARY KEY,
  pipeline       TEXT NOT NULL,
  project_id     TEXT NOT NULL,
  generator_dry  INTEGER NOT NULL DEFAULT 0,  -- the source generator found nothing new
  completed      INTEGER NOT NULL DEFAULT 0,  -- the completes-once conditional-UPDATE guard
  created_at     INTEGER NOT NULL DEFAULT 0
);

-- A Store: a capacity-limited buffer holding work-items waiting for a stage.
-- Aggregate root keyed by (run_id, stage); owns the invariant occupancy<=capacity,
-- enforced by the atomic `occupancy < capacity` guard on reserve (no count-then-act).
CREATE TABLE IF NOT EXISTS stores (
  run_id     TEXT NOT NULL,
  stage      TEXT NOT NULL,
  capacity   INTEGER NOT NULL,
  occupancy  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (run_id, stage)
);

-- The generator (source-stage) found-key ledger: the per-run set of candidate
-- keys already produced. Append-only; the dedup + dry-detection source of truth.
-- INSERT OR IGNORE against the PK makes re-recording a key a no-op (dedup).
CREATE TABLE IF NOT EXISTS generator_ledger (
  run_id        TEXT NOT NULL,
  stage         TEXT NOT NULL,
  candidate_key TEXT NOT NULL,
  PRIMARY KEY (run_id, stage, candidate_key)
);

-- Work-item identity on a task (the Task repurposed as the flowing work-item).
-- Nullable + unused by the existing single-task pool; populated by ④b+.
ALTER TABLE tasks ADD COLUMN run_id   TEXT;  -- the Run this work-item belongs to
ALTER TABLE tasks ADD COLUMN item_key TEXT;  -- the stable candidate key (lineage/dedup)

CREATE INDEX IF NOT EXISTS idx_tasks_run ON tasks(run_id);
