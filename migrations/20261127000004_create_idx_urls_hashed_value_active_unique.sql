-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS idx_urls_hashed_value_active_unique
ON urls (hashed_value) WHERE deleted_at IS NULL AND is_active = true;
