use crate::metrics::Metrics;
use crate::router::Router;
use crate::verify::peer_cache::PeerCache;
use crate::verify::peer_source::{SourceResult, source_peers};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

pub struct HealthProberConfig {
    pub interval: Duration,
    pub batch_size: i64,
    pub query_timeout: Duration,
    pub concurrency: usize,
}

impl Default for HealthProberConfig {
    fn default() -> Self {
        HealthProberConfig {
            interval: Duration::from_secs(5),
            batch_size: 60,
            query_timeout: Duration::from_secs(3),
            concurrency: 24,
        }
    }
}

pub struct HealthProber {
    pool: PgPool,
    router: Arc<Router>,
    metrics: Arc<Metrics>,
    cache: Arc<PeerCache>,
    ip_cooldown: Arc<crate::net::ip_cooldown::IpCooldownCache>,
    config: HealthProberConfig,
}

#[derive(Clone)]
struct FetchStats {
    total: i64,
    successful: i64,
    timeouts: i64,
    last_success: Option<chrono::DateTime<chrono::Utc>>,
}

impl HealthProber {
    pub fn new(
        pool: PgPool,
        router: Arc<Router>,
        metrics: Arc<Metrics>,
        cache: Arc<PeerCache>,
        ip_cooldown: Arc<crate::net::ip_cooldown::IpCooldownCache>,
        config: HealthProberConfig,
    ) -> Self {
        HealthProber {
            pool,
            router,
            metrics,
            cache,
            ip_cooldown,
            config,
        }
    }

    pub async fn run(self: Arc<Self>) {
        let mut interval = tokio::time::interval(self.config.interval);
        loop {
            interval.tick().await;
            if let Err(e) = self.probe_round().await {
                tracing::warn!(error = %e, "health prober: round error");
            }
        }
    }

