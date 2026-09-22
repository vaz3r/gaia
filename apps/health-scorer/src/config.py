"""
Configuration for the health scorer service.

All values are read from environment variables with sensible defaults.
"""

from __future__ import annotations

import os

# Worker settings
WORKER_BATCH_SIZE = int(os.environ.get("WORKER_BATCH_SIZE", 500))
WORKER_POLL_INTERVAL = int(os.environ.get("WORKER_POLL_INTERVAL", 10))
WORKER_ONCE = os.environ.get("WORKER_ONCE", "false").lower() == "true"
WORKER_DRY_RUN = os.environ.get("WORKER_DRY_RUN", "false").lower() == "true"

# Shadow mode: compute and log scores but do not write to torrents
SHADOW_MODE = os.environ.get("SHADOW_MODE", "true").lower() == "true"

# Legacy adapter: when enabled, fall back to legacy evidence derivation
# when no observations are available for a batch. Disabled at M2.5 activation.
LEGACY_ADAPTER_ENABLED = os.environ.get("LEGACY_ADAPTER_ENABLED", "true").lower() == "true"

# Heartbeat file for health checks
HEARTBEAT_PATH = os.environ.get("HEARTBEAT_PATH", "/tmp/health_scorer_heartbeat")

# Algorithm version
ALGORITHM_VERSION = os.environ.get("ALGORITHM_VERSION", "2.0.0")

# Logging
LOG_LEVEL = os.environ.get("LOG_LEVEL", "INFO")

# Failure lookback window (hours): only count failures after latest success within this window
FAILURE_LOOKBACK_HOURS = float(os.environ.get("FAILURE_LOOKBACK_HOURS", 48))

# Failure cap: maximum number of failures to consider (before decay)
FAILURE_CAP = int(os.environ.get("FAILURE_CAP", 3))

# DHT sighting window (hours): count sightings within this window for DHT family
DHT_SIGHTING_WINDOW_HOURS = float(os.environ.get("DHT_SIGHTING_WINDOW_HOURS", 12))

# Tier definitions for recalculation scheduling (M2+)
TIER_RESCORE_INTERVALS = {
    "hot": 15,      # minutes
    "active": 60,
    "warm": 360,
    "cold": 1440,
}
