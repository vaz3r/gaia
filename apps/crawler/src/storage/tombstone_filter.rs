use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::time;

/// In-memory set of infohashes that have been tombstoned (blocked) by the
/// category policy system.  The set is populated at startup from the
/// `blocked_infohashes` table and refreshed every `refresh_interval` seconds
/// by fetching only rows whose `blocked_at` timestamp is newer than the last
/// successful sync.
///
/// All reads on the hot `harvest()` path are lock-free in the common case
/// (RwLock with many concurrent readers and rare writers).
pub struct TombstoneFilter {
    set: Arc<RwLock<HashSet<[u8; 20]>>>,
    pool: PgPool,
    refresh_interval: Duration,
}

impl TombstoneFilter {
    /// Create a new filter.  Call [`TombstoneFilter::load`] or
    /// [`TombstoneFilter::run`] to populate it.
    pub fn new(pool: PgPool, refresh_interval: Duration) -> Arc<Self> {
        Arc::new(TombstoneFilter {
            set: Arc::new(RwLock::new(HashSet::new())),
            pool,
            refresh_interval,
        })
    }

    /// Returns `true` if `ih` is in the blocked set.
    /// This is a read lock and is safe to call on the hot harvest path.
    #[inline]
    pub fn is_blocked(&self, ih: &[u8; 20]) -> bool {
        self.set
            .read()
            .expect("tombstone_filter rwlock poisoned")
            .contains(ih)
    }

    /// Perform the initial full load, then spin up the periodic refresh task.
    /// This method never returns; spawn it as a detached task.
    pub async fn run(self: Arc<Self>) {
        // Full load on startup.
        self.full_load().await;

        let mut last_synced = std::time::SystemTime::now();
        let mut interval = time::interval(self.refresh_interval);
        interval.tick().await; // consume the immediate first tick

        loop {
            interval.tick().await;
            self.incremental_sync(&mut last_synced).await;
        }
    }

    /// Load every row from `blocked_infohashes` into the set.
    async fn full_load(&self) {
        match sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT infohash FROM blocked_infohashes",
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => {
                let mut set = self.set.write().expect("tombstone_filter rwlock poisoned");
                let before = set.len();
                for raw in &rows {
                    if raw.len() == 20 {
                        let mut ih = [0u8; 20];
                        ih.copy_from_slice(raw);
                        set.insert(ih);
                    }
                }
                tracing::info!(
                    loaded = set.len() - before,
                    total = set.len(),
                    "tombstone_filter: full load complete"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "tombstone_filter: full_load failed");
            }
        }
    }

    /// Fetch only rows newer than `last_synced`, add them to the set, and
    /// advance the cursor on success.
    async fn incremental_sync(&self, last_synced: &mut std::time::SystemTime) {
        // Convert SystemTime to a UTC timestamp string sqlx can bind.
        let ts = chrono::DateTime::<chrono::Utc>::from(*last_synced);

        match sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT infohash FROM blocked_infohashes WHERE blocked_at > $1",
        )
        .bind(ts)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) if !rows.is_empty() => {
                let count = rows.len();
                let mut set = self.set.write().expect("tombstone_filter rwlock poisoned");
                for raw in &rows {
                    if raw.len() == 20 {
                        let mut ih = [0u8; 20];
                        ih.copy_from_slice(raw);
                        set.insert(ih);
                    }
                }
                tracing::debug!(
                    new_tombstones = count,
                    total = set.len(),
                    "tombstone_filter: incremental sync"
                );
                *last_synced = std::time::SystemTime::now();
            }
            Ok(_) => {
                // Nothing new — still advance cursor so the next window is fresh.
                *last_synced = std::time::SystemTime::now();
            }
            Err(e) => {
                tracing::warn!(error = %e, "tombstone_filter: incremental_sync failed");
                // Do NOT advance last_synced — retry the same window next tick.
            }
        }
    }
}
