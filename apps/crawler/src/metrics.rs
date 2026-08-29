use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub inbound_ping: AtomicU64,
    pub inbound_find_node: AtomicU64,
    pub inbound_find_node_dropped: AtomicU64,
    pub inbound_get_peers: AtomicU64,
    pub inbound_announce_peer: AtomicU64,
    pub inbound_invalid: AtomicU64,
    pub inbound_find_node_bep42: AtomicU64,
    pub inbound_find_node_random: AtomicU64,
    pub inbound_get_peers_bep42: AtomicU64,
    pub inbound_get_peers_random: AtomicU64,
    pub inbound_announce_bep42: AtomicU64,
    pub inbound_announce_random: AtomicU64,
    pub inbound_announce_invalid_token: AtomicU64,
    pub inbound_announce_valid: AtomicU64,
    pub tokens_issued: AtomicU64,
    pub infohashes_harvested: AtomicU64,
    pub unique_infohashes: AtomicU64,
    pub outbound_queries: AtomicU64,
    pub outbound_timeouts: AtomicU64,
    pub tx_table_len: AtomicU64,
    pub routing_table_len: AtomicU64,
    pub verify_attempts: AtomicU64,
    pub verify_success: AtomicU64,
    pub verify_fail: AtomicU64,
    pub verify_timeouts: AtomicU64,
    pub fetch_attempts: AtomicU64,
    pub source_queries: AtomicU64,
    pub source_responses: AtomicU64,
    pub source_peers_returned: AtomicU64,
    pub send_dropped: AtomicU64,
    // Granular failure counters
    pub source_timeout: AtomicU64,
    pub source_no_peers: AtomicU64,
    pub source_all_timeout: AtomicU64,
    pub fetch_connect_timeout: AtomicU64,
    pub fetch_connect_io: AtomicU64,
    pub fetch_handshake: AtomicU64,
    pub fetch_no_extension: AtomicU64,
    pub fetch_reject: AtomicU64,
    pub fetch_bad_piece: AtomicU64,
    pub fetch_io: AtomicU64,
    pub sha1_mismatch: AtomicU64,
    // Peer cache metrics
    pub peer_cache_size: AtomicU64,
    pub peer_cache_hits: AtomicU64,
    pub peer_cache_evictions: AtomicU64,
    // Source peer quality metrics
    pub source_returned_peers: AtomicU64,
    pub source_filtered_by_cache: AtomicU64,
    pub source_no_values: AtomicU64,
    // Pipelined source lookup termination
    pub source_deadline_hits: AtomicU64,
    pub source_deadline_peers: AtomicU64,
    // Announce direct-fetch metrics
    pub announce_attempts: AtomicU64,
    pub announce_success: AtomicU64,
    // Transport metrics
    pub tcp_attempts: AtomicU64,
    pub utp_attempts: AtomicU64,
    pub tcp_connect_ok: AtomicU64,
    pub utp_connect_ok: AtomicU64,
    pub tcp_metadata_ok: AtomicU64,
    pub utp_metadata_ok: AtomicU64,
    // Per-transport actual connect success (regardless of race winner)
    pub tcp_connect_actual: AtomicU64,
    pub utp_connect_actual: AtomicU64,
    // Connect-vs-metadata accounting
    pub connect_ok_no_metadata: AtomicU64,
    pub metadata_failed_io: AtomicU64,
    pub metadata_failed_silent: AtomicU64,
    pub metadata_timeout: AtomicU64,
    // Walker metrics
    pub walker_steps: AtomicU64,
    pub walker_queries: AtomicU64,
    pub walker_ok: AtomicU64,
    pub walker_nodes_returned: AtomicU64,
    pub walker_self_target: AtomicU64,
    pub walker_random_target: AtomicU64,
    pub walker_sybil_target: AtomicU64,
    // Routing insert metrics
    pub routing_insert_calls: AtomicU64,
    pub routing_nodes_added: AtomicU64,
    pub routing_buckets_used: AtomicU64,
    pub routing_new_ids: AtomicU64,
    pub routing_rejected: AtomicU64,
    // Logging metrics
    pub log_dropped: Arc<AtomicU64>,
    // Channel backpressure instrumentation (Phase 1: observe, no behavior change)
    pub harvest_try_send_dropped: AtomicU64,
    pub harvest_sighting_tx_dropped: AtomicU64,
    pub scheduler_send_blocked: AtomicU64,
    pub scheduler_claims: AtomicU64,
    pub scheduler_claimed_fresh: AtomicU64,
    pub scheduler_claimed_retry: AtomicU64,
    pub verify_channel_depth: AtomicU64,
    pub verify_channel_depth_max: AtomicU64,
    pub fresh_channel_dropped: AtomicU64,
    pub fresh_channel_depth: AtomicU64,
    pub fresh_channel_depth_max: AtomicU64,
    pub scheduler_skipped_backpressure: AtomicU64,
    // Pipeline observability — bounded atomics, no alloc on hot path
    pub fresh_dequeued_total: AtomicU64,
    pub retry_dequeued_total: AtomicU64,
    pub announce_dequeued_total: AtomicU64,
    pub pipeline_dequeued_total: AtomicU64,
    pub pipeline_spawned_total: AtomicU64,
    pub pipeline_completed_total: AtomicU64,
    pub pipeline_cancelled_total: AtomicU64,
    pub pipeline_active: AtomicU64,
    pub pipeline_active_max_interval: AtomicU64,
    pub pipeline_permit_wait_micros_total: AtomicU64,
    pub pipeline_permit_acquisitions_total: AtomicU64,
    pub pipeline_task_micros_total: AtomicU64,
    /// Final pipeline outcome `NoPeers` (terminal `VerifyResult::NoPeers` after
    /// DHT lookup + direct-peer race). Distinct from per-query `source_timeout`
    /// / `source_no_values`: it is the pipeline task's terminal result, not
    /// just the source sub-phase. Kept separate from `source_no_peers` (which
    /// is the historical pipeline terminal counter) to make `pipeline_*`
    /// conservation checks self-contained.
    pub pipeline_no_peers_total: AtomicU64,
    // Verification phase durations — wall-clock per task vs aggregate per attempt
    pub verify_source_micros_total: AtomicU64,
    pub verify_source_completed_total: AtomicU64,
    pub fetch_permit_wait_micros_total: AtomicU64,
    pub fetch_permit_acquisitions_total: AtomicU64,
    pub per_ip_wait_micros_total: AtomicU64,
    pub per_ip_acquisitions_total: AtomicU64,
    pub transport_connect_micros_total: AtomicU64,
    pub transport_connect_completed_total: AtomicU64,
    pub metadata_exchange_micros_total: AtomicU64,
    pub metadata_exchange_completed_total: AtomicU64,
    pub fetch_joinset_drain_micros_total: AtomicU64,
    pub fetch_joinset_drain_completed_total: AtomicU64,
    pub fetch_permit_owned_attempts_total: AtomicU64,
    pub fetch_candidates_skipped_budget_total: AtomicU64,
    // Per-candidate-source metrics
    pub source_direct_accepted_total: AtomicU64,
    pub source_direct_attempts_total: AtomicU64,
    pub source_direct_connect_ok_total: AtomicU64,
    pub source_direct_connect_timeout_total: AtomicU64,
    pub source_direct_connect_io_total: AtomicU64,
    pub source_direct_metadata_ok_total: AtomicU64,
    pub source_direct_metadata_fail_total: AtomicU64,
    pub source_direct_verified_total: AtomicU64,

    pub source_announce_cache_accepted_total: AtomicU64,
    pub source_announce_cache_attempts_total: AtomicU64,
    pub source_announce_cache_connect_ok_total: AtomicU64,
    pub source_announce_cache_connect_timeout_total: AtomicU64,
    pub source_announce_cache_connect_io_total: AtomicU64,
    pub source_announce_cache_metadata_ok_total: AtomicU64,
    pub source_announce_cache_metadata_fail_total: AtomicU64,
    pub source_announce_cache_verified_total: AtomicU64,

    pub source_dht_accepted_total: AtomicU64,
    pub source_dht_attempts_total: AtomicU64,
    pub source_dht_connect_ok_total: AtomicU64,
    pub source_dht_connect_timeout_total: AtomicU64,
    pub source_dht_connect_io_total: AtomicU64,
    pub source_dht_metadata_ok_total: AtomicU64,
    pub source_dht_metadata_fail_total: AtomicU64,
    pub source_dht_verified_total: AtomicU64,

    // Lead task & query attribution counters
    pub lead_tasks_total: AtomicU64,
    pub lead_tasks_dht_started_total: AtomicU64,
    pub lead_tasks_queries_total: AtomicU64,
    pub lead_tasks_lead_verified_total: AtomicU64,
    pub lead_tasks_dht_verified_total: AtomicU64,
    pub non_lead_tasks_total: AtomicU64,
    pub non_lead_tasks_queries_total: AtomicU64,

    // Hybrid lead grace metrics
    pub lead_dht_deferred_total: AtomicU64,
    pub lead_dht_started_grace_expired_total: AtomicU64,
    pub lead_dht_started_exhausted_total: AtomicU64,
    /// Count of lead-bearing tasks where metadata was retrieved from a lead before
    /// DHT sourcing was spawned, so DHT spawning was avoided. Final SHA-1 validation
    /// happens afterward in the caller, therefore this counter does not itself guarantee
    /// a verified torrent.
    pub lead_dht_avoided_total: AtomicU64,
    pub lead_grace_micros_total: AtomicU64,
    pub lead_grace_completed_total: AtomicU64,

    // Lead attempt latency buckets (elapsed from attempt start to completion)
    pub lead_success_le_250ms_total: AtomicU64,
    pub lead_success_le_500ms_total: AtomicU64,
    pub lead_success_le_1000ms_total: AtomicU64,
    pub lead_success_le_2000ms_total: AtomicU64,
    pub lead_success_gt_2000ms_total: AtomicU64,

    pub lead_failure_le_250ms_total: AtomicU64,
    pub lead_failure_le_500ms_total: AtomicU64,
    pub lead_failure_le_1000ms_total: AtomicU64,
    pub lead_failure_le_2000ms_total: AtomicU64,
    pub lead_failure_gt_2000ms_total: AtomicU64,

    pub result_handling_micros_total: AtomicU64,
    pub result_handling_completed_total: AtomicU64,
    pub source_active: AtomicU64,
    pub fetch_active: AtomicU64,
    pub metadata_active: AtomicU64,
    // Linux recvmmsg receive metrics
    pub udp_recv_syscalls_total: AtomicU64,
    pub udp_recv_successful_syscalls_total: AtomicU64,
    pub udp_recv_packets_total: AtomicU64,
    pub udp_recv_batch_max_interval: AtomicU64,
    pub udp_recv_eagain_total: AtomicU64,
    pub udp_recv_eintr_total: AtomicU64,
    pub udp_recv_errors_total: AtomicU64,
    pub udp_recv_truncated_total: AtomicU64,
    pub udp_recv_invalid_addr_total: AtomicU64,
    pub udp_recv_zero_length_total: AtomicU64,
    pub udp_recv_fatal_total: AtomicU64,
}

