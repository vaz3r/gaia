use crate::metrics::Metrics;
use crate::router::Router;
use crate::verify::peer_cache::PeerCache;
use crate::verify::peer_source::{SourceResult, source_peers};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

pub struct HealthProberConfig {
    pub interval: Duration,
    pub batch_size: i64,
    pub query_timeout: Duration,
}

impl Default for HealthProberConfig {
    fn default() -> Self {
        HealthProberConfig {
            interval: Duration::from_secs(45),
            batch_size: 25,
            query_timeout: Duration::from_secs(3),
        }
    }
}

pub struct HealthProber {
    pool: PgPool,
    router: Arc<Router>,
    metrics: Arc<Metrics>,
    cache: Arc<PeerCache>,
    config: HealthProberConfig,
}

impl HealthProber {
    pub fn new(
        pool: PgPool,
        router: Arc<Router>,
        metrics: Arc<Metrics>,
        cache: Arc<PeerCache>,
        config: HealthProberConfig,
    ) -> Self {
        HealthProber {
            pool,
            router,
            metrics,
            cache,
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
        let rows = sqlx::query_as::<_, (Vec<u8>, i64, chrono::DateTime<chrono::Utc>)>(
            "SELECT infohash, total_seen, last_seen \
             FROM torrents \
             ORDER BY last_health_check ASC NULLS FIRST \
             LIMIT $1",
        )
        .bind(self.config.batch_size)
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now();
        for (ih_bytes, total_seen, last_seen) in rows {
            if ih_bytes.len() != 20 {
                continue;
            }
            let mut ih = [0u8; 20];
            ih.copy_from_slice(&ih_bytes);

            let res = source_peers(
                self.router.clone(),
                ih,
                8,
                self.metrics.clone(),
                self.config.query_timeout * 2,
                8,
                3,
                self.config.query_timeout,
                16,
                &self.cache,
                false,
            )
            .await;

            let peers_count = match res {
                SourceResult::Peers(p) => p.len(),
                SourceResult::NoPeers => 0,
                SourceResult::AllTimeout => 0,
            };

            let hours_decay = (now - last_seen).num_seconds().max(0) as f64 / 3600.0;
            let decay = (-hours_decay / 48.0).exp(); // 48-hour half-life (honest real-time swarm decay)

            let p_sat = if peers_count > 0 {
                ((1.0 + peers_count as f64).ln() / (26.0f64).ln()).min(1.0)
            } else {
                0.0
            };

            let s = if peers_count > 0 { 1.0 } else { 0.0 };
            let health_score = ((100.0 * (0.6 * s + 0.4 * p_sat) * decay).round() as i16)
                .clamp(0, 100);

            let pop_base = ((total_seen.max(1) as f64 + 1.0).log10() / 50001.0f64.log10()).min(1.0);
            let vel = (-hours_decay / 168.0).exp(); // 7-day velocity window
            let pop_score = ((100.0 * (0.40 * pop_base + 0.35 * vel + 0.25 * p_sat)).round() as i16)
                .clamp(0, 100);

            // Confirmed seed requires active peers probed AND sighted within 48h
            let seed_confirmed = peers_count > 0 && hours_decay <= 48.0;

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

        Ok(())
    }
}
