## Why

The initial `dht-crawler` build (previous change) worked end-to-end but captured torrents far too slowly to be useful, and its storage schema and module layout did not scale. Live measurement on the NAT host showed the real bottlenecks were **not** the network (bitmagnet proves this machine reaches peers fine) but four crawl-engine defects:

1. **The fetch pool was capped at 64 concurrent, not 512.** `fetch_one` held the lookup semaphore permit for the *entire* call (get_peers + up to 45s of peer dialing), so effective concurrency was `lookup_concurrency` (64), not `concurrency` (512). The DhtLookup runs in the actor background, so the permit is only needed to start the get_peers stream. Only ~30% of discovered hashes were ever fetched (3561 unique vs 1081 attempted).
2. **The movie/TV filter discarded most verified torrents.** 8 of 9 verified hashes were dropped as "not movie/TV", so verified-but-filtered torrents wasted fetch slots.
3. **Discovery was throttled to nothing by three compounding bugs** (a) nodes advertising 6-hour BEP 51 intervals froze the sampler, (b) `pick_target` picked the single closest node so a few cooling nodes starved all loops, and (c) `rand::choose_multiple` returns items in *original order* when `k >= len`, so with a small routing table all sampler loops converged on the same node and hammered it.
4. **The `torrents` table was shaped like a movie/TV app** (category, title, year, season, episode), coupling a generic torrent index to a specific media taxonomy.

This change fixes all four, re-architects the codebase for separation of concerns, rethinks the storage schema as torrent-metadata-only, and adds the measurement (passively-announced infohash count) needed to decide whether a second discovery source is ever worth building.

## What Changes

- **Fetch pipeline unblocked**: lookup permit released immediately after `get_peers()` returns the stream, before dialing. Effective concurrency becomes `concurrency` (default 512).
- **Keep everything**: the movie/TV filter is removed. Every SHA-1-verified torrent is persisted; classification only labels records (`movie`/`tv`/`other`) and enriches title/year/season/episode when it can. Nothing is discarded.
- **Discovery fixed and hardened**:
  - BEP 51 re-query interval capped (`--sampler-max-interval`, default 60s) so nodes advertising hours-long intervals are re-queried regularly.
  - `pick_target` picks a random ready node and targets its own node ID, then shuffles before sampling so all loops spread across the table.
  - Higher defaults: `--sampler-loops 32`, `--sampler-qps 2000`, `--qps 5000`, `--min-seen 2`.
  - Tighter fetch budget: `FETCH_DEADLINE 45s→20s`, `MAX_PEERS_PER_HASH 100→50`.
- **Announcement measurement**: the stats loop logs `announced_hashes` (`handle.stats().peer_store_info_hashes`), the count of infohashes other nodes announced to us. No crate patch. If this stays near zero, passive announce capture is permanently out of scope; if it grows large, a future change may add a `peer_store_hashes()` reader.
- **Modular codebase**: `main.rs` god-file split into focused modules with one-way dependencies (crawler orchestration, discovery, fetch, classify, storage, query, purge).
- **Schema redesign**: `torrents` stores torrent metadata only (`info_hash`, `name`, `size_bytes`, `file_count`, `first_seen`, `last_seen`). Movie/TV fields removed; a future `torrent_details` table can hold extra info if ever requested. Raw `info_bytes` already persists in `scanned` for offline re-analysis.

## Capabilities

### New Capabilities

- `discovery`: BEP 51 keyspace traversal with interval capping, node-quality targeting, spread across the routing table, and passive-announcement observability (`announced_hashes`).
- `fetch`: SHA-1-verified metadata acquisition with a concurrency-unblocked pool and tight per-hash budgets.
- `storage`: torrent-metadata-only `torrents` schema (no media taxonomy) with WAL, batched upserts, and name search; raw info dictionaries kept in `scanned`.
- `architecture`: modular module layout with one-way dependencies and separated CLI/query/purge concerns.

### Modified Capabilities

- `dht-crawler` (previous change): retains bootstrap, persistent routing, BEP 51 sampling, and unique-hash emission; gains interval capping and multi-source discovery plumbing.
- `metadata-enrichment` (previous change): retains BEP 9/10 fetch + SHA-1 verify; classification no longer filters, it labels.
- `media-filter` (previous change): renamed to the classify concern; no longer gates persistence.

## Impact

- **Code**: `main.rs` shrinks to a dispatcher; new `crawler.rs`, `discovery/`, `fetch/`, `classify.rs`, `query.rs`, `purge.rs`; `storage.rs` splits into `storage/{mod,schema,model}.rs`.
- **Schema**: `torrents` table rebuild migration drops `category`/`title`/`year`/`season`/`episode`; existing rows' torrent metadata preserved, classification re-derivable from `scanned.info_bytes`.
- **Dependencies**: none added. `irontide-dht` stays a plain crates.io dependency — the decision is **not** to vendor/patch it for announcement capture unless the `announced_hashes` counter proves the store grows large.
- **Operations**: crawler is significantly more aggressive (more sampling QPS, more fetch concurrency); on shared/limited connections, reduce `--sampler-qps`/`--qps`/`--concurrency` accordingly. The `--aggressive` preset already scales these up for VPS use.
- **Performance**: measured on the NAT host — torrents persisted jumped from ~1 per 30 min to ~9–15 in ~5–8 min; fetch attempts from ~36/min to ~200/min; sampling from ~600/min to ~3000+/min; routing table grows 0→200+ nodes.
