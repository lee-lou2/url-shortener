-- Create urls table for URL shortener service
-- This migration creates the main table for storing shortened URL information

CREATE TABLE IF NOT EXISTS urls (
    id BIGSERIAL PRIMARY KEY,
    -- 2-character random key used in the shortened URL
    random_key VARCHAR(2) NOT NULL,
    -- iOS deep link URL
    ios_deep_link TEXT,
    -- Fallback URL when iOS deep link fails
    ios_fallback_url TEXT,
    -- Android deep link URL
    android_deep_link TEXT,
    -- Fallback URL when Android deep link fails
    android_fallback_url TEXT,
    -- Default redirection URL (required)
    default_fallback_url TEXT NOT NULL,
    -- Hash value to prevent URL duplication
    hashed_value TEXT NOT NULL,
    -- Webhook URL to call when URL is accessed
    webhook_url TEXT,
    -- Open Graph title
    og_title VARCHAR(255),
    -- Open Graph description
    og_description TEXT,
    -- Open Graph image URL
    og_image_url TEXT,
    -- URL activation status
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Soft delete timestamp
    deleted_at TIMESTAMPTZ
);

-- Index for fast hash lookup (duplicate prevention)
CREATE INDEX IF NOT EXISTS idx_urls_hashed_value ON urls(hashed_value);

-- Index for filtering active URLs
CREATE INDEX IF NOT EXISTS idx_urls_is_active ON urls(is_active);

-- Index for soft delete queries
CREATE INDEX IF NOT EXISTS idx_urls_deleted_at ON urls(deleted_at);

-- Composite index for common query pattern
CREATE INDEX IF NOT EXISTS idx_urls_id_deleted ON urls(id, deleted_at);

