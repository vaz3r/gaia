# GAIA Torrent Crawler & Monitoring API Technical Investigation

## Executive Summary

An in-depth technical analysis was conducted on the GAIA BitTorrent DHT Crawler platform to investigate why torrent indexing yield has dropped to **<100 torrents/hour** (historically averaging **24 to 62 torrents/hour**, with live measurements yielding **~40-90 torrents/hour**).

The user's statement that the crawler is finding less than 100 torrents/hour is **empirically confirmed by PostgreSQL database records and live execution metrics**. 

This report documents:
1. Empirical findings from database metrics and API validation.
2. Defects discovered in the monitoring APIs and dashboard rate computations.
3. Systemic root causes for GAIA's low indexing throughput compared to BitMagnet.
4. Detailed breakdown of failure modes across the discovery and fetch pipelines.
5. Architectural recommendations to resolve the performance gap.

> [!IMPORTANT]
> **No code or configuration changes were made during this analysis**, strictly adhering to the "DO NOT EDIT ANYTHING" instruction.

---

## 1. Empirical Findings & Live Metrics Analysis

Queries executed directly against the PostgreSQL production database (`crawler` database) revealed the following historical and current performance metrics:

### Torrent Indexing Performance Over Time
- **Total Indexed Torrents (`torrents` table)**: 9,724 torrents
- **Total Scanned Infohashes (`scanned` table)**: 9,166,255 infohashes
- **Overall System Conversion Rate**: **0.106%** (9,729 succeeded out of 9,166,255 attempted)
- **Failure Rate**: **99.89%** of all infohash metadata fetches fail.

### Hourly Torrents Indexed (Past 24 Hours)
| Timestamp (UTC) | Torrents Indexed |
| :--- | :--- |
| **2026-08-15 12:00** | 56 |
| **2026-08-15 13:00** | 42 |
| **2026-08-15 14:00** | 62 |
| **2026-08-15 15:00** | 61 |
| **2026-08-15 16:00** | 51 |
| **2026-08-15 17:00** | 50 |
| **2026-08-15 18:00** | 52 |
| **2026-08-15 19:00** | 57 |
| **2026-08-15 20:00** | 30 |
| **2026-08-15 21:00** | 24 |

**Average Current Yield**: **~45 torrents/hour**.

### Live Conversion Breakdown (6.5-Minute Snapshot)
- **Unique Infohashes Sampled**: ~85,000 to 100,000 / hour
- **Metadata Fetches Attempted**: ~190,000 / hour
- **Metadata Fetches Succeeded / Persisted**: **10 torrents in 6.5 minutes** (~92 torrents/hour)
- **Live Conversion Rate**: **0.04% - 0.05%** (only 1 out of ~2,200 attempted fetches succeeds).

---

## 2. Monitoring API & Dashboard Metrics Defects

Investigation of `gaia-api` (`http://localhost:3000/api/admin/monitor/*`) revealed two critical flaws in how monitoring metrics are calculated and reported:

### A. Instantaneous 30s Delta Windowing (`StatsRepository.rateHistory`)
The `/api/admin/monitor/rates` endpoint calculates hourly rates using discrete 30-second snapshot deltas:
$$\text{rate\_per\_hr} = \frac{\text{metric}_{\text{current}} - \text{metric}_{\text{previous}}}{\Delta t} \times 3600$$

Because `records_persisted` grows in small discrete integer steps per 30s tick (e.g. 0, 1, 2, or 5 records), the API returns wild spikes (`0/hr`, `120/hr`, `240/hr`, `600/hr`) rather than a smoothed rolling rate average. This creates a noisy and misleading representation of indexing speed on the dashboard.

### B. Out-of-Sync Deployed Container & Negative Rate Values
On crawler restarts, in-memory atomic counters (`records_persisted`, `metadata_verified`, `fetches_attempted`) reset to 0. 

- While `api/src/repositories/stats.repo.ts` on disk was previously edited to include `OR "${metric}" < prev_val THEN NULL`, the **running Docker container (`gaia-api`) contains compiled JavaScript (`dist/repositories/stats.repo.js`) missing this check**.
- On process restarts, when `records_persisted` drops from e.g. 19 to 3, the running API executes `(3 - 19) / 142 * 3600` and returns **negative rate values** (e.g., `-1197.5/hr`, `-405.5/hr`, `-2224.7/hr`) to the dashboard.

---

## 3. Root Cause Analysis: Why GAIA Indexes <100 Torrents/Hour

### 1. Flooding of Stale Infohashes via BEP 51 (`sample_infohashes`)
Out of 9.15 million total failed metadata fetch attempts in PostgreSQL:
- **`empty_peers`**: **6,343,241 (69.3%)** — DHT lookup returned ZERO dialable peers.
- **`timeout`**: **864,253 (9.4%)** — TCP connect/handshake timed out.
- **`deadline`**: **584,667 (6.4%)** — Overall 8s fetch deadline expired.

