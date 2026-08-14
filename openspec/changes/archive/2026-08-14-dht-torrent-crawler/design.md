## Context

Greenfield Rust project, empty directory. See proposal.md — Why for the failure history and the two corrections to the source write-up (mainline lacks BEP 51; BEP 51 returns raw infohashes, not names). The pipeline is a tokio-based daemon: BEP 51 sampler (UDP) → BEP 9 metadata fetcher (TCP) → deterministic name filter → SQLite storage, exposed via a `run`/`query` CLI. Requirements live in the five delta specs under `specs/`.

## Goals / Non-Goals

**Goals:**
- Correct BEP 51 implementation (no more blind `find_node` walking or passive `get_peers` waiting).
- Warm-start crawling via a persisted routing table.
- Name acquisition exclusively from verified BEP 9 metadata fetches.
- Light crawl footprint: UDP only inside the crawler core; TCP only for metadata fetches.
- Monotonic, deduplicated SQLite dataset that survives restarts.

**Non-Goals:**
- No full BitTorrent client / peer (leech/seed) engine.
- No seeder/peer counting via `get_peers` scrape in this change (a later change can add it).
- No TMDB/IMDB title resolution; filtering stays name-pattern based.
- No web search API or FTS in this change (plain `LIKE` search via the `query` CLI).

## Decisions

### D1 — DHT library: `irontide-dht`
Use `irontide-dht` (v1.x, GPL-3.0, tokio) as the Kademlia actor. It provides `DhtHandle::start`, actor-based routing, **native BEP 51** (`sample_infohashes -> SampleInfohashesResult` with samples + interval + closer nodes), built-in routing-table persistence to `dht_state.json`, `get_peers`, `get_routing_nodes`, and stats.
- *Alternatives considered:* `mainline` (v8) — rejected, no BEP 51 (only BEP 5/42/43/44). Hand-rolled RAW UDP KRPC — rejected: duplicates a correct Kademlia implementation and bucket hygiene for little gain. `0xddy/dht-crawler` — a monolithic full crawler app rather than building blocks; less control, harder to make fit our exact pipelined specs.
- *Trade-off:* GPL-3.0 propagation to the binary; acceptable for a local/personal project, must be revisited for distribution.

### D2 — BEP 9 metadata fetcher: minimal codec, no peer engine
Implement `ut_metadata` (BEP 10 + BEP 9) as a focused module over tokio `TcpStream`: BitTorrent handshake → extension handshake (learn `ut_metadata` id + `metadata_size`) → piece-wise `ut_metadata` requests → assemble → SHA-1 over the bencoded `info` dictionary. Reuse `irontide-wire`'s bencode primitives from the same suite for message serialization.
- *Alternatives considered:* depend on a full client API (e.g. `librqbit`/`aquatic` helpers) — rejected, overkill for a fetch-only path; hand-writing bencode — rejected, an unnecessary 200-line risk when `irontide-bencode` already exists.
- *Efficiency:* most peer connections fail (dead peers, no `ut_metadata`, timeouts), so the pool is sized for many cheap failures with tight per-connect and per-piece timeouts.

### D3 — Keyspace traversal and interval bookkeeping
The sampler loop issues `sample_infohashes(target)` against rotating random 20-byte targets. A per-node map records the last query time + returned `interval`; a node is re-queryable only after `interval` elapses. Response `nodes` are fed back into the routing table to discover new BEP 51-capable nodes. Guard rails: a global query-per-second budget and a max in-flight sampler count.
- *Rationale:* BEP 51 says `interval` ∈ [0, 21600] and re-query before it yields nothing new; honoring it is both etiquette and efficiency.

### D4 — Storage: `rusqlite` + WAL + batched upserts
`rusqlite` with a small connection pool. Schema:
`torrents(info_hash BLOB PRIMARY KEY, name TEXT, category TEXT CHECK(category IN ('movie','tv')), title TEXT, year INTEGER NULL, season INTEGER NULL, episode INTEGER NULL, size_bytes INTEGER NULL, file_count INTEGER NULL, first_seen INTEGER, last_seen INTEGER)`.
WAL mode enables concurrent reads while the daemon writes. Commits are batched via `INSERT … ON CONFLICT(info_hash) DO UPDATE SET last_seen=excluded.last_seen, …` (never touching `first_seen`). Membership checks (`SELECT 1 WHERE info_hash=?`) feed pipeline dedup.
- *Alternatives considered:* sqlx/sqlite async — heavier; a KV like sled — unnecessary, we want relational query + `LIKE`.

### D5 — Pipeline concurrency model
- **Sampler**: low concurrency, interval-respecting, feeds a bounded `mpsc` channel.
- **Metadata pool**: `Semaphore`-bounded tasks (default 512), tight timeouts, per-hash peer iteration; enforces the "bounded concurrency" + "skip seen" requirements.
- **Filter**: pure function, no I/O.
- **Storage writer**: single-threaded consumer batching in WAL transactions.
Backpressure propagates naturally: bounded channels + the fetch semaphore; overflow is logged, not fatal.

### D6 — Termination / state hygiene
On SIGTERM/SIGINT: stop the sampler, let in-flight fetches finish or cancel, drain the writer, call `save_routing_table` (or the actor's shutdown persistence), and exit 0. State lives in a configurable `--state-dir`.

## Risks / Trade-offs

- **NAT/firewall drops replies** → Run on a VPS with a public IP/port; document it; the actor's client mode degrades gracefully to bootstrapping from bootstrap nodes even behind NAT.
- **GPL-3.0 propagation from `irontide-dht`** → Fine locally; flag before distribution. Mitigation is a licensing decision, not a code change.
- **Metadata fetch failure rate is high** → Large semaphore + short timeouts so throughput stays high despite many failures; skip-seen prevents wasted re-fetch churn.
- **BEP 51 `interval` bookkeeping grows with node count** → Cap the node map (LRU-style eviction) and rely on the actor's routing-table hygiene.
- **Sponsor concern: bencode edge cases (`.pad` files, BEP 47, v2/hybrid)** → Enrichment tolerantly parses v1 metadata and forward-skips unknown fields; hybrid/v2 handled as best-effort rather than a hard failure.

## Migration Plan

Greenfield — no existing code to migrate. Initial run: `cargo build --release && ./target/release/dht-crawler run --db crawler.sqlite --state-dir ./state`. Rollback: delete the crate; no external systems touched.

## Open Questions

- Confirm accepting GPL-3.0 for the project (design D1 stands regardless).
- Whether to reuse `irontide-wire` for the BEP 9/10 codec or hand-roll the ~200-line extension handshake; both satisfy the specs — decide during task 3.