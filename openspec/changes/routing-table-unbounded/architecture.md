# GAIA Crawler System Architecture

## End-to-End Data Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        DHT NETWORK                                  │
│   (thousands of BitTorrent nodes supporting BEP 5/42/44/51)        │
└───────────────┬─────────────────────────────────────┬───────────────┘
                │                                     │
    ┌───────────▼───────────┐             ┌───────────▼───────────┐
    │   SAMPLER (BEP 51)    │             │   PASSIVE INTAKE      │
    │   - sample_infohashes │             │   - announce_peer     │
    │   - picks random nodes│             │   - get_peers events  │
    │   - returns infohashes│             │   - live peer hints   │
    └───────────┬───────────┘             └───────────┬───────────┘
                │                                     │
                │  FetchRequest {                     │  FetchRequest {
                │    source: Sampled,                 │    source: Announced,
                │    peer_hint: None,                 │    peer_hint: Some(addr),
                │    lookup_seed: Some(addr)          │    lookup_seed: None
                │  }                                  │  }
                │                                     │
                └───────────────┬─────────────────────┘
                                │
                    ┌───────────▼───────────┐
                    │   HASH QUEUE          │
                    │   (priority: hinted   │
                    │    > sampled)         │
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   FETCHER POOL        │
                    │   (concurrency: 200)  │
                    │   - fetch_one()       │
                    │   - dial_peers()      │
                    │   - wire protocol     │
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   WRITE LOOP          │
                    │   (batch: 256,        │
                    │    flush: 2s)         │
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   POSTGRESQL          │
                    │   - torrents table    │
                    │   - scanned table     │
                    └───────────────────────┘
```

## Key Components

### 1. DHT Actor (per instance)
- Handles inbound queries (announce_peer, get_peers)
- Manages routing table, UDP socket, pending queries
- Rate-limited by token bucket (QPS budget)

### 2. Routing Table
- **Structure:** Flat `BTreeMap<Id20, RoutingNode>` (uncapped, 500k safety ceiling)
- **Eviction:** Only on failure (fail_count > 2) or LRU when over capacity
- **Growth:** Fed by grower's `find_node` batches (128 nodes/tick) + `get_peers` walkers
- **Current state:** 80k-120k nodes per instance (✅ exceeds old 2,400 cap)

### 3. Sampler (BEP 51)
- **Purpose:** Query random DHT nodes with `sample_infohashes` to discover infohashes
- **Flow:** `pick_target()` → `sample_infohashes_from()` → `emit_sample()`
- **Dedup:** Bloom filter (in-memory) → Redis sets → PostgreSQL scanned table
- **Liveness:** Tracks distinct sources per hash (min_seen threshold)

### 4. Passive Intake
- **Purpose:** Capture hashes from inbound `announce_peer` and `get_peers` events
- **Advantage:** These hashes have proven live peers (high conversion rate)
- **Dedup:** Redis sets (separate from sampler's dedup)

### 5. Fetcher
- **Purpose:** Fetch metadata from peers for each infohash
- **Three phases:**
  - Phase A: Peer hint fast path (direct dial to hinted peer)
  - Phase B: Direct get-peers to reporting node (sampled hashes only)
  - Phase C: Tracker fallback + iterative DHT walk (announced/hinted only)
- **Key design:** Sampled hashes fail-fast after Phase B (no Phase C)

### 6. Storage
- **PostgreSQL:** Persistent data (torrents, scan status, stats history)
- **Redis:** Ephemeral coordination (dedup sets, dead peers, node pool, sampler state)

## Fetch Pipeline Detail

```
fetch_one(info_hash, peer_hint, source, lookup_seed)

  ┌─────────────────────────────────────────────────────────────────┐
  │ PHASE A: PEER HINT (only if peer_hint.is_some())               │
  │   - Dial the hinted peer directly                              │
  │   - If success → ACCEPTED                                      │
  │   - If failure → continue to Phase B/C                         │
  └─────────────────────────────────────────────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────┐
  │ PHASE B: DIRECT GET-PEERS (only for sampled hashes)            │
  │   - One-shot get_peers to reporting node                       │
  │   - If peers returned → dial_peers()                           │
  │   - If success → ACCEPTED                                      │
  │   - If no peers → FAIL-FAST (return EmptyPeers)                │
  │                                                                 │
  │ ⚠️  SAMPLED HASHES NEVER REACH PHASE C                         │
  └─────────────────────────────────────────────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────┐
  │ PHASE C: TRACKER + ITERATIVE DHT (announced/hinted only)       │
  │   - Query 24 public trackers (UDP + HTTP)                      │
  │   - If peers → dial_peers()                                    │
  │   - If no peers → iterative DHT walk                           │
  │   - If success → ACCEPTED                                      │
  │   - If exhausted → EmptyPeers                                   │
  └─────────────────────────────────────────────────────────────────┘
