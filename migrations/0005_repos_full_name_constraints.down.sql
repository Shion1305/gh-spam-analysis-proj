-- Drop case-insensitive uniqueness and format check for repository full names
DROP INDEX IF EXISTS idx_repositories_full_name_ci;
ALTER TABLE repositories
    DROP CONSTRAINT IF EXISTS repositories_full_name_format;

