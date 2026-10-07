-- 018_invocation_effort.sql — the effort level each invocation was started
-- with, taken from the argv the app built. NULL = Default (no --effort flag).
-- The stream never reports the effort used, so this is its only record.
ALTER TABLE invocation_audit ADD COLUMN effort TEXT;
