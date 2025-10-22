-- Cannot remove enum value in PostgreSQL, so we need to recreate the enum
-- First update all 'error' statuses to 'failed'
UPDATE collection_jobs SET status = 'failed' WHERE status = 'error';

-- Recreate the enum without 'error'
ALTER TYPE collection_status RENAME TO collection_status_old;
CREATE TYPE collection_status AS ENUM ('pending', 'in_progress', 'completed', 'failed');
ALTER TABLE collection_jobs ALTER COLUMN status TYPE collection_status USING status::text::collection_status;
DROP TYPE collection_status_old;
