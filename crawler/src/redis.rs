use std::sync::Arc;

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use tracing::warn;

/// Max entries in a Redis dedup set before it is flushed and rebuilt. Dedup is
/// best-effort (the in-process bloom + DB are authoritative), so a flush only
/// re-attempts a handful of hashes once. Bounds Redis memory (~40 B/entry →
/// ~40 MB at 1M entries).
const MAX_SEEN_ENTRIES: usize = 1_000_000;

/// Shared cross-instance coordination via Redis. Used for a fleet-wide
/// seen-set (dedup emitted infohashes across instances) and a fleet-wide
/// dead-peer cache (skip IPs that failed to connect anywhere).
///
/// All operations are best-effort: if Redis is unreachable or a command
/// fails, callers fall back to per-instance in-memory state and the crawler
/// continues normally.
#[derive(Clone)]
pub struct SharedState {
    conn: Option<Arc<ConnectionManager>>,
    /// Redis keyspace prefix (namespace) for this crawler.
    prefix: String,
}

impl SharedState {
    /// Connect to Redis at `url`. Returns a state with `conn = None` if the
    /// URL is absent or the initial connection fails.
    pub async fn connect(url: Option<String>) -> SharedState {
        let Some(url) = url else {
            return SharedState { conn: None, prefix: "dht".into() };
        };
        let client = match redis::Client::open(url.as_str()) {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "invalid redis url; running without shared state");
                return SharedState { conn: None, prefix: "dht".into() };
            }
        };
        let connect_fut = ConnectionManager::new(client.clone());
        match tokio::time::timeout(std::time::Duration::from_secs(5), connect_fut).await {
            Ok(Ok(cm)) => {
                info_connected();
                SharedState { conn: Some(Arc::new(cm)), prefix: "dht".into() }
            }
            Ok(Err(e)) => {
                warn!(error = %e, "could not connect to redis; running without shared state");
                SharedState { conn: None, prefix: "dht".into() }
            }
            Err(_) => {
                warn!("redis connect timed out after 5s; running without shared state");
                SharedState { conn: None, prefix: "dht".into() }
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// True if `hash` has already been emitted by any instance (present in the
    /// shared seen set). Best-effort; returns false on any Redis error so a
    /// hash is not spuriously dropped.
    pub async fn seen_contains(&self, hash: &[u8; 20]) -> bool {
        let Some(conn) = &self.conn else { return false };
        let key = format!("{}:seen", self.prefix);
        let mut c = conn.as_ref().clone();
        let r: redis::RedisResult<bool> = c.sismember(key, hash).await;
        r.unwrap_or(false)
    }

    /// Mark `hash` as emitted by this crawler (add to the shared seen set).
    /// Best-effort. The set is capped: past `MAX_SEEN_ENTRIES` it is flushed
    /// and rebuilt, because dedup is best-effort (the in-process bloom + DB
    /// are the authoritative "already fetched" check) and an unbounded Redis
    /// set grows without limit (5.7M entries -> 236 MB and climbing).
    pub async fn seen_add(&self, hash: &[u8; 20]) {
        let Some(conn) = &self.conn else { return };
        let key = format!("{}:seen", self.prefix);
        let mut c = conn.as_ref().clone();
        if let Err(e) = c.sadd::<_, _, i64>(&key, hash).await {
            warn!(error = %e, "redis seen_add failed");
            return;
        }
        self.maybe_cap_set(&key).await;
    }

    /// Flush a dedup set once it exceeds a cardinality cap. Dedup is
    /// best-effort, so a flush only causes a brief re-attempt of a few hashes
    /// (absorbed by the DB/bloom authoritative checks). Keeps Redis bounded.
    async fn maybe_cap_set(&self, key: &str) {
        let Some(conn) = &self.conn else { return };
        let mut c = conn.as_ref().clone();
        let Ok(size): redis::RedisResult<i64> = c.scard(key).await else { return };
        if size > MAX_SEEN_ENTRIES as i64 {
            warn!(key, size, cap = MAX_SEEN_ENTRIES, "redis dedup set capped, flushing");
            let _: redis::RedisResult<i64> = c.del(key).await;
        }
    }

    /// True if this hash has already been fetched via the announce path.
    /// Uses a SEPARATE set from `seen` so an announce carrying a live peer
    /// hint is never dropped just because the sampler already fetched the
    /// hash blindly (announce fetches convert far higher).
    pub async fn announced_contains(&self, hash: &[u8; 20]) -> bool {
        let Some(conn) = &self.conn else { return false };
        let key = format!("{}:announced", self.prefix);
        let mut c = conn.as_ref().clone();
        let r: redis::RedisResult<bool> = c.sismember(key, hash).await;
        r.unwrap_or(false)
    }

    /// Mark `hash` as fetched via the announce path.
    pub async fn announced_add(&self, hash: &[u8; 20]) {
        let Some(conn) = &self.conn else { return };
        let key = format!("{}:announced", self.prefix);
        let mut c = conn.as_ref().clone();
        if let Err(e) = c.sadd::<_, _, i64>(&key, hash).await {
            warn!(error = %e, "redis announced_add failed");
            return;
        }
        self.maybe_cap_set(&key).await;
    }

    /// True if this hash was already emitted via the get_peers (looked-up)
    /// path. Separate set so each passive source dedups independently.
    pub async fn looked_up_contains(&self, hash: &[u8; 20]) -> bool {
        let Some(conn) = &self.conn else { return false };
        let key = format!("{}:lookedup", self.prefix);
        let mut c = conn.as_ref().clone();
        let r: redis::RedisResult<bool> = c.sismember(key, hash).await;
        r.unwrap_or(false)
    }

    /// Mark `hash` as emitted via the get_peers path.
    pub async fn looked_up_add(&self, hash: &[u8; 20]) {
        let Some(conn) = &self.conn else { return };
        let key = format!("{}:lookedup", self.prefix);
        let mut c = conn.as_ref().clone();
        if let Err(e) = c.sadd::<_, _, i64>(&key, hash).await {
            warn!(error = %e, "redis looked_up_add failed");
            return;
        }
        self.maybe_cap_set(&key).await;
    }

    /// Whether `ip` is currently flagged dead fleet-wide. Best-effort; returns
    /// false on error so a peer is not spuriously skipped.
    pub async fn dead_contains(&self, ip: std::net::IpAddr) -> bool {
        let Some(conn) = &self.conn else { return false };
        let key = format!("{}:dead", self.prefix);
        let mut c = conn.as_ref().clone();
        let r: redis::RedisResult<bool> = c.sismember(key, ip.to_string()).await;
        r.unwrap_or(false)
    }

    /// Flag `ip` as dead fleet-wide with a TTL. Best-effort.
    pub async fn dead_add(&self, ip: std::net::IpAddr, ttl_secs: i64) {
        let Some(conn) = &self.conn else { return };
        let key = format!("{}:dead", self.prefix);
        let mut c = conn.as_ref().clone();
        let member = ip.to_string();
        if let Err(e) = c.sadd::<_, _, i64>(key.clone(), &member).await {
            warn!(error = %e, "redis dead_add failed");
            return;
        }
        if let Err(e) = c.expire::<_, i64>(key, ttl_secs).await {
            warn!(error = %e, "redis dead_add expire failed");
        }
    }

}

fn info_connected() {
    tracing::info!("shared state connected to redis");
}

/// Build a `SharedState` from a CLI option, logging whether shared state is
/// active.
pub async fn init_shared(redis_url: Option<String>) -> SharedState {
    let s = SharedState::connect(redis_url).await;
    if s.is_connected() {
        tracing::info!("shared redis dedup/cache enabled");
    } else {
        tracing::info!("no redis — using per-instance in-memory dedup/cache");
    }
    s
}
