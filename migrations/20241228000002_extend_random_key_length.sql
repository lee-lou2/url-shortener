-- Extend random_key column from VARCHAR(2) to VARCHAR(4)
-- This migration supports the new short key format:
-- prefix (2 chars) + Base62 ID + suffix (2 chars)

ALTER TABLE urls
ALTER COLUMN random_key TYPE VARCHAR(4);

