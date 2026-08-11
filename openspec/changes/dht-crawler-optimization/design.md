## Context

Building on the completed `dht-torrent-crawler` change. The greenfield crawler now runs end-to-end but underperforms for the reasons in proposal.md: a concurrency cap hidden by the lookup permit, a discarding media filter, three compounding discovery bugs, and a movie/TV-shaped schema. The prior design (D1–D6) remains the foundation; this change adds decisions D7–D12 and updates D3/D4/D5.

## Goals / Non-Goals

**Goals:**
- Raise verified-torrent throughput by unblocking fetch concurrency and removing discards.
- Fix the three discovery defects so the sampler keeps querying and spreads across nodes.
- Re-architect modules so dependencies flow one way and concerns are separable.
- Store torrent metadata only; keep classification as optional enrichment, re-derivable from `scanned.info_bytes`.
- Measure passive-announce intake (`announced_hashes`) to decide, with data, whether a second discovery source is worth the cost.

**Non-Goals:**
- No public tracker / index scraping (deferred indefinitely).
- No vendoring or patching `irontide-dht` unless `announced_hashes` proves the peer store grows large (decision D8, revisit-with-data).
- No full BitTorrent client/peer engine; no TMDB/IMDB resolution.
- No `torrent_details` table yet — schema is prepared so it can be added later without a torrents-table rebuild.

## Decisions

### D7 — Fetch concurrency is unblocked by releasing the lookup permit early
`fetch_one` acquires the lookup semaphore only to start `get_peers()`, then drops it before dialing. The actor's DhtLookup keeps running in the background and feeds the peer channel, so the pool's effective concurrency becomes `concurrency` (default 512), not `lookup_concurrency` (default 64).
- *Alternatives considered:* raising `lookup_concurrency` to match `concurrency` (512 lookups in flight — wasteful; the permit's purpose is bounding lookups); splitting permits per phase — unnecessary once the early release works.
- *Trade-off:* `lookup_concurrency` now bounds only how many lookups may be *started* concurrently, which is the intended meaning.

### D8 — Do NOT vendor/patch irontide; poll harder, and measure announcements
Capturing passively-announced hashes would require reading irontide's internal `peer_store`, which has no public API — adding one means vendoring/patching the crate. Analysis shows the patch is small and purely additive (one `peer_store_hashes()` accessor + `DhtCommand` variant), but the *yield* is uncertain: `announce_peer` is routed to the K closest nodes to the hash, so a random-ID NAT node receives few announces. Decision: **no patch now**. Instead:
- Push BEP 51 polling harder (D9).
- Log `announced_hashes` from the existing `handle.stats()` (`peer_store_info_hashes`) in the stats loop — zero code in irontide.
- Revisit the patch only if `announced_hashes` grows into the thousands, at which point the vendoring cost is justified. This resolves the "what if a new irontide release comes" concern entirely — we never fork, so upgrades stay a dependency bump.
- *Alternatives considered:* `start_unified` socket tap (own the UDP socket, re-demux KRPC) — rejected: bigger, riskier, a forwarding bug could break the DHT we rely on. Git fork + `[patch.crates-io]` — rejected for the same maintenance burden as vendoring with no present benefit.

### D9 — Discovery defaults pushed higher; interval capped; nodes spread
- Cap the effective BEP 51 re-query interval at `--sampler-max-interval` (default 60s) so nodes advertising 6h intervals are re-queried regularly (previously the sampler froze after a couple of samples).
- `pick_target` selects a random *ready* node and uses the node's own ID as the sample target (the actor resolves `closest(target,1)` to exactly that node), then shuffles the ready set before sampling (fixes `choose_multiple` returning original order when `k>=len`, which made every loop hit the same node).
- New defaults: `--sampler-loops 32`, `--sampler-qps 2000`, `--qps 5000`, `--min-seen 2`. Aggressive preset: loops 64, sampler-qps 4000, qps 10000, concurrency 1024, lookup-concurrency 256, max-nodes 4096.
- *Rationale:* these are the measured constraints — sampling was the binding discovery rate, and single-sighting hashes (min-seen 1) dominate the fetch-failure long tail.

### D10 — Fetch budgets tuned for "fail fast, free slots"
`FETCH_DEADLINE 45s→20s` and `MAX_PEERS_PER_HASH 100→50`. Successful fetches almost always complete in the first few dials, so dead hashes free their pool slot quickly.
- *Trade-off:* rare torrents with many slow peers may exceed 20s; acceptable because those are precisely the low-yield cases.

### D11 — Classification labels, never filters; schema is torrent-metadata-only
Every SHA-1-verified torrent is persisted. `MediaFilter` (now the classify concern) labels `movie`/`tv`/`other` and enriches title/year/season/episode when it can; when it cannot, the record is stored as `other`. The `torrents` table drops `category`, `title`, `year`, `season`, `episode`:
`torrents(info_hash BLOB PK, name TEXT NOT NULL, size_bytes INTEGER, file_count INTEGER, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL)`.
A future `torrent_details(info_hash FK, category, title, year, season, episode, …)` can be added without touching `torrents`. Raw `info_bytes` already lives in `scanned` for re-analysis.
- *Alternatives considered:* keeping category/title/etc. in `torrents` — rejected: couples a generic index to a media taxonomy and duplicates what `scanned.info_bytes` already preserves.

### D12 — Modular layout with one-way dependencies
Split the `main.rs` god-file and the fat `storage.rs`/`metadata/mod.rs` into focused modules:
`main` (dispatch) → `cli` (args) / `crawler` (pipeline+shutdown) / `query` / `purge`; `discovery/{mod,sampler,announce}` → hash stream; `fetch/{mod,wire,parse}` → records; `classify` → labels; `storage/{mod,schema,model}` → persistence; `net`, `stats`. Dependencies flow only forward (crawler → discovery/fetch/storage; fetch → classify/net/storage); no cycles.
- *Rationale:* the previous flat layout mixed CLI, pipeline wiring, the storage writer, stats, query, and purge in one file; separation makes each concern testable and maintainable.

## Risks / Trade-offs

- **More aggressive sampling/fetching** → On bandwidth- or rate-limited links this may look noisy; mitigate by lowering `--sampler-qps`, `--qps`, `--concurrency`, or use `--aggressive` only on a VPS.
- **`announced_hashes` stays near 0** → Confirms the no-patch decision permanently; discovery remains BEP 51 sampling, which is sufficient.
- **`announced_hashes` grows large** → Vendoring/patching irontide becomes worthwhile; the patch is additive and documented, and the upgrade path (dependency bump or re-apply patch) is straightforward.
- **Schema migration loses classification columns** → Acceptable: torrent metadata is preserved and classification is re-derivable from `scanned.info_bytes`.

## Migration Plan

Existing databases: `configure()` rebuilds the `torrents` table (SQLite cannot alter a CHECK), copying `info_hash`, `name`, `size_bytes`, `file_count`, `first_seen`, `last_seen` and dropping the five media columns. The `scanned` table is unchanged and still holds `info_bytes` for re-classification. Rollback: restore the previous binary + DB backup; no external systems are touched.
