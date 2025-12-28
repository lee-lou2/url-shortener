-- Add UNIQUE constraint on hashed_value to prevent race condition duplicates
-- and optimize indexes with partial indexes for better query performance

-- Drop existing indexes that will be replaced with better versions
DROP INDEX IF EXISTS idx_urls_hashed_value;
DROP INDEX IF EXISTS idx_urls_id_deleted;

-- Add UNIQUE constraint on hashed_value for active (non-deleted) URLs
-- This prevents duplicate URLs at the database level
CREATE UNIQUE INDEX IF NOT EXISTS idx_urls_hashed_value_unique
    ON urls(hashed_value) WHERE deleted_at IS NULL;

-- Partial index for ID lookup (only non-deleted URLs)
-- Optimizes the common query pattern: WHERE id = $1 AND deleted_at IS NULL
CREATE INDEX IF NOT EXISTS idx_urls_id_active
    ON urls(id) WHERE deleted_at IS NULL;

-- Partial index for is_active check (only non-deleted URLs)
DROP INDEX IF EXISTS idx_urls_is_active;
CREATE INDEX IF NOT EXISTS idx_urls_is_active_partial
    ON urls(is_active) WHERE deleted_at IS NULL AND is_active = true;

