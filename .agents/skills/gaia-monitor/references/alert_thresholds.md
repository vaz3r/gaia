# GAIA Operational Alert & Performance Thresholds

This document establishes the official performance thresholds and SLA criteria utilized by the `diagnose_bottlenecks.py` engine and the `gaia-monitor` skill.

---

## Threshold Matrix

| Subsystem | Metric | Normal / Healthy (🟢) | Elevated / Warning (🟡) | Critical Outage (🔴) |
| :--- | :--- | :--- | :--- | :--- |
| **Host System** | CPU Load Average (15m) | `<= 12.0` | `> 12.0` | `> 16.0` |
| **Host System** | RAM Available | `>= 1.5 GB` | `< 1.0 GB` | `< 500 MB` |
| **Host System** | Root Disk Space Used | `<= 80%` | `> 80%` | `> 90%` |
| **Host System** | Data Disk Space Used | `<= 80%` | `> 80%` | `> 90%` |
| **Container Fleet** | State | `running` | — | `exited`, `dead`, `restarting` |
| **Container Fleet** | Health Status | `healthy` / `none` | `starting` | `unhealthy` |
| **Container Fleet** | Restart Count (24h) | `0` | `1 - 3` | `> 3` |
| **Crawler & Ingestion**| Ingestion Rate vs 24h Avg | `>= 80%` of avg | `50% - 79%` of avg | `< 50%` of avg or `0/hr` |
| **Crawler & Ingestion**| DHT Routing Table Size | `> 5,000 nodes` | `2,000 - 5,000` | `< 2,000 nodes` |
| **Crawler & Ingestion**| BEP-9 Wire Success Rate | `>= 40.0%` | `20.0% - 39.9%` | `< 20.0%` |
| **Classification** | Unclassified Backlog | `<= 25,000` | `> 50,000` | `> 200,000` |
| **Classification** | DB Write Time Ratio | `<= 30%` of batch | `31% - 50%` of batch | `> 50%` of batch |
| **Classification** | Inference Throughput | `>= 50 rec/s` | `10 - 49 rec/s` | `< 10 rec/s` |
| **ML Intelligence** | Anomaly Detector Score | `<= 0.40` | `0.41 - 0.60` | `> 0.60` (High Anomaly) |
| **Database** | Buffer Cache Hit Ratio | `>= 99.0%` | `95.0% - 98.9%` | `< 95.0%` |
| **Database** | Slow Queries (> 5s) | `0` | `1 - 3` | `> 3` |
| **Database** | Active Locks Waiting | `0` | `1 - 2` | `> 2` |
| **Search Sync** | Meilisearch CDC Sync Lag | `<= 5,000 docs` | `5,001 - 25,000 docs` | `> 25,000 docs` |
| **Search Sync** | Last Full Rebuild Age | `<= 135 mins` | `136 - 180 mins` | `> 180 mins` |
| **Networking & OpSec**| WireGuard Handshake Age | `<= 180s` | `181s - 300s` | `> 300s` (Severed) |
| **Networking & OpSec**| Tunnel Ping Latency | `< 50 ms` | `50 - 150 ms` | `> 150 ms` or packet loss |
| **Networking & OpSec**| OpSec Killswitch Status | `ACTIVE (+DoT, Port 80/443 blocked)` | — | `LEAK DETECTED` |
| **Disaster Recovery** | Last Backup Completion | `<= 26 hours ago` | `26 - 36 hours ago` | `> 36 hours ago` |
