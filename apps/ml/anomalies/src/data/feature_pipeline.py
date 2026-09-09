import logging
import numpy as np
import pandas as pd
from typing import List, Optional, Tuple, Dict
from config import COUNTER_METRICS, GAUGE_METRICS, MODEL_FEATURE_NAMES

logger = logging.getLogger(__name__)

class FeaturePipeline:
    def __init__(
        self,
        counter_metrics: List[str] = COUNTER_METRICS,
        gauge_metrics: List[str] = GAUGE_METRICS,
        feature_names: List[str] = MODEL_FEATURE_NAMES,
    ):
        self.counter_metrics = counter_metrics
        self.gauge_metrics = gauge_metrics
        self.feature_names = feature_names

    def build_time_series_matrix(self, raw_df: pd.DataFrame) -> pd.DataFrame:
        """
        Pivot raw (ts, metric_name, metric_value) into a wide DataFrame indexed by minute-level ts.
        """
        if raw_df.empty:
            return pd.DataFrame()

        logger.info("Pivoting raw metrics into wide time series matrix...")
        # Ensure timestamp is datetime and floor to minute
        df = raw_df.copy()
        df["ts"] = pd.to_datetime(df["ts"]).dt.floor("1min")
        
        # In case of duplicate keys within the same minute, take the latest metric value
        pivot_df = df.pivot_table(
            index="ts",
            columns="metric_name",
            values="metric_value",
            aggfunc="last",
        )
        pivot_df = pivot_df.sort_index()
        return pivot_df

    def compute_counter_deltas(self, wide_df: pd.DataFrame) -> pd.DataFrame:
        """
        Compute positive deltas for monotonic counters, resetting on crawler restart or negative diffs.
        """
        df = wide_df.copy()
        counters_present = [col for col in self.counter_metrics if col in df.columns]

        # Forward fill up to 3 consecutive missing minutes, fill remaining NaNs with 0
        df[counters_present] = df[counters_present].ffill(limit=3).fillna(0)

        delta_df = pd.DataFrame(index=df.index)

        # Identify explicit session restarts if present
        session_starts = set()
        if "_session_start" in df.columns:
            session_starts = set(df[df["_session_start"].notnull()].index)

        for col in counters_present:
            vals = df[col].values
            deltas = np.zeros(len(vals), dtype=np.float64)
            
            for i in range(1, len(vals)):
                diff = vals[i] - vals[i - 1]
                # If negative diff or session restart, treat as counter reset
                if diff < 0 or df.index[i] in session_starts:
                    deltas[i] = max(0.0, float(vals[i]))
                else:
                    deltas[i] = max(0.0, float(diff))

            delta_df[f"{col}_delta"] = deltas

        return delta_df

    def aggregate_to_hourly(self, wide_df: pd.DataFrame, delta_df: pd.DataFrame) -> pd.DataFrame:
        """
        Aggregate 1-minute time series into hourly vectors with derived ratios and features.
        """
        logger.info("Aggregating minute metrics into hourly feature vectors...")
        # 1. Sum counter deltas within each hour
        hourly_deltas = delta_df.resample("1h").sum()

        # 2. Compute mean, min, max for gauges within each hour
        gauges_present = [col for col in self.gauge_metrics if col in wide_df.columns]
        gauge_wide = wide_df[gauges_present].ffill().bfill()
        hourly_gauges = gauge_wide.resample("1h").agg(["mean", "min", "max", "std"])
        hourly_gauges.columns = [f"{col}_{stat}" for col, stat in hourly_gauges.columns]

        # Combine into master hourly frame
        hourly = pd.concat([hourly_deltas, hourly_gauges], axis=1).dropna(how="all")

        # 3. Derive high-signal operational rates and ratios
        feats = pd.DataFrame(index=hourly.index)

        # --- Inbound DHT ---
        feats["inbound_get_peers_rate"] = hourly.get("inbound_get_peers_delta", 0)
        feats["inbound_find_node_rate"] = hourly.get("inbound_find_node_delta", 0)
        feats["inbound_announce_peer_rate"] = hourly.get("inbound_announce_peer_delta", 0)
        
        inbound_total = (
            feats["inbound_get_peers_rate"]
            + feats["inbound_find_node_rate"]
            + hourly.get("inbound_dropped_rate_limit_delta", 0)
        )
        feats["inbound_dropped_rate_limit_ratio"] = (
            hourly.get("inbound_dropped_rate_limit_delta", 0) / np.maximum(1.0, inbound_total)
        )

        # --- Routing Table Dynamics ---
        rt_mean = hourly.get("routing_table_len_mean", pd.Series(0, index=hourly.index))
        feats["routing_table_len_mean"] = rt_mean
        feats["routing_table_len_delta"] = rt_mean.diff().fillna(0)
        feats["tx_table_len_mean"] = hourly.get("tx_table_len_mean", 0)

        # --- Verification Health ---
        v_att = hourly.get("verify_attempts_delta", pd.Series(0, index=hourly.index))
        feats["verify_attempts_rate"] = v_att
        feats["verify_success_ratio"] = hourly.get("verify_success_delta", 0) / np.maximum(1.0, v_att)
        feats["verify_fail_ratio"] = hourly.get("verify_fail_delta", 0) / np.maximum(1.0, v_att)
        feats["verify_timeout_ratio"] = hourly.get("verify_timeouts_delta", 0) / np.maximum(1.0, v_att)

        # --- Peer Sourcing Efficiency ---
        sq = hourly.get("source_queries_delta", pd.Series(0, index=hourly.index))
        feats["source_query_rate"] = sq
        feats["source_response_ratio"] = hourly.get("source_responses_delta", 0) / np.maximum(1.0, sq)
        feats["source_timeout_ratio"] = hourly.get("source_timeout_delta", 0) / np.maximum(1.0, sq)
        feats["source_no_peers_ratio"] = hourly.get("source_no_peers_delta", 0) / np.maximum(1.0, sq)
        feats["source_peers_per_query"] = hourly.get("source_peers_returned_delta", 0) / np.maximum(1.0, sq)

        # --- Fetch Pipeline & Transport (ISP/Seedbox Block Indicators) ---
        fa = hourly.get("fetch_attempts_delta", pd.Series(0, index=hourly.index))
        feats["fetch_connect_timeout_ratio"] = hourly.get("fetch_connect_timeout_delta", 0) / np.maximum(1.0, fa)
        
        tcpa = hourly.get("tcp_attempts_delta", pd.Series(0, index=hourly.index))
        feats["tcp_success_ratio"] = hourly.get("tcp_connect_ok_delta", 0) / np.maximum(1.0, tcpa)
        
        utpa = hourly.get("utp_attempts_delta", pd.Series(0, index=hourly.index))
        feats["utp_success_ratio"] = hourly.get("utp_connect_ok_delta", 0) / np.maximum(1.0, utpa)
        
        feats["tcp_to_utp_ratio"] = tcpa / np.maximum(1.0, utpa)
        feats["sha1_mismatch_rate"] = hourly.get("sha1_mismatch_delta", 0)

        # --- Channel Backpressure & Queues ---
        feats["fresh_channel_depth_mean"] = hourly.get("fresh_channel_depth_mean", 0)
        feats["verify_channel_depth_mean"] = hourly.get("verify_channel_depth_mean", 0)
        feats["fresh_channel_dropped_rate"] = hourly.get("fresh_channel_dropped_delta", 0)
        feats["scheduler_send_blocked_rate"] = hourly.get("scheduler_send_blocked_delta", 0)

        # Ensure all designated model features exist
        for col in self.feature_names:
            if col not in feats.columns:
                feats[col] = 0.0

        feats = feats[self.feature_names].fillna(0)
        logger.info(f"Constructed {len(feats)} hourly feature rows across {feats.shape[1]} features.")
        return feats

    def transform_raw_to_features(self, raw_metrics_df: pd.DataFrame) -> pd.DataFrame:
        """
        End-to-end transformation from raw database metrics to model-ready hourly feature DataFrame.
        """
        wide_df = self.build_time_series_matrix(raw_metrics_df)
        delta_df = self.compute_counter_deltas(wide_df)
        hourly_features = self.aggregate_to_hourly(wide_df, delta_df)
        return hourly_features
