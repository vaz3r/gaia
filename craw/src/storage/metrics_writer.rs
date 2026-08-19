use crate::metrics::{Metrics, Snapshot};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

pub struct MetricsWriter {
    pool: PgPool,
    metrics: Arc<Metrics>,
}

impl MetricsWriter {
    pub fn new(pool: PgPool, metrics: Arc<Metrics>) -> Self {
        MetricsWriter { pool, metrics }
    }

    pub async fn write(&self) -> Result<(), sqlx::Error> {
        let s = self.metrics.snapshot();
        let rows = snapshot_rows(&s);
        let mut tx = self.pool.begin().await?;
        for (name, value) in rows {
            sqlx::query(
                "INSERT INTO metrics (ts, metric_name, metric_value) VALUES (now(), $1, $2) \
                 ON CONFLICT (ts, metric_name) DO UPDATE SET metric_value = EXCLUDED.metric_value",
            )
            .bind(name)
            .bind(value as i64)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn run(self: Arc<Self>, interval: Duration) {
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;
            if let Err(e) = self.write().await {
                tracing::warn!(error = %e, "metrics writer: flush failed");
            }
        }
    }
}

fn snapshot_rows(s: &Snapshot) -> Vec<(&'static str, u64)> {
    vec![
        ("inbound_ping", s.inbound_ping),
        ("inbound_find_node", s.inbound_find_node),
        ("inbound_get_peers", s.inbound_get_peers),
        ("inbound_announce_peer", s.inbound_announce_peer),
        ("inbound_invalid", s.inbound_invalid),
        ("inbound_find_node_bep42", s.inbound_find_node_bep42),
        ("inbound_find_node_random", s.inbound_find_node_random),
        ("inbound_get_peers_bep42", s.inbound_get_peers_bep42),
        ("inbound_get_peers_random", s.inbound_get_peers_random),
        ("inbound_announce_bep42", s.inbound_announce_bep42),
        ("inbound_announce_random", s.inbound_announce_random),
        (
            "inbound_announce_invalid_token",
            s.inbound_announce_invalid_token,
        ),
        ("tokens_issued", s.tokens_issued),
        ("infohashes_harvested", s.infohashes_harvested),
        ("unique_infohashes", s.unique_infohashes),
        ("outbound_queries", s.outbound_queries),
        ("outbound_timeouts", s.outbound_timeouts),
        ("tx_table_len", s.tx_table_len),
        ("routing_table_len", s.routing_table_len),
        ("verify_attempts", s.verify_attempts),
        ("verify_success", s.verify_success),
        ("verify_fail", s.verify_fail),
        ("verify_timeouts", s.verify_timeouts),
        ("fetch_attempts", s.fetch_attempts),
        ("source_queries", s.source_queries),
        ("source_responses", s.source_responses),
        ("source_peers_returned", s.source_peers_returned),
        ("send_dropped", s.send_dropped),
    ]
}
