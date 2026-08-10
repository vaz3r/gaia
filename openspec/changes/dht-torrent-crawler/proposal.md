## Why

Two prior attempts at a DHT torrent indexer captured only a handful of torrents after hours of running. Both failed for the same structural reasons: BEP 51 (`sample_infohashes`) was never implemented, so infohash discovery relied on blind `find_node` walks and trickle-in `get_peers`; and the routing table was not persisted, so every restart restarted from zero. A published write-up asserting "mainline is BEP 51 ready" is also out of date (the crate dropped BEP 51), and its claim that BEP 51 responses carry a human-readable `name` is wrong — BEP 51 returns only raw 20-byte infohashes. Names can only be obtained by fetching torrent metadata over TCP (BEP 9). This change builds a correct, lightweight, efficient crawler that indexes movie/TV torrents into a local SQLite database.

## What Changes

- New Rust binary crate `dht-crawler` (tokio-based 24/7 daemon).
- **DHT crawler core**: bootstrap from known nodes, maintain a large persistent routing table (saved to disk, reloaded on start), and traverse the keyspace via BEP 51 `sample_infohashes` while honoring each node's `interval` backoff.
- **Metadata enrichment**: BEP 9 `ut_metadata` fetches over TCP for sampled infohashes; assembled metadata is SHA-1 verified against the infohash, then parsed for `name`, file list, and total size. This is a core pipeline stage, not optional enrichment — names are unobtainable otherwise.
- **Media filter**: classify release names as `movie` / `tv` / skip, requiring a year + quality tag for movies and a clear season/episode pattern for TV; extract year, season, episode where present.
- **Storage**: SQLite database (WAL) with an `info_hash`-keyed schema, upsert semantics that preserve `first_seen` and bump `last_seen`, and name search.
- **CLI**: `run` (crawl daemon) and `query "<name>"` subcommands with structured tracing logs.

## Capabilities

### New Capabilities

- `dht-crawler`: bootstraps and maintains a persistent Kademlia routing table, traverses the keyspace with BEP 51 `sample_infohashes`, and emits unique infohashes to the pipeline.
- `metadata-enrichment`: fetches torrent metadata via BEP 9 for sampled infohashes, verifies metadata integrity, and extracts name/files/size.
- `media-filter`: classifies release names as movie/TV and extracts title metadata (year, season, episode).
- `storage`: persists torrent records in SQLite with upsert semantics and supports name search.
- `cli`: exposes the crawler and its search as unix-style subcommands with configurable runtime options and structured logging.

### Modified Capabilities

_None (greenfield project)._

## Impact

- **Code**: new `Cargo.toml`, `src/main.rs`, and modules for dht (`node.rs`/`sampler`), metadata fetching, filtering, and storage.
- **Dependencies**: `tokio`, `irontide-dht` (BEP 51 actor), `rusqlite`, `clap`, `tracing`/`tracing-subscriber`, `bytes`, `serde`. BEP 9/10 protocol encoding may reuse `irontide-wire` primitives or a minimal hand-rolled codec.
- **Licensing**: `irontide-dht` is GPL-3.0-or-later; this propagates to the `dht-crawler` binary. Acceptable for a personal/local project; revisit before any public distribution.
- **Operations**: daemon is UDP-light (no TCP for crawling) but requires outbound TCP to peer ports for metadata fetches; best run on a VPS with a public IP to avoid NAT reply-drop.