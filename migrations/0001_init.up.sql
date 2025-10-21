CREATE TABLE repositories (
    id BIGINT PRIMARY KEY,
    full_name TEXT NOT NULL UNIQUE,
    is_fork BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    pushed_at TIMESTAMPTZ,
    raw JSONB NOT NULL
);

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
    raw JSONB NOT NULL
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
    raw JSONB NOT NULL
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
