-- Enforce case-insensitive uniqueness and basic format for repository full names
-- 1) Ensure there is exactly one slash and no empty owner/name parts
ALTER TABLE repositories
    ADD CONSTRAINT repositories_full_name_format
    CHECK (full_name ~ '^[^/]+/[^/]+$');

-- 2) Add a case-insensitive unique index on full_name
CREATE UNIQUE INDEX IF NOT EXISTS idx_repositories_full_name_ci
    ON repositories (LOWER(full_name));