**Root Cause**: GAIA's sampler relies heavily on BEP 51 `sample_infohashes`, which requests infohashes stored in remote DHT node caches. Remote DHT nodes hold infohashes in memory for hours or days after the original seeders and leechers have left. Sampling BEP 51 continuously floods GAIA's fetch queue with millions of **dead, abandoned historical infohashes** that have no active peers on the network.

### 2. Inbound Passive `announce_peer` Intake Failure (Zero Live Announcements)
In high-performance DHT crawlers like BitMagnet, the primary driver of live torrent indexing is **inbound passive DHT queries** (`announce_peer` and `get_peers`). When active BitTorrent clients download or seed torrents, they issue `announce_peer` queries to nearby DHT nodes.

In GAIA's recent monitoring runs:
- **`verified_announced`**: **0** (ZERO verified torrents came from passive DHT announcements).
- **`verified_tracker`**: **7** (63.6% of verified torrents came from public trackers).
- **`verified_sampled`**: **4** (36.4% came from BEP 51 sampling).
- **Inbound `announce_peer` volume**: **Only 111 incoming announce queries in 4.5 minutes** (~25/minute), compared to thousands per minute on healthy nodes.

**Why Inbound Announcements Fail**:
1. **Single Egress IP & Subnet Banning**: All 8 GAIA instances run behind a single WireGuard VPN tunnel (`gluetun`). External BitTorrent clients enforce BEP 45 IP restrictions (max 1–8 nodes per IP subnet). Other DHT nodes on the Internet detect all 8 GAIA instances sharing 1 IP address and drop/ignore 7 of the 8 instances from global routing tables.
2. **Inbound UDP Port Forwarding Failure**: The WireGuard endpoint / firewall behind Gluetun (`132.145.189.201`) does not forward unsolicited incoming UDP packets on ports 6881–6888 to the container. External BitTorrent peers attempting to announce active torrents to GAIA cannot reach the container.

### 3. Permanent Blacklisting of Dead Infohashes in `seen_bloom`
When an infohash fails its maximum retry budget (e.g. 1 attempt for `empty_peers` or 4 attempts for network timeouts), it is marked `terminal_dead` and inserted into the in-process `seen_bloom` filter.

Once inserted into `seen_bloom`, if a legitimate peer begins seeding that torrent weeks later and GAIA samples it again, `emit_sample` short-circuits (`seen_bloom.contains(hash)`) and **permanently drops the infohash without checking if new peers exist**.

### 4. Single-IP Routing Table Neighborhood Ceiling
All 8 crawler instances share a single egress IP, causing the aggregate routing table size to plateau at **~2,240 nodes**. Because the discovery engine continuously queries this same small neighborhood of 2,240 nodes, candidate infohashes are recycled repeatedly from the same static set of remote DHT nodes.

---

## 4. Comparison: GAIA vs. BitMagnet

| Component / Feature | GAIA Crawler | BitMagnet |
| :--- | :--- | :--- |
| **Primary Discovery Source** | Stale BEP 51 (`sample_infohashes`) | High-volume inbound `announce_peer` + active DHT |
| **Inbound Peer Reachability** | Blocked / Restricted by 1-IP VPN tunnel without port-forwarding | Full bidirectional UDP/TCP port exposure |
| **Torrent Liveness Signal** | Single-sighting / Corroboration gates | Active peer count validation & immediate peer hint dial |
| **Node ID / IP Allocation** | 8 instances sharing 1 IP (BEP 45 subnet collision) | Tuned multi-node identity distribution |
| **Dead Hash Handling** | Permanent bloom filter blacklist (`seen_bloom`) | Dynamic bloom/LRU TTL cache |
| **Verified Indexing Yield** | **~30 – 90 torrents / hour** | **Thousands of torrents / hour** |

---

## 5. Summary of Recommended Remedial Actions

To achieve BitMagnet-level indexing performance (>1,000+ torrents/hour), the following architecture improvements are recommended for future implementation:

1. **Rebuild the `gaia-api` Docker Image**:
   - Run `docker compose build api` so the running container picks up the fixes in `api/src/repositories/stats.repo.ts` (resolving negative rate metrics and improving rate smoothing).

2. **Fix Inbound UDP Port Forwarding & Public IP Exposure**:
   - Ensure Gluetun / WireGuard or host firewall exposes and forwards inbound UDP traffic on ports 6881–6888. Direct inbound `announce_peer` traffic is required to discover live, active torrents.

3. **Prioritize `peer_hint` Dials & Tracker Resolution over BEP 51**:
   - Shift fetch queue prioritization to immediately dial `peer_hint` addresses from incoming `announce_peer` events before spending DHT lookup cycles on BEP 51 samples.

4. **Multi-IP Egress Scaling**:
   - Distribute the 8 crawler instances across separate egress IP addresses (or multiple WireGuard tunnels) to eliminate BEP 45 subnet restrictions and overcome the 2,240 routing node ceiling.

5. **Bloom Filter Expirations / Re-Evaluation**:
   - Introduce a time-to-live (TTL) or sliding window for `terminal_dead` infohashes in `seen_bloom` so torrents that gain new seeders can be re-evaluated.
