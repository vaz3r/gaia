# 🛡️ GAIA Ecosystem — Health & Performance Report (Sample)
> **Generated at**: `2026-09-20 14:00:00 UTC`  
> **Fleet Status**: **🟡 DEGRADED PERFORMANCE / BOTTLENECK DETECTED**  

### Executive KPI Snapshot
| Metric | Current Value | SLA / Target | Status |
| :--- | :--- | :--- | :--- |
| **Hourly Ingestion Rate** | **48,225 torrents/hr** (+221.7% vs 24h avg) | >= 10,000/hr | 🟢 Normal |
| **Active Containers** | **23 / 24 containers online** | 100% online | 🟡 Degraded |
| **Unclassified Backlog** | **9,208 records** | <= 25,000 | 🟢 Drained |
| **PostgreSQL Cache Hit** | **53.88%** | >= 99.00% | 🔴 Disk-Bound |
| **Search Replication Lag** | **44,310 documents** | <= 10,000 | 🟡 Replicating |
| **WireGuard Tunnel Latency**| **2.33 ms** (56s handshake) | < 50ms / <= 180s | 🟢 Stable |

---

## 1. Ingestion & Crawler Dynamics
- **Total Database Torrents**: `2,816,714`
- **24h Rolling Volume**: `359,773` torrents (Hourly Moving Avg: `14,990` torrents/hr)
- **DHT Swarm Routing Table**: `13,727` active nodes
- **BEP-9 Wire Metadata Success**: `35.5%` (333,798 hashes / 940,416 attempts)
- **DHT Inbound Queries**: `get_peers=1,258,266`, `announce=22,960`

#### Hourly Ingestion History (Last 12 Hours)
| Hour Slot (UTC) | Verified Torrents | Throughput (Avg/Min) | Trend vs Baseline |
| :--- | :--- | :--- | :--- |
| `2026-09-20 11:00` | **3,781** | 63.0 / min | 🟢 +0.0% |
| `2026-09-20 10:00` | **48,225** | 803.8 / min | 🟢 +221.7% |
| `2026-09-20 09:00` | **30,537** | 509.0 / min | 🟢 +103.7% |
| `2026-09-20 08:00` | **20,636** | 343.9 / min | 🟢 +37.7% |
| `2026-09-20 07:00` | **15,144** | 252.4 / min | 🟢 +1.0% |
| `2026-09-20 06:00` | **13,297** | 221.6 / min | 🟡 -11.3% |
| `2026-09-20 05:00` | **14,107** | 235.1 / min | 🟡 -5.9% |
| `2026-09-20 04:00` | **13,619** | 227.0 / min | 🟡 -9.1% |
| `2026-09-20 03:00` | **13,813** | 230.2 / min | 🟡 -7.9% |
| `2026-09-20 02:00` | **13,164** | 219.4 / min | 🟡 -12.2% |
| `2026-09-20 01:00` | **12,185** | 203.1 / min | 🟡 -18.7% |
| `2026-09-20 00:00` | **13,080** | 218.0 / min | 🟡 -12.7% |

---

## 2. Classification & Machine Learning Intelligence
- **Classified Torrents**: `2,807,506` | **Quality Scored**: `2,816,714`
- **Latest Batch**: `2000` records | **Processing Velocity**: `84 rec/s`
- **Batch Verdict Breakdown**: Accepted `60.6%` | Flagged `39.5%`

#### Batch Execution Timing Breakdown
| Pipeline Phase | Elapsed Time | Proportion of Batch | Bottleneck Status |
| :--- | :--- | :--- | :--- |
| **PostgreSQL Record Fetch** | `1.07s` | — | 🟢 Fast |
| **LightGBM Model Inference** | `3.26s` | `13.7%` | 🟢 Normal |
| **PostgreSQL Batch Commit** | `19.42s` | `81.8%` | 🔴 Lock Contention |

- **ML Unsupervised Anomaly Engine**: Latest Score `0.244` (Severity: `NORMAL`) on window `2026-09-20 10:00:00+00:00`
- **Quality Scoring Velocity**: Scored `1,589` | Refreshed `2,500`

---

## 3. Storage, Database & Search Replication
- **PostgreSQL Master Size**: `74 GB` | **Buffer Cache Hit Ratio**: `53.88%`
- **Connections**: `8` Active | `10` Idle | `3` In-Transaction | `26 / 100` Pool
- **Contention Indicators**: `0` Waiting Locks | `2` Slow Queries (>5s)
- **Redis In-Memory Cache**: Used `1.21M / 1.00G` | Hit Rate: `0.0%` | Evictions: `0`
- **Meilisearch Search Engine**: `2,801,013` indexed documents (`available`)
- **CDC Replication Status**: `HEALTHY` | Last Rebuild: `68` mins ago | Streaming: `2565 docs/s`
- **Disaster Recovery Backup**: `SUCCESS` (Last dump: `2026-09-19T02:00:00+00:00`)

---

## 4. Bottleneck & Anomaly Diagnostic Matrix
### 1. 🔴 [CRITICAL] Sub-Optimal Buffer Cache Hit Ratio (53.88%)
- **Subsystem / Component**: `Storage / Database` / `PostgreSQL (craw)`
- **Evidence**: PostgreSQL buffer cache hit ratio is 53.88% (SLA >= 99.0%). Over 40% of page requests read from physical NVMe disk.
- **Root Cause**: Database size (74 GB) exceeds configured shared_buffers memory allocation.
- **Remediation Action**: `Increase shared_buffers in deploy/postgres/postgresql.workspace-production.conf and optimize top relation queries.`

### 2. 🟡 [WARNING] Database Commit Contention (81.8% of batch time)
- **Subsystem / Component**: `Classification Pipeline` / `gaia-classifier`
- **Evidence**: Batch write step took 19.42s while model inference took only 3.26s.
- **Root Cause**: Row-level lock contention on `torrents` table during concurrent crawler ingestion and classifier batch updates.
- **Remediation Action**: `Tune WORKER_BATCH_SIZE: 500 in deploy/targets/workspace-production/docker-compose.yml to commit updates in smaller, non-blocking transactions.`
