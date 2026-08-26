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
        // Granular failure counters
        ("source_timeout", s.source_timeout),
        ("source_no_peers", s.source_no_peers),
        ("source_all_timeout", s.source_all_timeout),
        ("source_deadline_hits", s.source_deadline_hits),
        ("source_deadline_peers", s.source_deadline_peers),
        ("fetch_connect_timeout", s.fetch_connect_timeout),
        ("fetch_connect_io", s.fetch_connect_io),
        ("fetch_handshake", s.fetch_handshake),
        ("fetch_no_extension", s.fetch_no_extension),
        ("fetch_reject", s.fetch_reject),
        ("fetch_bad_piece", s.fetch_bad_piece),
        ("fetch_io", s.fetch_io),
        ("sha1_mismatch", s.sha1_mismatch),
        // Peer cache metrics
        ("peer_cache_size", s.peer_cache_size),
        ("peer_cache_hits", s.peer_cache_hits),
        ("peer_cache_evictions", s.peer_cache_evictions),
        // Source peer quality metrics
        ("source_returned_peers", s.source_returned_peers),
        ("source_filtered_by_cache", s.source_filtered_by_cache),
        // Transport metrics
        ("tcp_attempts", s.tcp_attempts),
        ("utp_attempts", s.utp_attempts),
        ("tcp_connect_ok", s.tcp_connect_ok),
        ("utp_connect_ok", s.utp_connect_ok),
        ("tcp_metadata_ok", s.tcp_metadata_ok),
        ("utp_metadata_ok", s.utp_metadata_ok),
        ("tcp_connect_actual", s.tcp_connect_actual),
        ("utp_connect_actual", s.utp_connect_actual),
        ("connect_ok_no_metadata", s.connect_ok_no_metadata),
        ("metadata_failed_io", s.metadata_failed_io),
        ("metadata_failed_silent", s.metadata_failed_silent),
        ("metadata_timeout", s.metadata_timeout),
        // Channel backpressure instrumentation
        ("harvest_try_send_dropped", s.harvest_try_send_dropped),
        ("harvest_sighting_tx_dropped", s.harvest_sighting_tx_dropped),
        ("scheduler_send_blocked", s.scheduler_send_blocked),
        ("scheduler_claims", s.scheduler_claims),
        ("scheduler_claimed_fresh", s.scheduler_claimed_fresh),
        ("scheduler_claimed_retry", s.scheduler_claimed_retry),
        ("verify_channel_depth", s.verify_channel_depth),
        ("verify_channel_depth_max", s.verify_channel_depth_max),
        ("fresh_channel_dropped", s.fresh_channel_dropped),
        ("fresh_channel_depth", s.fresh_channel_depth),
        ("fresh_channel_depth_max", s.fresh_channel_depth_max),
        ("scheduler_skipped_backpressure", s.scheduler_skipped_backpressure),
    ]
}
