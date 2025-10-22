-- Add 'error' state to collection_status enum for permanent errors that should not retry
ALTER TYPE collection_status ADD VALUE 'error';
