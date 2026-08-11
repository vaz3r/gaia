# Tasks

## 1. Scaffold

- [x] 1.1 Initialize Cargo crate `dht-crawler` (edition 2021), binary target, MIT-or-GPL license per D1 decision
- [x] 1.2 Add dependencies: `tokio`, `irontide-dht`, `rusqlite`, `clap`, `tracing`, `tracing-subscriber`, `bytes`, `serde`/`serde_bytes`
- [x] 1.3 Create module skeleton: `main`, `dht/`, `metadata/`, `filter/`, `storage/`, `config`
- [x] 1.4 Add clap CLI with `run` and `query` subcommands and their flags (port, db, concurrency, ipv6, state-dir, RUST_LOG override)
- [x] 1.5 Add `tracing_subscriber` init honoring RUST_LOG; `cargo build` passes

## 2. DHT crawler core (dht-crawler capability)

- [x] 2.1 Start `DhtHandle` actor via `DhtConfig` bound to configured UDP port with optional IPv6
- [x] 2.2 Configure bootstrap nodes (router.bittorrent.com:6881, dht.transmissionbt.com:6881, dht.libtorrent.org:25401) and confirm bootstrap completes or times out with a warning
- [x] 2.3 Verify routing-table persistence: `save_routing_table()`/shutdown persistence writes `dht_state.json`; present/absent/corrupt state loads safely
- [x] 2.4 Implement sampler loop: random 20-byte targets → `sample_infohashes(target)` using `DhtHandle`
- [x] 2.5 Implement per-node `interval` backoff map (cap size, LRU eviction) and global QPS budget
- [x] 2.6 Feed returned `nodes` back into routing; expose routing-table size via `node_count()`/`get_routing_nodes()`
- [x] 2.7 Emit unique infohashes into bounded `mpsc` channel to the metadata stage; dedup against DB membership (storage stub)
- [x] 2.8 Unit tests: dedup at sampler level, interval honoring, bootstrap-failure warning path

## 3. Metadata enrichment (metadata-enrichment capability)

- [x] 3.1 Implement BitTorrent handshake + BEP 10 extension handshake over tokio `TcpStream` (irontide-wire primitives or minimal codec)
- [x] 3.2 Implement BEP 9 `ut_metadata` piece requests and assembly of the bencoded info dictionary
- [x] 3.3 Compute SHA-1 over assembled info; accept only on match; reject mismatches with no partial persist
- [x] 3.4 Parse verified metadata → `name`, file list/sizes, total size; tolerate missing/unknown fields
- [x] 3.5 `get_peers` per infohash via `DhtHandle`; iterate peers with tight connect/piece timeouts until success or exhaustion
- [x] 3.6 Build bounded worker pool with semaphore (default 512) controlled by `--concurrency`
- [x] 3.7 Skip previously-seen (DB membership) and recently-failed hashes (retry window map)
- [x] 3.8 Log fetch success/failure ratios at INFO/DEBUG
- [x] 3.9 Unit test: SHA-1 verification pass/fail on crafted info dictionaries

## 4. Media filter (media-filter capability)

- [x] 4.1 Implement deterministic name normalization (punctuation→space, collapse, lowercase)
- [x] 4.2 Implement movie classification (year + quality/container tag) and tv classification (SxxExx / Season N Episode N / SxNN)
- [x] 4.3 Implement title cleaning (strip tags) and year/season/episode extraction
- [x] 4.4 Wire filter between metadata extraction and storage; ensure skip categories never persist
- [x] 4.5 Unit tests: movie pass, movie-missing-quality reject, tv marker, software/adult skip, deterministic rerun equality

## 5. Storage (storage capability)

- [x] 5.1 Create `torrents` table with `info_hash BLOB PRIMARY KEY`, category CHECK, nullable year/season/episode, size, first/last_seen
- [x] 5.2 Open in WAL mode; batched upsert insert via `ON CONFLICT(info_hash) DO UPDATE` preserving `first_seen`
- [x] 5.3 Implement membership check `SELECT 1 WHERE info_hash=?` for pipeline dedup
- [x] 5.4 Implement case-insensitive `LIKE` name search used by `query`
- [x] 5.5 Reject records with category not in (movie, tv)
- [x] 5.6 Unit tests: upsert preserves first_seen, invalid category rejected, search match/no-match

## 6. Integration and polish

- [x] 6.1 Wire end-to-end pipeline: sampler → fetch pool → filter → storage writer (single-threaded batching)
- [x] 6.2 Graceful shutdown: drain in-flight, persist routing table + db batches on SIGTERM/SIGINT
- [x] 6.3 `query` command prints name/category/year/size from the live database
- [x] 6.4 Periodic stats logging: routing-table size, hashes sampled, fetch success, records persisted
- [x] 6.5 README: quickstart (build, run, query), prerequisites (public IP/port), option reference, GPL note
- [x] 6.6 Run `cargo test` and `cargo clippy -- -D warnings`; fix all findings