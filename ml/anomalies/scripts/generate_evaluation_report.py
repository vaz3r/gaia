#!/usr/bin/env python3
import sys
from pathlib import Path

# Add project root to path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import pandas as pd
import numpy as np
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

from config import DATA_DIR
from src.pipeline.detector import AnomalyDetector

def main():
    features_path = DATA_DIR / "hourly_features.parquet"
    if not features_path.exists():
        print("Run extract_features.py and train_models.py first.")
        return

    features_df = pd.read_parquet(features_path)
    detector = AnomalyDetector()
    reports = detector.detect(features_df)

    timestamps = [pd.to_datetime(r["timestamp"]) for r in reports]
    scores = [r["anomaly_score"] for r in reports]
    severities = [r["severity"] for r in reports]
    incidents = [r["predicted_incident"] for r in reports]

    df_res = pd.DataFrame({
        "timestamp": timestamps,
        "score": scores,
        "severity": severities,
        "incident": incidents,
        "get_peers": features_df["inbound_get_peers_rate"].values,
        "routing_table": features_df["routing_table_len_mean"].values,
        "verify_timeout_ratio": features_df["verify_timeout_ratio"].values,
    }).sort_values("timestamp")

    fig, axes = plt.subplots(4, 1, figsize=(14, 12), sharex=True)

    # 1. Anomaly Score & Severity Thresholds
    ax1 = axes[0]
    ax1.plot(df_res["timestamp"], df_res["score"], color="#1f77b4", lw=1.5, label="Ensemble Anomaly Score")
    ax1.axhline(0.85, color="red", linestyle="--", alpha=0.7, label="Critical (0.85)")
    ax1.axhline(0.70, color="orange", linestyle="--", alpha=0.7, label="Warning (0.70)")
    ax1.axhline(0.55, color="yellow", linestyle="--", alpha=0.7, label="Info (0.55)")
    
    # Highlight flagged incidents
    flagged = df_res[df_res["score"] >= 0.70]
    ax1.scatter(flagged["timestamp"], flagged["score"], color="crimson", s=40, zorder=5, label="Flagged Anomaly")
    ax1.set_ylabel("Anomaly Score")
    ax1.set_title("Crawler Operations Telemetry: 21-Day Historical Anomaly Backtest", fontsize=14, fontweight="bold")
    ax1.grid(True, alpha=0.3)
    ax1.legend(loc="upper left")

    # 2. Inbound get_peers rate
    ax2 = axes[1]
    ax2.plot(df_res["timestamp"], df_res["get_peers"] / 1e6, color="#2ca02c", lw=1.2)
    ax2.set_ylabel("get_peers (M / hr)")
    ax2.grid(True, alpha=0.3)
    ax2.set_title("Inbound DHT Traffic (get_peers rate)")

    # 3. Routing Table Length
    ax3 = axes[2]
    ax3.plot(df_res["timestamp"], df_res["routing_table"], color="#9467bd", lw=1.2)
    ax3.set_ylabel("Routing Nodes")
    ax3.grid(True, alpha=0.3)
    ax3.set_title("DHT Routing Table Size")

    # 4. Verification Timeout Ratio
    ax4 = axes[3]
    ax4.plot(df_res["timestamp"], df_res["verify_timeout_ratio"], color="#d62728", lw=1.2)
    ax4.set_ylabel("Timeout Ratio")
    ax4.set_xlabel("Time (UTC)")
    ax4.grid(True, alpha=0.3)
    ax4.set_title("Verification Timeout Ratio (verify_timeouts / attempts)")

    plt.tight_layout()
    out_img = DATA_DIR / "anomaly_backtest_report.png"
    plt.savefig(out_img, dpi=180)
    print(f"[SUCCESS] Saved backtesting chart to {out_img}")

if __name__ == "__main__":
    main()
