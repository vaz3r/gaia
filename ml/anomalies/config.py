import os
from pathlib import Path

# Base Paths
BASE_DIR = Path(__file__).resolve().parent
DATA_DIR = BASE_DIR / "data"
MODELS_DIR = BASE_DIR / "models_storage"
DATA_DIR.mkdir(exist_ok=True, parents=True)
MODELS_DIR.mkdir(exist_ok=True, parents=True)

# Database Configuration
DB_HOST = os.getenv("DB_HOST", "workspace-production")
DB_PORT = int(os.getenv("PG_PORT", os.getenv("DB_PORT", "5432")))
DB_USER = os.getenv("POSTGRES_USER", os.getenv("DB_USER", "crawler"))
DB_NAME = os.getenv("POSTGRES_DB", os.getenv("DB_NAME", "craw"))
DB_PASSWORD = os.getenv(
    "PG_PASSWORD",
    os.getenv("DB_PASSWORD", "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b"),
)

# Log Analyzer API Configuration
LOG_ANALYZER_URL = os.getenv("LOG_ANALYZER_URL", "http://workspace-production:3001")

# Metric Taxonomy
# 1. Monotonic Cumulative Counters (require rate/delta calculation with reset detection)
COUNTER_METRICS = [
    # DHT Inbound
    "inbound_get_peers",
    "inbound_announce_peer",
    "inbound_find_node",
    "inbound_ping",
    "inbound_invalid",
    "inbound_dropped_rate_limit",
    "tokens_issued",
    "infohashes_harvested",
    "unique_infohashes",
    "inbound_get_peers_bep42",
    "inbound_get_peers_random",
    "inbound_find_node_bep42",
    "inbound_find_node_random",
    "inbound_announce_bep42",
    "inbound_announce_random",
    # Verification Pipeline
    "verify_attempts",
    "verify_success",
    "verify_fail",
    "verify_timeouts",
    # Sourcing & Discovery
    "source_queries",
    "source_responses",
    "source_peers_returned",
    "source_timeout",
    "source_no_peers",
    "source_all_timeout",
    "source_returned_peers",
    "source_filtered_by_cache",
    # Fetch & Transports
    "fetch_attempts",
    "fetch_connect_timeout",
    "fetch_connect_io",
    "fetch_handshake",
    "fetch_no_extension",
    "fetch_reject",
    "fetch_bad_piece",
    "fetch_io",
    "sha1_mismatch",
    "tcp_attempts",
    "utp_attempts",
    "tcp_connect_ok",
    "utp_connect_ok",
    "tcp_metadata_ok",
    "utp_metadata_ok",
    "metadata_failed_io",
    "metadata_failed_silent",
    "metadata_timeout",
    # Channel drops & scheduler events
    "harvest_try_send_dropped",
    "scheduler_send_blocked",
    "fresh_channel_dropped",
]

# 2. Instantaneous Gauges (State at sample point: aggregate by mean, min, max, std)
GAUGE_METRICS = [
    "routing_table_len",
    "tx_table_len",
    "peer_cache_size",
    "verify_channel_depth",
    "fresh_channel_depth",
]

# Primary Features for Anomaly Detection Models
MODEL_FEATURE_NAMES = [
    # DHT Health & Inbound Rates
    "inbound_get_peers_rate",
    "inbound_find_node_rate",
    "inbound_announce_peer_rate",
    "inbound_dropped_rate_limit_ratio",
    "routing_table_len_mean",
    "routing_table_len_delta",
    "tx_table_len_mean",
    # Verification Health
    "verify_attempts_rate",
    "verify_success_ratio",
    "verify_fail_ratio",
    "verify_timeout_ratio",
    # Peer Sourcing Efficiency
    "source_query_rate",
    "source_response_ratio",
    "source_timeout_ratio",
    "source_no_peers_ratio",
    "source_peers_per_query",
    # Fetch Pipeline & Transport (ISP/Seedbox block indicators)
    "fetch_connect_timeout_ratio",
    "tcp_success_ratio",
    "utp_success_ratio",
    "tcp_to_utp_ratio",
    "sha1_mismatch_rate",
    # Queue Backpressure & Channel Drops
    "fresh_channel_depth_mean",
    "verify_channel_depth_mean",
    "fresh_channel_dropped_rate",
    "scheduler_send_blocked_rate",
]

# Anomaly Severity Thresholds
ALERT_THRESHOLDS = {
    "info": 0.55,
    "warning": 0.70,
    "critical": 0.85,
}
