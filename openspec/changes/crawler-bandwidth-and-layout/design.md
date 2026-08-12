## Context

Building on `dht-crawler-scale` (committed 2d0e19f). The scale change grew discovery to 250k unique/hr and verification to ~800-1,000/hr, but bandwidth rose to 1.57 MB/s with 0.27% fetch success. Review found two pure-waste leaks: (1) K=80 inflated inbound-response payloads 10x in five `actor.rs` paths, and (2) the fetch pipeline dials up to 16 dead peers per hash at scale-10 concurrency. Separately, the layout needs `dht-crawler/` → `crawler/` and `vendor/` → internal `crawler/crates/`. Adds decisions D49-D53.

## Goals / Non-Goals

**Goals:**
- Remove the 10x UDP response inflation without reducing routing-table capacity or discovery.
- Cut dead-peer TCP churn ~4x while keeping live-hash verification unchanged.
- Restructure the workspace to `crawler/` with internal `crawler/crates/gaia-*` members.
- Land at roughly the same verified/hr at ~1/3 the bandwidth.

**Non-Goals:**
- No change to `--scale` (stays 10), instance count (4), Docker/Gluetun/Redis architecture.
- No content filtering.
- No change to passive announce intake or the announce-first hint path.

## Decisions

### D49 — RESPONSE_K: decouple response payloads from table capacity
Introduce `RESPONSE_K` (16) used only when constructing inbound-query responses; the routing table keeps `K=80`.
- *Rationale:* table capacity (what drove discovery) and response payload size (pure bandwidth) are independent concerns that the scale change conflated. BEP 5 responses return the closest ~8 nodes; 16 is generous and keeps responses in one UDP packet (~26B × 16 = 416B + header).
- *Trade-off:* inbound queries get slightly fewer closer-nodes than the table could provide; irrelevant for our crawl (we answer queries as a node, not as a consumer of our own responses).

### D50 — Tighten fetch dial budgets
`PARALLEL_DIALS` 16→4, `MAX_PEERS_PER_HASH` 50→16, `FETCH_TIMEOUT` 5s→3s, `EARLY_ABORT_DIALS` 64→24.
- *Rationale:* 99.1% of fetches fail; most end in `connect_timeout` dialing peers that exist in get_peers but never respond. The first live peer wins, so 4 parallel dials finds a live hash as reliably as 16 while cutting dead-peer churn ~4x. 3s is ample for a handshake + ut_metadata exchange on a live peer (bitmagnet uses 6s total request timeout but dials one peer at a time).
- *Trade-off:* a hash whose only live peer is slow might miss the 3s window; retried via backoff.

### D51 — Keep `--scale` 10
Concurrency stays at scale=10; the bandwidth win comes from removing per-request waste, not reducing concurrency.
- *Rationale:* scale=10 is what lets the pipeline keep up with 250k unique/hr; the waste is in response size and dial budget, not the number of concurrent tasks.

### D52 — Workspace layout: `crawler/` + `crawler/crates/gaia-*`
Rename the app dir to `crawler/` and move the owned crates to `crawler/crates/gaia-*` as internal workspace members.
- *Rationale:* conventional Rust workspace layout for an app with internal library crates; removes the misleading root-level `vendor/` (these are now ours, not third-party vendored code).
- *Trade-off:* a broad rename touching Dockerfile, compose, scripts, docs; verified by full build/test/deploy.

### D53 — Stable data volume across the rename
Keep the compose volume name `dht-crawler-data` (DB + state) unchanged during the rename so crawl data and node identity persist.
- *Rationale:* the volume is the crawl's only durable state (SQLite + routing/node-id state); renaming it would orphan ~1,000 indexed torrents and the accumulated DHT reputation.
- *Trade-off:* the volume name retains the old `dht-crawler` prefix; cosmetic.

## Risks / Trade-offs

- **Lower dial parallelism**: live hashes with only slow peers may take a retry; backoff covers it.
- **RESPONSE_K 16 vs 8**: slightly larger than bare BEP5; keeps one-packet responses and ample closer-nodes for in-network routing. Can drop to 8 if bandwidth still high.
- **Broad rename**: touched in one commit; every reference updated and verified by test/build/Docker/deploy.
- **Bandwidth is bursty**: short windows are noisy; benchmark over ≥15 min.
