//! In-process `TinyLFU` cache (`moka`) keyed by decoded `i64` link ID.
//!
//! Keying by `id` (instead of raw `short_key`) ensures wrong-`random_key` guesses
//! hit RAM in nanoseconds without querying PostgreSQL. `try_get_with` coalesces
//! concurrent cold misses (singleflight), negative entries bound 404 storms, and
//! a byte weigher with fixed per-entry overhead caps RSS across replicas.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use moka::{future::Cache, Expiry};
use sqlx::PgPool;

use crate::{
    error::{AppError, AppResult},
    links::{self, LinkRecord},
    redirect::CachedLink,
};

/// Active links expire after 60 s so manual takedowns take effect promptly.
const FOUND_TTL: Duration = Duration::from_secs(60);
/// Missing IDs are cached briefly to absorb random-ID scan bursts.
const MISSING_TTL: Duration = Duration::from_secs(15);
/// Fixed weight added per entry to account for moka/allocator metadata overhead.
const ENTRY_OVERHEAD_BYTES: u32 = 256;

#[derive(Clone)]
pub(crate) struct LinkCache {
    local: Cache<i64, Arc<CacheEntry>>,
}

enum CacheEntry {
    Found(Arc<CachedLink>),
    Missing,
}

struct EntryExpiry;

impl Expiry<i64, Arc<CacheEntry>> for EntryExpiry {
    fn expire_after_create(
        &self,
        _: &i64,
        entry: &Arc<CacheEntry>,
        _: Instant,
    ) -> Option<Duration> {
        Some(match entry.as_ref() {
            CacheEntry::Found(_) => FOUND_TTL,
            CacheEntry::Missing => MISSING_TTL,
        })
    }

    fn expire_after_update(
        &self,
        key: &i64,
        entry: &Arc<CacheEntry>,
        updated: Instant,
        _: Option<Duration>,
    ) -> Option<Duration> {
        self.expire_after_create(key, entry, updated)
    }
}

impl LinkCache {
    pub(crate) fn new(max_capacity_bytes: u64) -> Self {
        let local = Cache::builder()
            .max_capacity(max_capacity_bytes)
            .weigher(|_: &i64, entry: &Arc<CacheEntry>| {
                ENTRY_OVERHEAD_BYTES
                    + match entry.as_ref() {
                        CacheEntry::Found(link) => link.weight(),
                        CacheEntry::Missing => 0,
                    }
            })
            .expire_after(EntryExpiry)
            .build();
        Self { local }
    }

    /// Returns the compiled link for `id`, loading from PostgreSQL at most once
    /// across concurrent callers (singleflight). Database errors are never cached.
    pub(crate) async fn get(
        &self,
        db: &PgPool,
        id: i64,
    ) -> Result<Option<Arc<CachedLink>>, Arc<AppError>> {
        let entry = self.local.try_get_with(id, load_from_db(db, id)).await?;
        Ok(match entry.as_ref() {
            CacheEntry::Found(link) => Some(link.clone()),
            CacheEntry::Missing => None,
        })
    }

    /// Pre-populates L1 immediately on link creation so the first click hits RAM.
    pub(crate) async fn warm(&self, record: LinkRecord) -> AppResult<()> {
        if let Some(link) = CachedLink::compile(&record)? {
            self.local
                .insert(record.id, Arc::new(CacheEntry::Found(Arc::new(link))))
                .await;
        }
        Ok(())
    }
}

async fn load_from_db(db: &PgPool, id: i64) -> AppResult<Arc<CacheEntry>> {
    let Some(record) = links::find_active(db, id).await? else {
        return Ok(Arc::new(CacheEntry::Missing));
    };
    let Some(link) = CachedLink::compile(&record)? else {
        return Ok(Arc::new(CacheEntry::Missing));
    };
    Ok(Arc::new(CacheEntry::Found(Arc::new(link))))
}
