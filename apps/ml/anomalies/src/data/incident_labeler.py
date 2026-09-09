import logging
import numpy as np
import pandas as pd
from typing import List, Optional, Dict
from datetime import datetime

logger = logging.getLogger(__name__)

# Incident Class Mapping
INCIDENT_MAP = {
    0: "NORMAL",
    1: "DB_LATENCY_SPIKE",
    2: "DHT_DROP_COLLAPSE",
    3: "TIMEOUT_CASCADE",
    4: "RESTART_EVENT",
}

class IncidentLabeler:
    """
    Automated and semi-supervised ground truth labeler combining
    explicit log evidence, crawler restart markers, and extreme operational outliers.
    """
    def __init__(self):
        pass

    def label_dataset(
        self,
        features_df: pd.DataFrame,
        restart_timestamps: Optional[List[datetime]] = None,
        db_slow_query_df: Optional[pd.DataFrame] = None,
    ) -> pd.Series:
        """
        Produce categorical incident labels (0-4) aligned with features_df.index (hourly timestamps).
        """
        labels = pd.Series(0, index=features_df.index, name="incident_label")

        # 1. Label Crawler Restarts
        if restart_timestamps:
            restart_hours = {pd.to_datetime(ts).floor("1h") for ts in restart_timestamps}
            for hr in features_df.index:
                if hr in restart_hours:
                    labels[hr] = 4  # RESTART_EVENT

        # 2. Label DB Latency Spikes (from logs if available, or severe backpressure)
        if db_slow_query_df is not None and not db_slow_query_df.empty:
            slow_hours = {pd.to_datetime(t).floor("1h") for t in db_slow_query_df["time"].dropna()}
            for hr in features_df.index:
                if hr in slow_hours and labels[hr] == 0:
                    labels[hr] = 1  # DB_LATENCY_SPIKE

        # Fallback/supplemental DB Latency signal: high scheduler blocked rate or channel drops
        if "fresh_channel_dropped_rate" in features_df.columns:
            channel_drop_mask = features_df["fresh_channel_dropped_rate"] > 1000
            for hr in features_df[channel_drop_mask].index:
                if labels[hr] == 0:
                    labels[hr] = 1

        # 3. Label DHT Drops / Routing Table Collapse
        if "routing_table_len_delta" in features_df.columns and "inbound_get_peers_rate" in features_df.columns:
            rt_delta = features_df["routing_table_len_delta"]
            get_peers = features_df["inbound_get_peers_rate"]
            # Severe drop in routing table (> 2,000 nodes lost in an hour) or inbound drops by > 75%
            rt_median = features_df["routing_table_len_mean"].median()
            dht_collapse_mask = (rt_delta < -2000) | (features_df["routing_table_len_mean"] < rt_median * 0.4)
            for hr in features_df[dht_collapse_mask].index:
                if labels[hr] == 0:
                    labels[hr] = 2  # DHT_DROP_COLLAPSE

        # 4. Label Verification / Fetch Timeout Spikes
        if "verify_timeout_ratio" in features_df.columns:
            timeout_spike_mask = (features_df["verify_timeout_ratio"] > 0.85) | (
                features_df.get("fetch_connect_timeout_ratio", 0) > 0.90
            )
            for hr in features_df[timeout_spike_mask].index:
                if labels[hr] == 0:
                    labels[hr] = 3  # TIMEOUT_CASCADE

        counts = labels.value_counts().to_dict()
        readable_counts = {INCIDENT_MAP.get(k, k): v for k, v in counts.items()}
        logger.info(f"Incident label distribution: {readable_counts}")
        return labels
