CREATE TYPE collection_status AS ENUM ('pending', 'in_progress', 'completed', 'failed');

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
