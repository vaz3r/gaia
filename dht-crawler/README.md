# dht-crawler

A lightweight BitTorrent DHT crawler that indexes torrents into a local SQLite
database. It bootstraps and maintains a persistent Kademlia routing table,
traverses the DHT keyspace with **BEP 51** (`sample_infohashes`), fetches torrent
metadata over TCP with **BEP 9/10** (`ut_metadata`), verifies each download by
SHA-1, and stores every accepted torrent with upsert semantics.

## Quickstart

```sh
cargo build --release

# Crawl (daemon)
./target/release/dht-crawler run \
  --db crawler.sqlite \
  --state-dir state \
  --port 6881

# Search the index
./target/release/dht-crawler query "matrix 1080p"
```

## Prerequisites

- A machine with a **public IP and reachable UDP port** (the DHT node replies
  from that port). Behind NAT/firewall, reply packets are often dropped and
  crawling stalls; a VPS is recommended.
- Outbound TCP to arbitrary peer ports is required for metadata fetches.
- Rust (stable) and a C toolchain for `rusqlite`'s bundled SQLite.

## How it works

```
sampler (BEP 51, UDP)
   │  unique infohashes
   ▼
metadata fetcher (BEP 9, TCP, SHA-1 verified)
   │  name / files / size
   ▼
SQLite (WAL, batched upserts)
```

- **Sampler**: several concurrent loops issue `sample_infohashes` against
  random keyspace targets, honoring each node's returned `interval` before
  re-querying it and biasing toward nodes that have historically returned
  samples. Each infohash is emitted with a *popularity* count (how many
  distinct nodes reported it); hashes below `--min-seen` are never fetched.
- **Metadata**: fetches are processed most-popular-first through a priority
  queue. For each infohash a DHT `get_peers` lookup finds swarm peers, up to 16
  peers are dialed concurrently, and the first peer whose `ut_metadata` pieces
  reassemble to a SHA-1 match wins. Every attempt is recorded in the `scanned`
  table (`ok`/`failed`) with the raw `info` dictionary, so restarts never
  refetch known content; failed hashes retry with exponential backoff
  (5m → 10m → ... → 6h).
- **Storage**: `torrents` table keyed by `info_hash` stores torrent metadata
  only (`name`, `size_bytes`, `file_count`, `first_seen`, `last_seen`) — no
  media taxonomy. WAL mode, batched `ON CONFLICT DO UPDATE` upserts that
  preserve `first_seen`, and case-insensitive name search. A `scanned` table
  tracks fetch outcomes across runs and retains the raw `info` dictionary for
  offline re-analysis (e.g. a future classification table).

## CLI reference

### `run`

| Flag | Default | Description |
|------|---------|-------------|
| `--port <PORT>` | `6881` | UDP port to bind the DHT node |
| `--db <DB>` | `crawler.sqlite` | SQLite database path |
| `--concurrency <N>` | `512` | Max concurrent in-flight metadata fetches |
| `--ipv6` | off | Enable IPv6 DHT support |
| `--state-dir <DIR>` | `state` | Directory for the persisted routing table |
| `--bootstrap <HOSTS>` | 5 well-known nodes | Comma-separated bootstrap nodes |
| `--qps <N>` | `5000` | Aggregate DHT query budget shared by sampling and peer lookups |
| `--sampler-qps <N>` | `2000` | Sampler query budget across all sampling loops |
| `--sampler-loops <N>` | `32` | Number of concurrent sampling loops |
| `--min-seen <N>` | `2` | Emit an infohash only after N distinct sampling responses reported it (culls the junk tail) |
| `--lookup-concurrency <N>` | `64` | Max concurrent DHT `get_peers` lookups |
| `--max-nodes <N>` | `2048` | Maximum number of nodes in the DHT routing table |
| `--query-timeout <SECS>` | `5` | Timeout for individual DHT queries (seconds) |
| `--aggressive` | off | VPS preset: sampler-qps=4000, sampler-loops=64, concurrency=1024, lookup-concurrency=256, dht-qps=10000, max-nodes=4096, query-timeout=3 |
| `--blocklist <FILE>` | none | Blocklist file (IP or CIDR per line, `#` comments) |
| `--log <FILTER>` | env `RUST_LOG` | tracing filter override |

The daemon logs structured crawl stats (routing-table size, `announced_hashes`,
hashes sampled, fetch success, records persisted) every 30s and shuts down
gracefully on SIGTERM/SIGINT: it drains in-flight fetches, flushes the database
batch, and persists the routing table. `announced_hashes` is the number of
infohashes other DHT nodes have announced to us; it stays near 0 on a NAT host,
which tells us passive announce capture is not worth building.

### `query`

```
dht-crawler query <NAME> [--db <DB>]
dht-crawler query <NAME> [--db <DB>] --failures
```

Prints matching `name / size / file count`. With `--failures`, prints an
aggregate breakdown of metadata fetch failures by their dominant `failure_reason`
from the `scanned` table instead.

### `purge`

```
dht-crawler purge [--db <DB>] [--state-dir <DIR>] [--yes]
./run.sh --purge
```

Deletes the SQLite database (plus its WAL/SHM sidecars) and the persisted
routing state so the next `run` starts completely fresh. Prompts for
confirmation unless `--yes` is given.

## Options reference (behavior notes)

- On startup the persisted routing table (`state/dht_state.json`) is loaded, so
  restarts resume from a warm table. A missing or corrupt file simply starts
  empty.
- Sampling politely respects per-node BEP 51 intervals and a global per-second
  query budget, and concentrates queries on nodes that actually return samples.
- Metadata fetch failure rates are high by nature (dead peers, no `ut_metadata`,
  timeouts); the pool is sized for many cheap failures with tight timeouts, and
  previously-failed hashes are quarantined with exponential backoff (persisted
  in the `scanned` table, so it survives restarts). Every failed fetch records
  its dominant `failure_reason` (`timeout`, `connect_refused`, `no_bep10`,
  `no_ut_metadata`, `metadata_rejected`, `sha1_mismatch`, `empty_peers`,
  `deadline`, `other`) so you can diagnose why hashes fail with
  `dht-crawler query anything --failures`. All verified torrents store their raw
  `info` dictionary for offline re-analysis.
- `--min-seen 2` (default) skips the single-sighting junk tail, so fetch slots
  go to hashes confirmed by multiple nodes; raising it to `3` culls further at
  the cost of delaying rare releases. The popularity value also controls
  processing order regardless of the threshold.
- `--blocklist` lets you avoid dialing peers in given networks (e.g. honeypot
  ranges); one IP or CIDR per line, `#` comments allowed.

## Privacy & footprint

See [`docs/PRIVACY.md`](../docs/PRIVACY.md) for how the crawler is visible on
the DHT, why there is no "invisible" mode, and the operational controls
(BEP 43 read-only mode, a drop-inbound firewall recipe, and VPN/egress options)
that actually reduce or relocate your exposure.

## Licensing

This binary links `irontide-dht`, which is **GPL-3.0-or-later**; that license
propagates to `dht-crawler`. This is fine for local/personal use but must be
reconsidered before any public distribution.