    async fn probe_round(&self) -> Result<(), sqlx::Error> {
        // Tier 1: Query active swarms sighted in the last 7 days first (highest ROI for users)
        let mut rows = sqlx::query_as::<_, (Vec<u8>, i64, chrono::DateTime<chrono::Utc>)>(
            "SELECT infohash, total_seen, last_seen \
             FROM torrents \
             WHERE last_seen > now() - interval '7 days' \
             ORDER BY last_health_check ASC NULLS FIRST \
             LIMIT $1",
        )
        .bind(self.config.batch_size)
        .fetch_all(&self.pool)
        .await?;

        // Tier 2: If all active swarms are recently checked, pick from the oldest unprobed cold torrents
        if rows.is_empty() {
            rows = sqlx::query_as::<_, (Vec<u8>, i64, chrono::DateTime<chrono::Utc>)>(
                "SELECT infohash, total_seen, last_seen \
                 FROM torrents \
                 ORDER BY last_health_check ASC NULLS FIRST \
                 LIMIT $1",
            )
            .bind(self.config.batch_size)
            .fetch_all(&self.pool)
            .await?;
        }

        if rows.is_empty() {
            return Ok(());
        }

        // Aggregate fetch_peer_outcomes for this batch (48h window)
        let infohashes: Vec<Vec<u8>> = rows.iter().map(|(ih, _, _)| ih.clone()).collect();
        let fetch_stats = self.fetch_outcome_stats(&infohashes).await;

        let now = chrono::Utc::now();
        let mut set = JoinSet::new();
        let sem = Arc::new(tokio::sync::Semaphore::new(self.config.concurrency));

        for (ih_bytes, total_seen, last_seen) in rows {
            if ih_bytes.len() != 20 {
                continue;
            }
            let mut ih = [0u8; 20];
            ih.copy_from_slice(&ih_bytes);

            let router = self.router.clone();
            let metrics = self.metrics.clone();
            let cache = self.cache.clone();
            let ip_cooldown = self.ip_cooldown.clone();
            let sem = sem.clone();
            let query_timeout = self.config.query_timeout;
            let stats = fetch_stats.get(&ih_bytes).cloned();

            set.spawn(async move {
                let _permit = sem.acquire_owned().await.ok();
                let res = source_peers(
                    router,
                    ih,
                    24,
                    metrics,
                    query_timeout * 2,
                    16,
                    3,
                    query_timeout,
                    24,
                    &cache,
                    &ip_cooldown,
                    false,
                )
                .await;

                let peers_count = match res {
                    SourceResult::Peers(p) => p.len(),
                    SourceResult::NoPeers => 0,
                    SourceResult::AllTimeout => 0,
                };

                let hours_decay = (now - last_seen).num_seconds().max(0) as f64 / 3600.0;
                let decay = (-hours_decay / 48.0).exp();

                let p_sat = if peers_count > 0 {
                    ((1.0 + peers_count as f64).ln() / (26.0f64).ln()).min(1.0)
                } else {
                    0.0
                };

                let s = if peers_count > 0 { 1.0 } else { 0.0 };

                // Compute fetch reliability from historical outcomes
                let (fetch_reliability, hours_since_success) = match &stats {
                    Some(fs) if fs.total > 0 => {
                        let success_rate = fs.successful as f64 / fs.total as f64;
                        let timeout_rate = fs.timeouts as f64 / fs.total as f64;
                        let hrs = fs.last_success.map(|t| (now - t).num_seconds().max(0) as f64 / 3600.0);
                        let recency = hrs.map(|h| (-h / 24.0).exp()).unwrap_or(0.0);
                        let reliability = success_rate * recency * (1.0 - timeout_rate);
                        (reliability, hrs)
                    }
                    _ => (0.0, None),
                };

                let health_score = if fetch_reliability > 0.0 {
                    // Has fetch outcome data: weight fetch reliability heavily
                    ((100.0 * (0.35 * s + 0.20 * p_sat + 0.45 * fetch_reliability) * decay).round()
                        as i16)
                        .clamp(0, 100)
                } else {
                    // No fetch outcome data: fall back to DHT-only formula
                    ((100.0 * (0.6 * s + 0.4 * p_sat) * decay).round() as i16).clamp(0, 100)
                };

                // Normalization calibrated to network sightings: 500 max baseline provides high dynamic range
                let pop_base =
                    ((total_seen.max(1) as f64 + 1.0).log10() / 501.0f64.log10()).min(1.0);
                let vel = (-hours_decay / 168.0).exp(); // 7-day velocity window
                let pop_score =
                    ((100.0 * (0.40 * pop_base + 0.35 * vel + 0.25 * p_sat)).round() as i16)
                        .clamp(0, 100);

                // Seed confirmed requires: (DHT peers found OR successful fetch) AND recent success within 48h
                let has_recent_success = hours_since_success
                    .map(|h| h <= 48.0)
                    .unwrap_or(false);
                let seed_confirmed =
                    (peers_count > 0 || has_recent_success) && has_recent_success || (peers_count > 0 && hours_decay <= 48.0);

                (ih_bytes, peers_count, health_score, pop_score, seed_confirmed)
            });
        }

        while let Some(res) = set.join_next().await {
            if let Ok((ih_bytes, peers_count, health_score, pop_score, seed_confirmed)) = res {
                let _ = sqlx::query(
                    "UPDATE torrents \
                     SET swarm_peers = $2, health_score = $3, popularity_score = $4, seed_confirmed = $5, last_health_check = now() \
                     WHERE infohash = $1",
                )
                .bind(&ih_bytes)
                .bind(peers_count as i32)
                .bind(health_score)
                .bind(pop_score)
                .bind(seed_confirmed)
                .execute(&self.pool)
                .await;
            }
        }

        Ok(())
    }

    async fn fetch_outcome_stats(
        &self,
        infohashes: &[Vec<u8>],
    ) -> HashMap<Vec<u8>, FetchStats> {
        if infohashes.is_empty() {
            return HashMap::new();
        }
        let rows = match sqlx::query_as::<_, (Vec<u8>, i64, i64, i64, Option<chrono::DateTime<chrono::Utc>>)>(
            "SELECT infohash, \
                    count(*) AS total, \
                    count(*) FILTER (WHERE result IN ('ok','metadata_ok')) AS successful, \
                    count(*) FILTER (WHERE result IN ('timeout','metadata_timeout')) AS timeouts, \
                    max(created_at) FILTER (WHERE result IN ('ok','metadata_ok')) AS last_success \
             FROM fetch_peer_outcomes \
             WHERE infohash = ANY($1::bytea[]) \
               AND created_at > now() - interval '48 hours' \
             GROUP BY infohash",
        )
        .bind(infohashes)
        .fetch_all(&self.pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "health prober: fetch_outcome_stats query failed");
                return HashMap::new();
            }
        };

        rows.into_iter()
            .map(|(ih, total, successful, timeouts, last_success)| {
                (ih, FetchStats { total, successful, timeouts, last_success })
            })
            .collect()
    }
}