impl Metrics {
    pub fn new(log_dropped: Arc<AtomicU64>) -> Self {
        Metrics {
            log_dropped,
            ..Metrics::default()
        }
    }
}

pub trait Add1 {
    fn add(&self, n: u64);
}

impl Add1 for AtomicU64 {
    fn add(&self, n: u64) {
        self.fetch_add(n, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Snapshot {
    pub inbound_ping: u64,
    pub inbound_find_node: u64,
    pub inbound_find_node_dropped: u64,
    pub inbound_get_peers: u64,
    pub inbound_announce_peer: u64,
    pub inbound_invalid: u64,
    pub inbound_find_node_bep42: u64,
    pub inbound_find_node_random: u64,
    pub inbound_get_peers_bep42: u64,
    pub inbound_get_peers_random: u64,
    pub inbound_announce_bep42: u64,
    pub inbound_announce_random: u64,
    pub inbound_announce_invalid_token: u64,
    pub inbound_announce_valid: u64,
    pub tokens_issued: u64,
    pub infohashes_harvested: u64,
    pub unique_infohashes: u64,
    pub outbound_queries: u64,
    pub outbound_timeouts: u64,
    pub tx_table_len: u64,
    pub routing_table_len: u64,
    pub verify_attempts: u64,
    pub verify_success: u64,
    pub verify_fail: u64,
    pub verify_timeouts: u64,
    pub fetch_attempts: u64,
    pub source_queries: u64,
    pub source_responses: u64,
    pub source_peers_returned: u64,
    pub send_dropped: u64,
    pub source_timeout: u64,
    pub source_no_peers: u64,
    pub source_all_timeout: u64,
    pub fetch_connect_timeout: u64,
    pub fetch_connect_io: u64,
    pub fetch_handshake: u64,
    pub fetch_no_extension: u64,
    pub fetch_reject: u64,
    pub fetch_bad_piece: u64,
    pub fetch_io: u64,
    pub sha1_mismatch: u64,
    pub peer_cache_size: u64,
    pub peer_cache_hits: u64,
    pub peer_cache_evictions: u64,
    pub source_returned_peers: u64,
    pub source_filtered_by_cache: u64,
    pub source_no_values: u64,
    pub source_deadline_hits: u64,
    pub source_deadline_peers: u64,
    pub announce_attempts: u64,
    pub announce_success: u64,
    pub tcp_attempts: u64,
    pub utp_attempts: u64,
    pub tcp_connect_ok: u64,
    pub utp_connect_ok: u64,
    pub tcp_metadata_ok: u64,
    pub utp_metadata_ok: u64,
    pub tcp_connect_actual: u64,
    pub utp_connect_actual: u64,
    pub connect_ok_no_metadata: u64,
    pub metadata_failed_io: u64,
    pub metadata_failed_silent: u64,
    pub metadata_timeout: u64,
    pub walker_steps: u64,
    pub walker_queries: u64,
    pub walker_ok: u64,
    pub walker_nodes_returned: u64,
    pub walker_self_target: u64,
    pub walker_random_target: u64,
    pub walker_sybil_target: u64,
    pub routing_insert_calls: u64,
    pub routing_nodes_added: u64,
    pub routing_buckets_used: u64,
    pub routing_new_ids: u64,
    pub routing_rejected: u64,
    pub log_dropped: u64,
    pub harvest_try_send_dropped: u64,
    pub harvest_sighting_tx_dropped: u64,
    pub scheduler_send_blocked: u64,
    pub scheduler_claims: u64,
    pub scheduler_claimed_fresh: u64,
    pub scheduler_claimed_retry: u64,
    pub verify_channel_depth: u64,
    pub verify_channel_depth_max: u64,
    pub fresh_channel_dropped: u64,
    pub fresh_channel_depth: u64,
    pub fresh_channel_depth_max: u64,
    pub scheduler_skipped_backpressure: u64,
    pub fresh_dequeued_total: u64,
    pub retry_dequeued_total: u64,
    pub announce_dequeued_total: u64,
    pub pipeline_dequeued_total: u64,
    pub pipeline_spawned_total: u64,
    pub pipeline_completed_total: u64,
    pub pipeline_cancelled_total: u64,
    pub pipeline_active: u64,
    pub pipeline_active_max_interval: u64,
    pub pipeline_permit_wait_micros_total: u64,
    pub pipeline_permit_acquisitions_total: u64,
    pub pipeline_task_micros_total: u64,
    pub pipeline_no_peers_total: u64,
    pub verify_source_micros_total: u64,
    pub verify_source_completed_total: u64,
    pub fetch_permit_wait_micros_total: u64,
    pub fetch_permit_acquisitions_total: u64,
    pub per_ip_wait_micros_total: u64,
    pub per_ip_acquisitions_total: u64,
    pub transport_connect_micros_total: u64,
    pub transport_connect_completed_total: u64,
    pub metadata_exchange_micros_total: u64,
    pub metadata_exchange_completed_total: u64,
    pub fetch_joinset_drain_micros_total: u64,
    pub fetch_joinset_drain_completed_total: u64,
    pub fetch_permit_owned_attempts_total: u64,
    pub fetch_candidates_skipped_budget_total: u64,
    pub source_direct_accepted_total: u64,
    pub source_direct_attempts_total: u64,
    pub source_direct_connect_ok_total: u64,
    pub source_direct_connect_timeout_total: u64,
    pub source_direct_connect_io_total: u64,
    pub source_direct_metadata_ok_total: u64,
    pub source_direct_metadata_fail_total: u64,
    pub source_direct_verified_total: u64,
    pub source_announce_cache_accepted_total: u64,
    pub source_announce_cache_attempts_total: u64,
    pub source_announce_cache_connect_ok_total: u64,
    pub source_announce_cache_connect_timeout_total: u64,
    pub source_announce_cache_connect_io_total: u64,
    pub source_announce_cache_metadata_ok_total: u64,
    pub source_announce_cache_metadata_fail_total: u64,
    pub source_announce_cache_verified_total: u64,
    pub source_dht_accepted_total: u64,
    pub source_dht_attempts_total: u64,
    pub source_dht_connect_ok_total: u64,
    pub source_dht_connect_timeout_total: u64,
    pub source_dht_connect_io_total: u64,
    pub source_dht_metadata_ok_total: u64,
    pub source_dht_metadata_fail_total: u64,
    pub source_dht_verified_total: u64,

    pub lead_tasks_total: u64,
    pub lead_tasks_dht_started_total: u64,
    pub lead_tasks_queries_total: u64,
    pub lead_tasks_lead_verified_total: u64,
    pub lead_tasks_dht_verified_total: u64,
    pub non_lead_tasks_total: u64,
    pub non_lead_tasks_queries_total: u64,

    pub lead_dht_deferred_total: u64,
    pub lead_dht_started_grace_expired_total: u64,
    pub lead_dht_started_exhausted_total: u64,
    pub lead_dht_avoided_total: u64,
    pub lead_grace_micros_total: u64,
    pub lead_grace_completed_total: u64,

    pub lead_success_le_250ms_total: u64,
    pub lead_success_le_500ms_total: u64,
    pub lead_success_le_1000ms_total: u64,
    pub lead_success_le_2000ms_total: u64,
    pub lead_success_gt_2000ms_total: u64,

    pub lead_failure_le_250ms_total: u64,
    pub lead_failure_le_500ms_total: u64,
    pub lead_failure_le_1000ms_total: u64,
    pub lead_failure_le_2000ms_total: u64,
    pub lead_failure_gt_2000ms_total: u64,

    pub result_handling_micros_total: u64,
    pub result_handling_completed_total: u64,
    pub source_active: u64,
    pub fetch_active: u64,
    pub metadata_active: u64,
    // Linux recvmmsg receive metrics
    pub udp_recv_syscalls_total: u64,
    pub udp_recv_successful_syscalls_total: u64,
    pub udp_recv_packets_total: u64,
    pub udp_recv_batch_max_interval: u64,
    pub udp_recv_eagain_total: u64,
    pub udp_recv_eintr_total: u64,
    pub udp_recv_errors_total: u64,
    pub udp_recv_truncated_total: u64,
    pub udp_recv_invalid_addr_total: u64,
    pub udp_recv_zero_length_total: u64,
    pub udp_recv_fatal_total: u64,
}

impl Metrics {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            inbound_ping: self.inbound_ping.load(Ordering::Relaxed),
            inbound_find_node: self.inbound_find_node.load(Ordering::Relaxed),
            inbound_find_node_dropped: self.inbound_find_node_dropped.load(Ordering::Relaxed),
            inbound_get_peers: self.inbound_get_peers.load(Ordering::Relaxed),
            inbound_announce_peer: self.inbound_announce_peer.load(Ordering::Relaxed),
            inbound_invalid: self.inbound_invalid.load(Ordering::Relaxed),
            inbound_find_node_bep42: self.inbound_find_node_bep42.load(Ordering::Relaxed),
            inbound_find_node_random: self.inbound_find_node_random.load(Ordering::Relaxed),
            inbound_get_peers_bep42: self.inbound_get_peers_bep42.load(Ordering::Relaxed),
            inbound_get_peers_random: self.inbound_get_peers_random.load(Ordering::Relaxed),
            inbound_announce_bep42: self.inbound_announce_bep42.load(Ordering::Relaxed),
            inbound_announce_random: self.inbound_announce_random.load(Ordering::Relaxed),
            inbound_announce_invalid_token: self
                .inbound_announce_invalid_token
                .load(Ordering::Relaxed),
            inbound_announce_valid: self.inbound_announce_valid.load(Ordering::Relaxed),
            tokens_issued: self.tokens_issued.load(Ordering::Relaxed),
            infohashes_harvested: self.infohashes_harvested.load(Ordering::Relaxed),
            unique_infohashes: self.unique_infohashes.load(Ordering::Relaxed),
            outbound_queries: self.outbound_queries.load(Ordering::Relaxed),
            outbound_timeouts: self.outbound_timeouts.load(Ordering::Relaxed),
            tx_table_len: self.tx_table_len.load(Ordering::Relaxed),
            routing_table_len: self.routing_table_len.load(Ordering::Relaxed),
            verify_attempts: self.verify_attempts.load(Ordering::Relaxed),
            verify_success: self.verify_success.load(Ordering::Relaxed),
            verify_fail: self.verify_fail.load(Ordering::Relaxed),
            verify_timeouts: self.verify_timeouts.load(Ordering::Relaxed),
            fetch_attempts: self.fetch_attempts.load(Ordering::Relaxed),
            source_queries: self.source_queries.load(Ordering::Relaxed),
            source_responses: self.source_responses.load(Ordering::Relaxed),
            source_peers_returned: self.source_peers_returned.load(Ordering::Relaxed),
            send_dropped: self.send_dropped.load(Ordering::Relaxed),
            source_timeout: self.source_timeout.load(Ordering::Relaxed),
            source_no_peers: self.source_no_peers.load(Ordering::Relaxed),
            source_all_timeout: self.source_all_timeout.load(Ordering::Relaxed),
            fetch_connect_timeout: self.fetch_connect_timeout.load(Ordering::Relaxed),
            fetch_connect_io: self.fetch_connect_io.load(Ordering::Relaxed),
            fetch_handshake: self.fetch_handshake.load(Ordering::Relaxed),
            fetch_no_extension: self.fetch_no_extension.load(Ordering::Relaxed),
            fetch_reject: self.fetch_reject.load(Ordering::Relaxed),
            fetch_bad_piece: self.fetch_bad_piece.load(Ordering::Relaxed),
            fetch_io: self.fetch_io.load(Ordering::Relaxed),
            sha1_mismatch: self.sha1_mismatch.load(Ordering::Relaxed),
            peer_cache_size: self.peer_cache_size.load(Ordering::Relaxed),
            peer_cache_hits: self.peer_cache_hits.load(Ordering::Relaxed),
            peer_cache_evictions: self.peer_cache_evictions.load(Ordering::Relaxed),
            source_returned_peers: self.source_returned_peers.load(Ordering::Relaxed),
            source_filtered_by_cache: self.source_filtered_by_cache.load(Ordering::Relaxed),
            source_no_values: self.source_no_values.load(Ordering::Relaxed),
            source_deadline_hits: self.source_deadline_hits.load(Ordering::Relaxed),
            source_deadline_peers: self.source_deadline_peers.load(Ordering::Relaxed),
            announce_attempts: self.announce_attempts.load(Ordering::Relaxed),
            announce_success: self.announce_success.load(Ordering::Relaxed),
            tcp_attempts: self.tcp_attempts.load(Ordering::Relaxed),
            utp_attempts: self.utp_attempts.load(Ordering::Relaxed),
            tcp_connect_ok: self.tcp_connect_ok.load(Ordering::Relaxed),
            utp_connect_ok: self.utp_connect_ok.load(Ordering::Relaxed),
            tcp_metadata_ok: self.tcp_metadata_ok.load(Ordering::Relaxed),
            utp_metadata_ok: self.utp_metadata_ok.load(Ordering::Relaxed),
            tcp_connect_actual: self.tcp_connect_actual.load(Ordering::Relaxed),
            utp_connect_actual: self.utp_connect_actual.load(Ordering::Relaxed),
            connect_ok_no_metadata: self.connect_ok_no_metadata.load(Ordering::Relaxed),
            metadata_failed_io: self.metadata_failed_io.load(Ordering::Relaxed),
            metadata_failed_silent: self.metadata_failed_silent.load(Ordering::Relaxed),
            metadata_timeout: self.metadata_timeout.load(Ordering::Relaxed),
            walker_steps: self.walker_steps.load(Ordering::Relaxed),
            walker_queries: self.walker_queries.load(Ordering::Relaxed),
            walker_ok: self.walker_ok.load(Ordering::Relaxed),
            walker_nodes_returned: self.walker_nodes_returned.load(Ordering::Relaxed),
            walker_self_target: self.walker_self_target.load(Ordering::Relaxed),
            walker_random_target: self.walker_random_target.load(Ordering::Relaxed),
            walker_sybil_target: self.walker_sybil_target.load(Ordering::Relaxed),
            routing_insert_calls: self.routing_insert_calls.load(Ordering::Relaxed),
            routing_nodes_added: self.routing_nodes_added.load(Ordering::Relaxed),
            routing_buckets_used: self.routing_buckets_used.load(Ordering::Relaxed),
            routing_new_ids: self.routing_new_ids.load(Ordering::Relaxed),
            routing_rejected: self.routing_rejected.load(Ordering::Relaxed),
            log_dropped: self.log_dropped.load(Ordering::Relaxed),
            harvest_try_send_dropped: self.harvest_try_send_dropped.load(Ordering::Relaxed),
            harvest_sighting_tx_dropped: self.harvest_sighting_tx_dropped.load(Ordering::Relaxed),
            scheduler_send_blocked: self.scheduler_send_blocked.load(Ordering::Relaxed),
            scheduler_claims: self.scheduler_claims.load(Ordering::Relaxed),
            scheduler_claimed_fresh: self.scheduler_claimed_fresh.load(Ordering::Relaxed),
            scheduler_claimed_retry: self.scheduler_claimed_retry.load(Ordering::Relaxed),
            verify_channel_depth: self.verify_channel_depth.load(Ordering::Relaxed),
            verify_channel_depth_max: self.verify_channel_depth_max.load(Ordering::Relaxed),
            fresh_channel_dropped: self.fresh_channel_dropped.load(Ordering::Relaxed),
            fresh_channel_depth: self.fresh_channel_depth.load(Ordering::Relaxed),
            fresh_channel_depth_max: self.fresh_channel_depth_max.load(Ordering::Relaxed),
            scheduler_skipped_backpressure: self
                .scheduler_skipped_backpressure
                .load(Ordering::Relaxed),
            fresh_dequeued_total: self.fresh_dequeued_total.load(Ordering::Relaxed),
            retry_dequeued_total: self.retry_dequeued_total.load(Ordering::Relaxed),
            announce_dequeued_total: self.announce_dequeued_total.load(Ordering::Relaxed),
            pipeline_dequeued_total: self.pipeline_dequeued_total.load(Ordering::Relaxed),
            pipeline_spawned_total: self.pipeline_spawned_total.load(Ordering::Relaxed),
            pipeline_completed_total: self.pipeline_completed_total.load(Ordering::Relaxed),
            pipeline_cancelled_total: self.pipeline_cancelled_total.load(Ordering::Relaxed),
            pipeline_active: self.pipeline_active.load(Ordering::Relaxed),
            pipeline_active_max_interval: self.pipeline_active_max_interval.load(Ordering::Relaxed),
            pipeline_permit_wait_micros_total: self
                .pipeline_permit_wait_micros_total
                .load(Ordering::Relaxed),
            pipeline_permit_acquisitions_total: self
                .pipeline_permit_acquisitions_total
                .load(Ordering::Relaxed),
            pipeline_task_micros_total: self.pipeline_task_micros_total.load(Ordering::Relaxed),
            pipeline_no_peers_total: self.pipeline_no_peers_total.load(Ordering::Relaxed),
            verify_source_micros_total: self.verify_source_micros_total.load(Ordering::Relaxed),
            verify_source_completed_total: self
                .verify_source_completed_total
                .load(Ordering::Relaxed),
            fetch_permit_wait_micros_total: self
                .fetch_permit_wait_micros_total
                .load(Ordering::Relaxed),
            fetch_permit_acquisitions_total: self
                .fetch_permit_acquisitions_total
                .load(Ordering::Relaxed),
            per_ip_wait_micros_total: self.per_ip_wait_micros_total.load(Ordering::Relaxed),
            per_ip_acquisitions_total: self.per_ip_acquisitions_total.load(Ordering::Relaxed),
            transport_connect_micros_total: self
                .transport_connect_micros_total
                .load(Ordering::Relaxed),
            transport_connect_completed_total: self
                .transport_connect_completed_total
                .load(Ordering::Relaxed),
            metadata_exchange_micros_total: self
                .metadata_exchange_micros_total
                .load(Ordering::Relaxed),
            metadata_exchange_completed_total: self
                .metadata_exchange_completed_total
                .load(Ordering::Relaxed),
            fetch_joinset_drain_micros_total: self
                .fetch_joinset_drain_micros_total
                .load(Ordering::Relaxed),
            fetch_joinset_drain_completed_total: self
                .fetch_joinset_drain_completed_total
                .load(Ordering::Relaxed),
            fetch_permit_owned_attempts_total: self
                .fetch_permit_owned_attempts_total
                .load(Ordering::Relaxed),
            fetch_candidates_skipped_budget_total: self
                .fetch_candidates_skipped_budget_total
                .load(Ordering::Relaxed),
            source_direct_accepted_total: self.source_direct_accepted_total.load(Ordering::Relaxed),
            source_direct_attempts_total: self.source_direct_attempts_total.load(Ordering::Relaxed),
            source_direct_connect_ok_total: self
                .source_direct_connect_ok_total
                .load(Ordering::Relaxed),
            source_direct_connect_timeout_total: self
                .source_direct_connect_timeout_total
                .load(Ordering::Relaxed),
            source_direct_connect_io_total: self
                .source_direct_connect_io_total
                .load(Ordering::Relaxed),
            source_direct_metadata_ok_total: self
                .source_direct_metadata_ok_total
                .load(Ordering::Relaxed),
            source_direct_metadata_fail_total: self
                .source_direct_metadata_fail_total
                .load(Ordering::Relaxed),
            source_direct_verified_total: self.source_direct_verified_total.load(Ordering::Relaxed),
            source_announce_cache_accepted_total: self
                .source_announce_cache_accepted_total
                .load(Ordering::Relaxed),
            source_announce_cache_attempts_total: self
                .source_announce_cache_attempts_total
                .load(Ordering::Relaxed),
            source_announce_cache_connect_ok_total: self
                .source_announce_cache_connect_ok_total
                .load(Ordering::Relaxed),
            source_announce_cache_connect_timeout_total: self
                .source_announce_cache_connect_timeout_total
                .load(Ordering::Relaxed),
            source_announce_cache_connect_io_total: self
                .source_announce_cache_connect_io_total
                .load(Ordering::Relaxed),
            source_announce_cache_metadata_ok_total: self
                .source_announce_cache_metadata_ok_total
                .load(Ordering::Relaxed),
            source_announce_cache_metadata_fail_total: self
                .source_announce_cache_metadata_fail_total
                .load(Ordering::Relaxed),
            source_announce_cache_verified_total: self
                .source_announce_cache_verified_total
                .load(Ordering::Relaxed),
            source_dht_accepted_total: self.source_dht_accepted_total.load(Ordering::Relaxed),
            source_dht_attempts_total: self.source_dht_attempts_total.load(Ordering::Relaxed),
            source_dht_connect_ok_total: self.source_dht_connect_ok_total.load(Ordering::Relaxed),
            source_dht_connect_timeout_total: self
                .source_dht_connect_timeout_total
                .load(Ordering::Relaxed),
            source_dht_connect_io_total: self.source_dht_connect_io_total.load(Ordering::Relaxed),
            source_dht_metadata_ok_total: self.source_dht_metadata_ok_total.load(Ordering::Relaxed),
            source_dht_metadata_fail_total: self
                .source_dht_metadata_fail_total
                .load(Ordering::Relaxed),
            source_dht_verified_total: self.source_dht_verified_total.load(Ordering::Relaxed),
            lead_tasks_total: self.lead_tasks_total.load(Ordering::Relaxed),
            lead_tasks_dht_started_total: self.lead_tasks_dht_started_total.load(Ordering::Relaxed),
            lead_tasks_queries_total: self.lead_tasks_queries_total.load(Ordering::Relaxed),
            lead_tasks_lead_verified_total: self
                .lead_tasks_lead_verified_total
                .load(Ordering::Relaxed),
            lead_tasks_dht_verified_total: self
                .lead_tasks_dht_verified_total
                .load(Ordering::Relaxed),
            non_lead_tasks_total: self.non_lead_tasks_total.load(Ordering::Relaxed),
            non_lead_tasks_queries_total: self.non_lead_tasks_queries_total.load(Ordering::Relaxed),
            lead_dht_deferred_total: self.lead_dht_deferred_total.load(Ordering::Relaxed),
            lead_dht_started_grace_expired_total: self
                .lead_dht_started_grace_expired_total
                .load(Ordering::Relaxed),
            lead_dht_started_exhausted_total: self
                .lead_dht_started_exhausted_total
                .load(Ordering::Relaxed),
            lead_dht_avoided_total: self.lead_dht_avoided_total.load(Ordering::Relaxed),
            lead_grace_micros_total: self.lead_grace_micros_total.load(Ordering::Relaxed),
            lead_grace_completed_total: self.lead_grace_completed_total.load(Ordering::Relaxed),
            lead_success_le_250ms_total: self.lead_success_le_250ms_total.load(Ordering::Relaxed),
            lead_success_le_500ms_total: self.lead_success_le_500ms_total.load(Ordering::Relaxed),
            lead_success_le_1000ms_total: self.lead_success_le_1000ms_total.load(Ordering::Relaxed),
            lead_success_le_2000ms_total: self.lead_success_le_2000ms_total.load(Ordering::Relaxed),
            lead_success_gt_2000ms_total: self.lead_success_gt_2000ms_total.load(Ordering::Relaxed),
            lead_failure_le_250ms_total: self.lead_failure_le_250ms_total.load(Ordering::Relaxed),
            lead_failure_le_500ms_total: self.lead_failure_le_500ms_total.load(Ordering::Relaxed),
            lead_failure_le_1000ms_total: self.lead_failure_le_1000ms_total.load(Ordering::Relaxed),
            lead_failure_le_2000ms_total: self.lead_failure_le_2000ms_total.load(Ordering::Relaxed),
            lead_failure_gt_2000ms_total: self.lead_failure_gt_2000ms_total.load(Ordering::Relaxed),
            result_handling_micros_total: self.result_handling_micros_total.load(Ordering::Relaxed),
            result_handling_completed_total: self
                .result_handling_completed_total
                .load(Ordering::Relaxed),
            source_active: self.source_active.load(Ordering::Relaxed),
            fetch_active: self.fetch_active.load(Ordering::Relaxed),
            metadata_active: self.metadata_active.load(Ordering::Relaxed),
            udp_recv_syscalls_total: self.udp_recv_syscalls_total.load(Ordering::Relaxed),
            udp_recv_successful_syscalls_total: self
                .udp_recv_successful_syscalls_total
                .load(Ordering::Relaxed),
            udp_recv_packets_total: self.udp_recv_packets_total.load(Ordering::Relaxed),
            udp_recv_batch_max_interval: self.udp_recv_batch_max_interval.load(Ordering::Relaxed),
            udp_recv_eagain_total: self.udp_recv_eagain_total.load(Ordering::Relaxed),
            udp_recv_eintr_total: self.udp_recv_eintr_total.load(Ordering::Relaxed),
            udp_recv_errors_total: self.udp_recv_errors_total.load(Ordering::Relaxed),
            udp_recv_truncated_total: self.udp_recv_truncated_total.load(Ordering::Relaxed),
            udp_recv_invalid_addr_total: self.udp_recv_invalid_addr_total.load(Ordering::Relaxed),
            udp_recv_zero_length_total: self.udp_recv_zero_length_total.load(Ordering::Relaxed),
            udp_recv_fatal_total: self.udp_recv_fatal_total.load(Ordering::Relaxed),
        }
    }
}
