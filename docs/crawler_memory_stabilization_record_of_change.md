# Record of Change: Crawler Memory Stabilization & Zero-Degradation Resource Optimization

**Date:** 2026-09-09  
**Target Component:** `apps/crawler` (`gaia-crawler` on host `gaia` / `100.66.211.64`)  
**Status:** In Progress / Staged for Deployment  

---

## 1. Context & Motivation
Following the CPU optimization phase (which successfully brought total system CPU usage down from ~190% to ~24% via radial routing table walks, stack-allocated compact KRPC encoding, and `jemalloc`), long-term monitoring over a 30-minute window revealed persistent memory growth:
- **05:47 UTC:** 145 MiB RSS at startup.
- **06:07 UTC (+20 min):** 1.45 GiB RSS.
- **06:24 UTC (+37 min):** 1.83 GiB RSS (`1,916,736 kB` dirty anonymous heap).

The objective is to strictly cap crawler memory consumption to **< 400 MiB RSS** and maintain **< 30% CPU load** while preserving maximum throughput (**>= 50,000 verified torrents/hour**) and **100% DHT discovery response (`CRAW_FIND_NODE_RESPONSE_PERCENT=100`)**.

---

## 2. Root Cause Analysis
Profiling via `/proc/<pid>/smaps_rollup`, `pmap -x`, and static codebase analysis identified four primary drivers of memory bloat:

1. **Unbounded Telemetry Write Buffer (`PeerOutcomeWriter`):**
   - Location: `apps/crawler/src/storage/peer_outcomes.rs`
   - Ingest rate: At peak load (~60,000+ torrents/hr with 12 race peers), ~140 connection attempts/second occur, each pushing a `PeerOutcome` struct with 8 heap strings into `Mutex<Vec<PeerOutcome>>`.
   - Flush frequency: Default was once every 30 seconds (`peer_outcomes_flush_interval_secs: 30`) with a small chunk size of 256. Slow database network latency caused tens of thousands of debug records to back up in RAM.
   - Impact: Hundreds of megabytes of heap-allocated telemetry records.

2. **Unbounded Sighting Write Buffer (`SightingWriter`):**
   - Location: `apps/crawler/src/storage/sightings.rs`
   - The buffer `Mutex<Vec<(Infohash, Source)>>` lacked an upper bound. During sudden DHT discovery spikes, un-flushed sighting pairs accumulate without limit.

3. **Excessive Channel Depths:**
   - Location: `apps/crawler/src/config.rs`, `apps/crawler/src/main.rs`
   - Discovery, verification, and announcement channels had `channel_capacity = 65,536`.
   - Given concurrency limits of 2,000 tasks (`CRAW_PIPELINE_LIMIT`), 60,000+ items simply sat idle in memory queues waiting to be dequeued. Stale items in the queue decayed before being dialled.

4. **Over-Sized In-Memory Caches & Weak Eviction Policy:**
   - Location: `apps/crawler/src/verify/peer_cache.rs`, `apps/crawler/src/verify/mod.rs`
   - `PeerCache` (bad peer IP tracking) had a 600-second TTL and 100,000 capacity. Remote dynamic BitTorrent peers churn within 120 seconds.
   - When capacity was exceeded, `enforce_bound()` only removed `excess / 8` entries. Because incoming discovery rates exceeded `excess / 8`, the collections grew beyond target bounds.

5. **`jemalloc` Page Retention:**
   - Without aggressive decay settings, jemalloc retained dirty/muzzy arena pages in memory instead of notifying Linux via `madvise(MADV_DONTNEED)`.

---

## 3. Changes Implemented

### A. Storage Writer Bounds & Fast Flush
1. **`PeerOutcomeWriter` (`apps/crawler/src/storage/peer_outcomes.rs`):**
   - Added a hard bound (`MAX_QUEUE_LEN = 10,000`). When full, new telemetry logs are dropped instead of exhausting system RAM.
   - Reduced default flush interval from 30 seconds to 2 seconds.
   - Increased SQL insert chunk size from 256 to 1000 to minimize database round-trips.
2. **`SightingWriter` (`apps/crawler/src/storage/sightings.rs`):**
   - Added a hard bound (`MAX_QUEUE_LEN = 25,000`).

### B. Eviction Logic Fix in Caches
1. **`PeerCache` (`apps/crawler/src/verify/peer_cache.rs`):**
   - Changed `enforce_bound()` to purge all `excess` entries immediately when over capacity, rather than trimming `excess / 8`.
2. **`ConnLimiter` (`apps/crawler/src/verify/mod.rs`):**
   - Changed `enforce_bound()` to purge all `excess` entries immediately when over capacity.

### C. Configuration & Environment Sizing
1. **Queue Capacities (`.env` and `config.rs`):**
   - `CRAW_CHANNEL_CAPACITY`: Reduced from 65,536 to 8,192 (absorbs bursts up to 4x pipeline limit while preventing queue bloat).
   - `CRAW_FRESH_CHANNEL_CAPACITY`: Reduced from 65,536 to 8,192.
2. **Cache Retention (`.env`):**
   - `CRAW_PEER_CACHE_TTL_SECS`: Reduced from 600s to 120s.
   - `CRAW_PEER_CACHE_MAX_ENTRIES`: Reduced from 100,000 to 25,000.
   - `CRAW_ANNOUNCE_CACHE_MAX_ENTRIES`: Reduced from 250,000 to 50,000.
   - `CRAW_CONN_LIMITER_MAX_ENTRIES`: Reduced from 100,000 to 25,000.
3. **Allocator Tuning (`.env`):**
   - `MALLOC_CONF`: Set to `background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500`.

---

## 4. Why Throughput is Preserved
- **Non-blocking Telemetry:** Dropping excess `fetch_peer_outcomes` during DB saturation only drops debug records; it does not touch the torrent download pipeline or metadata saving in `torrents`.
- **Higher Peer Freshness:** Reducing queue lengths prevents torrent infohashes from languishing in queues for minutes after their swarms have moved.
- **Unchanged Verification Concurrency:** `CRAW_FETCH_LIMIT` (1,500 concurrent connections) and `CRAW_PIPELINE_LIMIT` (2,000 active infohash tasks) remain untouched.
- **Unchanged DHT Responsiveness:** `CRAW_FIND_NODE_RESPONSE_PERCENT=100` and radial routing table lookups remain 100% responsive with zero packet dropping.

---

## 5. Rollback Procedure
If unexpected regressions occur:
1. **Revert Git commit:**
   ```bash
   git revert HEAD
   git push origin main
   ```
2. **Restore `.env` on `gaia`:**
   ```bash
   # Revert CRAW_PEER_CACHE_TTL_SECS=600, CRAW_PEER_CACHE_MAX_ENTRIES=100000, CRAW_ANNOUNCE_CACHE_MAX_ENTRIES=250000
   ssh gaia "nano /home/ubuntu/gaia/deploy/targets/gaia-node/.env"
   ```
3. **Redeploy previous build:**
   ```bash
   ./deploy/scripts/deploy-gaia-node.sh
   ```

---

## 6. Verification Checklist
- [ ] Run `docker stats --no-stream gaia-crawler` on host `gaia`: CPU < 50% of 1 core, Memory < 400 MiB.
- [ ] Verify `uptime` on `gaia`: load average < 1.0.
- [ ] Verify `free -m`: 0 MB swap used.
- [ ] Check DB ingest metrics for verified torrents/hour to ensure >= 50,000 torrents/hour.
