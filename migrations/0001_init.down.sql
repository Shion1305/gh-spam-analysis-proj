-- Drop collection jobs and enum
DROP INDEX IF EXISTS idx_collection_jobs_status_priority;
DROP INDEX IF EXISTS idx_collection_jobs_full_name;
DROP INDEX IF EXISTS idx_collection_jobs_priority;
DROP INDEX IF EXISTS idx_collection_jobs_status;
DROP TABLE IF EXISTS collection_jobs;
DROP TYPE IF EXISTS collection_status;

-- Drop supporting metrics tables
DROP TABLE IF EXISTS collector_watermarks;
DROP INDEX IF EXISTS idx_spam_flags_subject;
DROP INDEX IF EXISTS idx_spam_flags_subject_version;
DROP TABLE IF EXISTS spam_flags;

-- Drop comments
DROP INDEX IF EXISTS idx_comments_body_gin;
DROP INDEX IF EXISTS idx_comments_dedupe_hash;
DROP INDEX IF EXISTS idx_comments_issue_id;
DROP TABLE IF EXISTS comments;

-- Drop issues
DROP INDEX IF EXISTS idx_issues_body_gin;
DROP INDEX IF EXISTS idx_issues_updated_at;
DROP INDEX IF EXISTS idx_issues_dedupe_hash;
DROP INDEX IF EXISTS idx_issues_repo_id;
DROP TABLE IF EXISTS issues;

-- Drop users
DROP INDEX IF EXISTS idx_users_login;
DROP TABLE IF EXISTS users;

-- Drop repositories
DROP INDEX IF EXISTS idx_repositories_full_name_ci;
DROP INDEX IF EXISTS idx_repositories_full_name;
ALTER TABLE IF EXISTS repositories DROP CONSTRAINT IF EXISTS repositories_full_name_format;
DROP TABLE IF EXISTS repositories;
