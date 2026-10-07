-- 019_invocation_denials.sql — the actions each invocation was refused, by a
-- permission rule or by the auto-mode classifier, as a JSON array of
-- {tool_name, tool_input, source}. NULL for rows written before this column.
ALTER TABLE invocation_audit ADD COLUMN permission_denials TEXT;
