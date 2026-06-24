-- A4: per-project skill sources. A JSON-encoded TEXT list of extra `.claude`
-- roots beyond the always-scanned global ~/.claude, resolved at the composition
-- root and handed to the skill scanner. Nullable; null = global only.
-- Append-only; 001-010 untouched. (A JSON column over a child table: the list is
-- small and always read/written whole — same pragmatism as the rest of the row.)
ALTER TABLE projects ADD COLUMN skill_sources TEXT;
