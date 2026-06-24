-- A5: project-level target repo. Binds ${target_repo} for all teams' scope
-- resolution and defaults the inject target. Nullable; tilde-expanded at create
-- (Workspace), same discipline as root_path. Append-only; 001-009 untouched.
ALTER TABLE projects ADD COLUMN target_repo TEXT;
