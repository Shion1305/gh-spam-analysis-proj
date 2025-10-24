-- Core repositories table
CREATE TABLE repositories (
    id BIGINT PRIMARY KEY,
    full_name TEXT NOT NULL,
    is_fork BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    pushed_at TIMESTAMPTZ,
    raw JSONB NOT NULL
);

-- Basic format: exactly one slash, non-empty owner and name
ALTER TABLE repositories
    ADD CONSTRAINT repositories_full_name_format
    CHECK (full_name ~ '^[^/]+/[^/]+$');

-- Case-insensitive uniqueness on full_name
CREATE UNIQUE INDEX idx_repositories_full_name_ci
    ON repositories (LOWER(full_name));

-- Helpful index for equality lookups by full_name
CREATE INDEX idx_repositories_full_name ON repositories (full_name);

CREATE TABLE users (
    id BIGINT PRIMARY KEY,
    login TEXT NOT NULL UNIQUE,
    type TEXT NOT NULL,
    site_admin BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ,
    followers BIGINT,
    following BIGINT,
    public_repos BIGINT,
    raw JSONB NOT NULL,
    found BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE INDEX idx_users_login ON users (login);

CREATE TABLE issues (
    id BIGINT PRIMARY KEY,
    repo_id BIGINT NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    number BIGINT NOT NULL,
    is_pull_request BOOLEAN NOT NULL,
    state TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT,
    user_id BIGINT REFERENCES users (id),
    comments_count BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ,
    dedupe_hash TEXT NOT NULL,
    raw JSONB NOT NULL,
    found BOOLEAN NOT NULL DEFAULT TRUE,
    UNIQUE (repo_id, number)
);

CREATE INDEX idx_issues_repo_id ON issues (repo_id);
CREATE INDEX idx_issues_dedupe_hash ON issues (dedupe_hash);
CREATE INDEX idx_issues_updated_at ON issues (updated_at DESC);
CREATE INDEX idx_issues_body_gin ON issues USING GIN (to_tsvector('english', coalesce(body, '')));

CREATE TABLE comments (
    id BIGINT PRIMARY KEY,
    issue_id BIGINT NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
    user_id BIGINT REFERENCES users (id),
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ,
    dedupe_hash TEXT NOT NULL,
    raw JSONB NOT NULL,
    found BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE INDEX idx_comments_issue_id ON comments (issue_id);
CREATE INDEX idx_comments_dedupe_hash ON comments (dedupe_hash);
CREATE INDEX idx_comments_body_gin ON comments USING GIN (to_tsvector('english', body));

CREATE TABLE spam_flags (
    id BIGSERIAL PRIMARY KEY,
    subject_type TEXT NOT NULL CHECK (subject_type IN ('issue', 'comment')),
    subject_id BIGINT NOT NULL,
    score REAL NOT NULL,
    reasons TEXT[] NOT NULL,
    version TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_spam_flags_subject_version ON spam_flags (subject_type, subject_id, version);
CREATE INDEX idx_spam_flags_subject ON spam_flags (subject_type, subject_id);

CREATE TABLE collector_watermarks (
    repo_full_name TEXT PRIMARY KEY,
    last_updated TIMESTAMPTZ NOT NULL
);

-- Collection jobs (seeding queue for the collector)
-- Include permanent 'error' state in the enum.
CREATE TYPE collection_status AS ENUM ('pending', 'in_progress', 'completed', 'failed', 'error');

CREATE TABLE collection_jobs (
    id BIGSERIAL PRIMARY KEY,
    owner TEXT NOT NULL,
    name TEXT NOT NULL,
    full_name TEXT NOT NULL GENERATED ALWAYS AS (owner || '/' || name) STORED,
    status collection_status NOT NULL DEFAULT 'pending',
    priority INT NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    last_completed_at TIMESTAMPTZ,
    failure_count INT NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner, name)
);

CREATE INDEX idx_collection_jobs_status ON collection_jobs (status);
CREATE INDEX idx_collection_jobs_priority ON collection_jobs (priority DESC, created_at ASC);
CREATE INDEX idx_collection_jobs_full_name ON collection_jobs (full_name);
CREATE INDEX idx_collection_jobs_status_priority ON collection_jobs (status, priority DESC, created_at ASC);
