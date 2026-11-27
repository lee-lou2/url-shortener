use std::{
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering::Relaxed},
        Arc,
    },
    time::{Duration, Instant},
};

use moka::{future::Cache, Expiry};
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use redis::{Client, ConnectionInfo, RedisResult};
use sqlx::PgPool;
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::{
    error::{AppError, AppResult},
    links::{self, LinkRecord},
    redirect::CachedLink,
};

const FOUND_TTL: Duration = Duration::from_secs(60);
const MISSING_TTL: Duration = Duration::from_secs(15);
const REDIS_TIMEOUT: Duration = Duration::from_millis(150);
const REDIS_DEADLINE: Duration = Duration::from_millis(200);
const OPEN_MILLIS: u64 = 10_000;

#[derive(Clone)]
pub(crate) struct LinkCache {
    local: Cache<i64, Arc<CacheEntry>>,
    redis: Option<Arc<RedisCache>>,
}

enum CacheEntry {
    Found(Arc<CachedLink>, Instant),
    Missing,
}

struct EntryExpiry;

impl Expiry<i64, Arc<CacheEntry>> for EntryExpiry {
    fn expire_after_create(
        &self,
        _: &i64,
        entry: &Arc<CacheEntry>,
        created: Instant,
    ) -> Option<Duration> {
        Some(match entry.as_ref() {
            CacheEntry::Found(_, expires) => expires.saturating_duration_since(created),
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
    pub(crate) async fn new(capacity: u64, info: Option<ConnectionInfo>) -> RedisResult<Self> {
        let redis = if let Some(info) = info {
            let redis = Arc::new(RedisCache {
                client: Client::open(info)?,
                connection: RwLock::new(None),
                failures: AtomicU32::new(0),
                reopen_at: AtomicU64::new(0),
                started: Instant::now(),
                probe: Mutex::new(()),
                writes: Arc::new(Semaphore::new(32)),
            });
            if !redis.ping().await {
                redis.open();
            }
            Some(redis)
        } else {
            None
        };

        let local = Cache::builder()
            .max_capacity(capacity)
            .weigher(|_: &i64, entry: &Arc<CacheEntry>| {
                256 + match entry.as_ref() {
                    CacheEntry::Found(link, _) => link.weight(),
                    CacheEntry::Missing => 0,
                }
            })
            .expire_after(EntryExpiry)
            .build();
        Ok(Self { local, redis })
    }

    pub(crate) async fn get(
        &self,
        db: &PgPool,
        id: i64,
    ) -> Result<Option<Arc<CachedLink>>, Arc<AppError>> {
        let entry = self.local.try_get_with(id, self.load(db, id)).await?;
        Ok(match entry.as_ref() {
            CacheEntry::Found(link, _) => Some(link.clone()),
            CacheEntry::Missing => None,
        })
    }

    async fn load(&self, db: &PgPool, id: i64) -> AppResult<Arc<CacheEntry>> {
        if let Some(redis) = &self.redis {
            if let Some((record, expires)) = redis.get(id).await {
                if let Some(link) = CachedLink::compile(&record)? {
                    return Ok(Arc::new(CacheEntry::Found(Arc::new(link), expires)));
                }
            }
        }
        let observed_at = Instant::now();
        let Some(record) = links::find_active(db, id).await? else {
            return Ok(Arc::new(CacheEntry::Missing));
        };
        let Some(link) = CachedLink::compile(&record)? else {
            return Ok(Arc::new(CacheEntry::Missing));
        };
        let expires = observed_at + FOUND_TTL;
        if let Some(redis) = &self.redis {
            redis.write(record, expires);
        }
        Ok(Arc::new(CacheEntry::Found(Arc::new(link), expires)))
    }

    pub(crate) async fn warm(&self, record: LinkRecord) -> AppResult<()> {
        if let Some(link) = CachedLink::compile(&record)? {
            let entry = Arc::new(CacheEntry::Found(
                Arc::new(link),
                Instant::now() + FOUND_TTL,
            ));
            self.local.insert(record.id, entry).await;
        }
        Ok(())
    }

    pub(crate) async fn status(&self) -> &'static str {
        match &self.redis {
            None => "disabled",
            Some(redis) if redis.ping().await => "connected",
            Some(_) => "disconnected",
        }
    }
}

struct RedisCache {
    client: Client,
    connection: RwLock<Option<ConnectionManager>>,
    failures: AtomicU32,
    reopen_at: AtomicU64,
    started: Instant,
    probe: Mutex<()>,
    writes: Arc<Semaphore>,
}

impl RedisCache {
    fn elapsed_millis(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX - OPEN_MILLIS)
    }

    fn open(&self) {
        self.reopen_at
            .store(self.elapsed_millis() + OPEN_MILLIS, Relaxed);
    }

    async fn connection(&self) -> RedisResult<ConnectionManager> {
        if let Some(connection) = self.connection.read().await.clone() {
            return Ok(connection);
        }
        let mut slot = self.connection.write().await;
        if let Some(connection) = slot.as_ref() {
            return Ok(connection.clone());
        }
        let config = ConnectionManagerConfig::new()
            .set_connection_timeout(REDIS_TIMEOUT)
            .set_response_timeout(REDIS_TIMEOUT)
            .set_number_of_retries(1)
            .set_factor(25);
        let connection = self
            .client
            .get_connection_manager_with_config(config)
            .await?;
        *slot = Some(connection.clone());
        Ok(connection)
    }

    async fn run<T>(
        &self,
        query: impl AsyncFnOnce(&mut ConnectionManager) -> RedisResult<T>,
    ) -> Option<T> {
        let reopen_at = self.reopen_at.load(Relaxed);
        let _probe = if reopen_at == 0 {
            None
        } else {
            if self.elapsed_millis() < reopen_at {
                return None;
            }
            let probe = self.probe.try_lock().ok()?;
            if self.reopen_at.load(Relaxed) != reopen_at {
                return None;
            }
            Some(probe)
        };
        let result = tokio::time::timeout(REDIS_DEADLINE, async {
            query(&mut self.connection().await?).await
        })
        .await;

        if let Ok(Ok(value)) = result {
            if self
                .reopen_at
                .compare_exchange(reopen_at, 0, Relaxed, Relaxed)
                .is_ok()
            {
                self.failures.store(0, Relaxed);
            }
            return Some(value);
        }

        if reopen_at != 0 || self.failures.fetch_add(1, Relaxed) >= 2 {
            self.open();
        }
        None
    }

    async fn ping(&self) -> bool {
        self.run(async |conn| redis::cmd("PING").query_async::<String>(conn).await)
            .await
            .as_deref()
            == Some("PONG")
    }

    async fn get(&self, id: i64) -> Option<(LinkRecord, Instant)> {
        let key = format!("u:{id}");
        let started = Instant::now();
        let mut commands = redis::pipe();
        commands.atomic().get(&key).cmd("PTTL").arg(&key);
        let (value, millis): (Option<Vec<u8>>, i64) = self
            .run(async |conn| commands.query_async(conn).await)
            .await?;
        let ttl = Duration::from_millis(u64::try_from(millis).ok()?).min(FOUND_TTL);
        let expires = started + ttl;
        let record: LinkRecord = serde_json::from_slice(&value?).ok()?;
        (record.id == id && expires > Instant::now()).then_some((record, expires))
    }

    fn write(self: &Arc<Self>, record: LinkRecord, expires: Instant) {
        let Ok(permit) = self.writes.clone().try_acquire_owned() else {
            return;
        };
        let redis = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let Ok(payload) = serde_json::to_vec(&record) else {
                return;
            };
            // Subtract time already spent loading/rendering before writing.
            let seconds = expires.saturating_duration_since(Instant::now()).as_secs();
            if seconds != 0 {
                redis
                    .run(async |conn| {
                        redis::cmd("SET")
                            .arg(format!("u:{}", record.id))
                            .arg(payload)
                            .arg("EX")
                            .arg(seconds)
                            .exec_async(conn)
                            .await
                    })
                    .await;
            }
        });
    }
}