```

## The Fetch Failure Problem

### Current Metrics (Bench5)
- **Fetch failure rate:** 94%
- **empty_peers:** 50% of failures
- **connect_timeout:** 48% of failures
- **metadata_verified:** ~47/hr (target: 10k/hr)

### Root Cause Analysis

**Why 94% failure?**

1. **Sampled hashes dominate the fetch pool** (most hashes come from BEP 51 sampling)
2. **Sampled hashes use the cheap path** (Phase B only, no trackers/DHT walk)
3. **Phase B fails when:** reporting node is offline, returns empty peers, or peers are dead
4. **Fail-fast at line 512-517:** Returns `EmptyPeers` immediately, no Phase C
5. **By design:** Comment says "a single direct get_peers to the reporting node. If it returns NO peer values, the hash is dropped immediately -- NO tracker fallback, NO iterative DHT walk."

**Why does Phase B fail?**

- BEP 51 nodes store hashes but may not have live peers
- The reporting node may be temporarily unreachable (3s timeout)
- Peers returned may be dead/blocklisted
- The swarm may be dead (no active seeders)

**Why is this a problem?**

- Bitmagnet's equivalent flow also uses 94% fast-fail, but it has **131k unique/hr** (vs our 28k-129k volatile)
- At 0.65% conversion: need 1.4M unique/hr for 10k verified/hr
- We're at 28k-129k unique/hr → 182-839 verified/hr (theoretical)
- Actual: 47/hr → something else is wrong

## The Sampler Stall Problem

### What Happens

1. Sampler queries BEP 51 nodes, finds ~400 unique hashes from ~7,200 samples
2. After ~2 minutes, all BEP 51 nodes are in backoff (60s long backoff)
3. `pick_target()` returns None → loop sleeps 10ms → no queries sent
4. Effective throughput: ~2 QPS actual vs 1,600 QPS capacity (1000× below)

### Why It Happens

1. **IntervalMap cap (8,192)** tracks all nodes' backoff state
2. **NodeStats scoring:** Nodes with consecutive_stale >= 3 get 60s backoff
3. **No BEP 51 discovery:** Grower finds regular DHT nodes, not BEP 51 nodes
4. **No active search:** Sampler passively waits for grower to inject fresh nodes

### The Math

- Routing table: 100k nodes
- BEP 51 capable: ~400 nodes (0.4% of total)
- After sampling all 400: all in 60s backoff
- Grower adds regular DHT nodes (not BEP 51)
- Sampler has nothing productive to do

## Bottlenecks Identified

### Bottleneck 1: Sampler Supply (Phase 1)
- **Problem:** 400 BEP 51 nodes exhausted in 2 minutes
- **Impact:** unique_per_hr volatile (28k-129k), not sustained
- **Root cause:** No mechanism to discover NEW BEP 51 nodes

### Bottleneck 2: Fetch Conversion (Phase 2)
- **Problem:** 94% failure rate, sampled hashes never reach trackers/DHT
- **Impact:** 47/hr verified (need 10k/hr)
- **Root cause:** Fail-fast design, not a bug

### Bottleneck 3: OOM Kills
- **Problem:** jemalloc retains 5.7GB, exceeds 4GB Docker limit
- **Impact:** Process killed every 5-10 minutes, progress lost
- **Root cause:** jemalloc doesn't release pages aggressively enough

### Bottleneck 4: Failure Classification
- **Problem:** `record_peer_failure` not called at top-level error path
- **Impact:** All breakdown counters show 0 despite 1,204+ failures
- **Root cause:** Bug in error handling path

## Key Insight

**The routing table rewrite is working** (80k-120k nodes vs old 2,400 cap). But verified rate hasn't improved because:

1. **Supply bottleneck moved:** From routing table (solved) to BEP 51 node discovery (unsolved)
2. **Conversion bottleneck unchanged:** Sampled hashes still fail-fast at Phase B
3. **The system is working as designed** — but the design assumes abundant BEP 51 nodes

**To reach 10k/hr verified, we need either:**
- **Massive supply increase:** 1.4M unique/hr (at 0.65% conversion)
- **Conversion improvement:** Increase Phase B success rate or remove fail-fast
- **Both:** Most likely path forward

## Proposed Next Steps

### Step 1: Fix the Sampler Stall (Supply Bottleneck)

**Root cause:** The sampler exhausts all 400 BEP 51 nodes in 2 minutes, then all nodes are in 60s backoff. The grower adds regular DHT nodes, not BEP 51 nodes.

**Proposed fix:** Modify the sampler to actively discover new BEP 51 nodes instead of passively waiting for the grower.

**Option A: Reduce backoff further**
- `STALE_LONG_BACKOFF`: 60s → 10s (or even 0s for the first re-query)
- `STALE_GRADUATION`: 3 → 1 (graduate to long backoff after 1 empty)
- **Impact:** Nodes re-enter the pool faster, but may waste queries on dead nodes

**Option B: Add BEP 51 discovery to the sampler**
- When a BEP 51 node responds successfully, also process its `nodes` field (the response includes nearby nodes)
- These nodes are likely BEP 51 capable (same keyspace region)
- **Impact:** Grows the BEP 51 node pool organically

**Option C: Track BEP 51 capability in the routing table**
- Add a `bep51_capable: bool` flag to `RoutingNode`
- The sampler only queries nodes with this flag set
- The grower's `find_node` responses are marked as unknown (not BEP 51)
- **Impact:** Faster filtering, but requires routing table changes

**Recommendation:** Start with Option A (simplest), then add Option B if needed.

### Step 2: Fix the Fetch Conversion (Conversion Bottleneck)

**Root cause:** Sampled hashes use Phase B only (direct get_peers to reporting node) and fail-fast at line 512-517. Phase C (trackers + iterative DHT) is never reached.

**Proposed fix:** Allow sampled hashes to fall through to Phase C when Phase B fails.

**Option A: Remove the fail-fast entirely**
- Delete lines 512-517 (the `if lookup_seed.is_some()` block)
- Sampled hashes would go through Phase C (trackers + iterative DHT)
- **Impact:** Each failed hash costs ~6s (2s trackers + 4s DHT) instead of ~300ms
- **Trade-off:** Lower throughput but higher conversion

**Option B: Add a "second chance" for sampled hashes**
- After Phase B fails, check if the hash is "promising" (e.g., reported by multiple BEP 51 nodes)
- Only promising hashes go to Phase C
- **Impact:** Balances throughput and conversion

**Option C: Increase Phase B success rate**
- Increase `FETCH_TIMEOUT` from 3s to 5s (more time for slow peers)
- Increase `MAX_PEERS_PER_HASH` from 64 to 128 (more peers to try)
- **Impact:** Higher Phase B success rate, but slower per-hash

**Recommendation:** Start with Option A (simplest), measure impact, then optimize.

### Step 3: Fix the OOM Kills (Stability Bottleneck)

**Root cause:** jemalloc retains 5.7GB, exceeds 4GB Docker limit.

**Proposed fix:** Reduce memory usage or increase the Docker limit.

**Option A: Increase Docker memory limit**
- `mem_limit`: 4g → 8g
- **Impact:** Allows jemalloc to retain more pages without OOM

**Option B: Reduce memory usage**
- Decrease `--max-nodes` from 65536 to 32768 (halves routing table memory)
- Decrease `--sampler-loops` from 256 to 128 (halves sampler memory)
- **Impact:** Reduces memory usage, but may reduce throughput

**Option C: Configure jemalloc to release pages more aggressively**
- Set `MALLOC_CONF=background_thread:true,dirty_decay_ms:1000`
- **Impact:** jemalloc releases pages faster, reducing RSS

**Recommendation:** Start with Option A (simplest), then optimize with Option C if needed.

### Step 4: Fix the Failure Classification (Diagnostic Bottleneck)

**Root cause:** `record_peer_failure` not called at the top-level error path.

**Proposed fix:** Call `record_peer_failure` for all failure kinds at the top-level error path.

**Option A: Add `record_peer_failure` call at line 246-269**
- After `stats.fetches_failed.fetch_add(1, ...)`, call `record_peer_failure` with the dominant failure kind
- **Impact:** Enables diagnosis of fetch failures

**Option B: Add `record_peer_failure` call at line 512-517**
- After the fail-fast return, call `record_peer_failure(FetchFailureKind::EmptyPeers, stats)`
- **Impact:** Enables diagnosis of sampled hash failures

**Recommendation:** Implement both Option A and Option B.

### Step 5: Measure and Iterate

**After implementing the fixes:**
1. Rebuild and restart the crawler on a fresh DB
2. Monitor `routing_nodes`, `unique_per_hr`, `metadata_verified`, and failure counters
3. Verify that the sampler stall is resolved (sustained unique_per_hr)
4. Verify that the fetch conversion improves (lower failure rate)
5. Verify that OOM kills are eliminated
6. Verify that failure classification works (non-zero breakdown counters)

**Acceptance criteria:**
- `routing_nodes` > 50k/instance (already met)
- `unique_per_hr` > 300k (sustained, not volatile)
- `metadata_verified` > 10k/hr (target)
- Fetch failure rate < 50% (down from 94%)
- No OOM kills
- Failure breakdown counters non-zero

## Runtime Validation Results (2026-08-18)

### DEFINING MEASUREMENT: bitmagnet's actual output on THIS machine

Measured the real bitmagnet database (`bitmagnet-test-postgres`, port 35432) that run on this same host:

| Interval (2026-08-16) | torrents (cumulative) |
|---|---|
| 09:00 | cold start |
| 09:30 | 1,429 |
| 09:45 | 2,994 |
| 10:00 | 3,480 (hour 1 ≈ 3,480) |
| 10:15 | ~5,063 |
| 10:30 | ~6,655 |
| 10:45 | 8,101 (hour 2 total ≈ 4,621) |

Total: **8,101 real torrents (parsed files + names) in ~2 hours on this single host**; peak bursts ~1,550-1,600 / 15 min ≈ **6,000-6,400/hr**, tapering only as the fresh-node pool depleted.

**This falsifies the earlier "intrinsic ~0.1% sampled-hash conversion wall" theory.** On the identical machine, bitmagnet converts sampled hashes → persisted torrents at a far higher rate than our ~0.11%. The gap is architectural (engine), not a fixed network property. Our engine produces ample supply (60k+ unique/hr) yet verifies only ~65/hr — conversion is the fixable bottleneck.

### GAIA vs bitmagnet architecture gaps (the real cause)

1. **Unbounded live-node ktable.** Bitmagnet's routing table grows continuously from a many-node keyspace, never capped. Ours was 65,536/instance (now raised to 1M) and re-scans a bounded, increasingly-stale set.
2. **`lastRespondedAt >= now-5s` sampling gate.** Bitmagnet excludes any node that just answered (find_node/get_peers/sample) from sampling for 5s (`IsSampleInfoHashesCandidate`), spreading queries across freshly-live nodes whose hashes have active swarms. Our sampler has no last-responded gate, so it re-requests the same recent nodes.
3. **Channel pipeline + dedicated worker pools.** Bitmagnet runs ~60 sample / 100 find_node / 100 ping / 200 get_peers / 400 metainfo workers (×scale), each draining a buffered channel — no per-loop full-table rescan / thundering herd.
4. **`discoveredNodes` loop** (`FilterKnownAddrs` → find_node/sample/ping) continuously injects fresh nodes.

### GAIA shipped this session (bitmagnet-faithful pieces)

1. Shared rotating `soughtNodeID` (grower + sampler, 10s rotation).
2. Grower rewrite: OLDEST-nodes find_node with shared sought; drop-node-on-fail.
3. Candidate feeder queue (one refiller → shared queue, loops pop-one).
4. `max-nodes` 65,536 → 1,000,000/instance.
5. Empty-node backoff capped at 60s (6h starves our bounded table).
6. `RemoveNode` DHT command (drop-node-on-query-error).

### GAIA measured this session (crawler_bench5 / dhtbench5)

| Metric | Start of session | After changes |
|---|---|---|
| max routing nodes | 65,536/inst | 1,000,000/inst (growing past 350k) |
| hashes_sampled | ~16k / 20 min | 265k / 10 min (~880/sec) |
| hashes_unique | 1.3k | 14.4k (~61k/hr) |
| metadata_verified | 2 / 20 min | ~65/hr |
| conversion (unique→verified) | ~0.1% | ~0.11% |

**Supply is healthy; conversion (~0.11%) is the remaining gap vs bitmagnet's measured rate. Next: fresh-DB controlled measurement + last-responded sampling gate + dedicated worker pools.**

### Testing protocol (per user directive)

Always start crawler tests on a **fresh DB with clean data**: create a new benchmark DB/redis prefix, wipe routing state, and measure from a cold table so previous runs' 500k+ cached hashes don't bias `hashes_unique`/verified. Reuse the bitmagnet DB only as a reference of what THIS host achieves.

### FRESH-DB FAIL-FAST BREAKTHROUGH (crawler_bench8 / dhtbench8, 2026-08-19)

**Root cause found and fixed.** The earlier "conversion wall" was actually the FETCH POOL backing up: sampled dead hashes each burned ~7s on Phase C (tracker + DHT walk), so with ~5,000 in-flight the queue backlogged to **57,339** and live hashes starved behind dead ones (verified ~65/hr).

**Fix (bitmagnet-exact fail-fast, `fetch/mod.rs`):** sampled hashes whose direct get_peers found no peers (`!any_peers_seen && source == Sampled`) are dropped immediately — no tracker, no DHT walk. Announced/looked-up hashes (live by construction) still fall through to Phase C.

| Metric | Before fail-fast (bench7) | After (bench8) |
|---|---|---|
| queue_depth | 57,339 (backed up) | **1** (flowing) |
| fetch_in_flight | 5,048 (pinned) | ~2,900-4,300 (headroom) |
| metadata_verified (first ~8 min) | 5 | **47** |
| sustained verified/hr | ~65 | ~300-390 |
| hashes_unique | 77k @5min | 139k @11min |

Verified throughput **5-6x higher** and climbing. The pool now cycles through hashes in one RTT instead of ~7s per dead hash.

**Sustained rate (bench8, ~19 min / 64 torrents): ~200-390/hr**, tapering as the initial live-hash pool exhausts. Supply (161k unique) and pool headroom (2,500/5,000) are NOT the constraint; direct-peer yield is.

### Remaining bottleneck: direct-peer yield (~12%)

- `direct_peers_found = 17,073` vs `direct_peers_timeout = 118,489 + empty = 16,901` → reporting node produces peers only ~12% of direct attempts.
- `connect_timeout = 43,021` → most returned peers are dead; verified/(found) ≈ 0.37%.
- Net sampled→verified ≈ 0.02%. To reach ~7k/hr at this conversion needs ~30x more peer-yielding supply (millions of unique/hr), or a step-change in direct-peer yield.

**Next levers (in order):**
1. **Raise direct-peer yield** — the reporting node only answers/get_peers with peers ~12% of the time. Investigate whether sampling THEN immediately get_peers the same fresh node (within the freshness window) raises this; bitmagnet converts because its samples come from live nodes with current swarms.
2. **Peer dial success** — raise MAX_PEERS_PER_HASH/parallelism; only ~0.37% of peer-found hashes convert, so dialing more peers per hash (while pool has headroom) adds verified.
3. **More supply** — verified ∝ supply; supply still has headroom (pool only ~50% loaded).

### FRESH-DB BASELINE (crawler_bench7 / dhtbench7, 2026-08-19)

Fresh database, clean redis prefix, current engine (shared sought + 1M nodes + feeder + 60s empty backoff + own-max-nodes):

| Metric | 5 min warm | ≈ /hr |
|---|---|---|
| routing_nodes | 228k → 350k+ | growing |
| hashes_sampled | 277k | ~3.3M/hr |
| hashes_unique | 37.9k | ~455k/hr |
| metadata_verified | 21 | ~250/hr (burst; tapers) |
| conversion (unique→verified) | 21/37.9k | ~0.055% |

**Clean-data result confirms supply is strong but conversion (~0.05%) is the wall — not supply.** Even on a cold table with a fresh bloom, verified lags 100x behind bitmagnet's ~6,000/hr on this same host.

### Fresh-DB failure breakdown (the conversion wall, quantified)

From `peer failure breakdown` on the clean run:
- `empty_peers` = 44,446 (68% of fetches): direct get_peers to the SAME reporting node that sampled the hash returned NO peer `values`.
- `direct_peers_timeout` = 22,932 (~47% of direct attempts): the 1-shot UDP get_peers to the reporting node never got an answer within 3s.
- `direct_peers_found` = 2,261 → dialed; but `connect_timeout` = 11,571 drowned the rest, leaving ~21 verified.

These two numbers (`empty_peers` + `direct_peers_timeout` ≈ 90%+ of direct attempts) are the mechanism behind the conversion gap. Bitmagnet gets live peer `values` from its sampled nodes far more often because it samples from a **continuously-refreshed, unbounded pool of LIVE nodes**.

### Definitive plan for the conversion gap (next session)

1. **Fresh-DB controlled runs only** (already set up as crawler_bench6/dhtbench6; bitmagnet DB untouched as reference).
2. **`lastRespondedAt` sampling gate** (bitmagnet `IsSampleInfoHashesCandidate`): exclude a node from sampling for 5s after it answers ANY query, so we sample freshly-live nodes whose hashes have active swarms — targets the `empty_peers`/`direct_peers_timeout` wall.
3. **Direct get_peers resilience**: raise the direct timeout behavior; on a timeout/empty from the reporting node, fall back to a fast seeded DHT get_peers toward the hash (bitmagnet does a full iterative walk when the dirGetPeers path is insufficient via its huge ktable). Prefer fewer, higher-yield direct attempts over 47%-timeout spam.
4. **Worker-pool fetch** (bitmagnet channel model): confirm the fetch pool drains at the sampler's unique rate; today the pool idles at ~84 fetches/sec vs 5000 capacity because `queue_depth≈0` — the wall is upstream peer-resolution, not metainfo dialing.
5. **Re-measure on fresh DB** after each change; compare verified/hr against the 8,101-torrent bitmagnet reference table.
