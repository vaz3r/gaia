# crawler

A lightweight BitTorrent DHT crawler that indexes torrents into a local SQLite
database. It bootstraps and maintains a persistent Kademlia routing table,
traverses the DHT keyspace with **BEP 51** (`sample_infohashes`), fetches torrent
metadata over TCP with **BEP 9/10** (`ut_metadata`), verifies each download by
SHA-1, and stores every accepted torrent with upsert semantics.

## Quickstart

```sh
cargo build --release

# Crawl (daemon)
./target/release/crawler run \
  --db crawler.sqlite \
  --state-dir state \
  --port 6881

# Search the index
./target/release/crawler query "matrix 1080p"
```

### Docker + Gluetun (recommended for public egress)

A `docker-compose.yml` runs the crawler behind a **Gluetun WireGuard client**,
so all crawler traffic (DHT UDP + metadata TCP) egresses from a public IP —
which is what makes peers reachable and raises the verify rate (behind NAT it
stays ~0.5%). The crawler container shares Gluetun's network namespace
(`network_mode: "service:gluetun"`); DHT ports 6881–6884 are opened inbound on
the tunnel.

**Docker context:** this stack is deployed to the `remote-dev` context
(`ssh://core@100.99.147.104`), not the local daemon. Bind mounts do **not**
propagate correctly there, so the stack uses a **named volume** (`dht-crawler-data`)
for the database and routing state. Confirm the context first:
`docker context use remote-dev`.

```sh
cd crawler
cp .env.example .env            # fill in WireGuard keys (gitignored)
docker compose up -d --build
docker compose logs -f crawler
```

Warm-start migration (first run): copy the existing DB + state into the named
volume via `docker cp` (bind mounts don't work on remote-dev):

```sh
docker run -d --name dht-seed -v dht-crawler-data:/data alpine sleep 300
docker cp crawler.sqlite dht-seed:/data/crawler.sqlite
docker cp state dht-seed:/data/state
docker rm -f dht-seed
```

Notes:
- The stack runs **8 instances** (ports 6881-6888) plus a **redis** service for a
  shared seen-set and dead-peer cache; the crawler is configured for
  low-bandwidth/high-discovery (lower QPS budgets, tight fetch budgets).
- The WireGuard server must be reachable on a non-default port (Oracle Cloud
  blocks the default 51820; this stack uses UDP `:443`, which does not conflict
  with TCP 443 HTTPS — WireGuard is UDP, web is TCP).
- If the tunnel's private DNS is unreachable, set
  `DNS_UPSTREAM_PLAIN_ADDRESSES` to public resolvers (default `1.1.1.1:53,8.8.8.8:53`).
- Data lives in the `dht-crawler-data` named volume on the daemon host.

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
| `--instances <N>` | `1` | Run N independent DHT nodes/samplers, sharing one DB. Instance i binds `port+i` and uses `state-dir/instance-i/` |
| `--qps <N>` | `2000` | Aggregate DHT query budget shared by sampling and peer lookups |
| `--sampler-qps <N>` | `400` | Sampler query budget across all sampling loops |
| `--sampler-loops <N>` | `32` | Number of concurrent sampling loops |
| `--min-seen <N>` | `1` | Emit an infohash to the fetcher on first sighting (bloom filter prevents re-fetching known-dead hashes) |
| `--scale <N>` | `10` | Concurrency scale factor (bitmagnet `scaling_factor`): multiplies sampler QPS/loops, fetch/lookup concurrency, and buffers |
| `--lookup-concurrency <N>` | `256` | Max concurrent DHT `get_peers` lookups |
| `--max-nodes <N>` | `4096` | Maximum number of nodes in the DHT routing table |
| `--max-attempts <N>` | `4` | Retry budget for transient failure classes (timeout, deadline, connect_refused, dht_lookup_failed, …). Dead-verdict classes (empty_peers, no_ut_metadata, …) are always capped at 2. A parallel retry worker actively drains retry-eligible failed hashes (`source: Retried`), so transient infrastructure failures get a second chance without waiting for the sampler to re-report them |
| `--no-restrict-ips` | off | Disable one-node-per-IP routing restriction (opt-in for NAT) |
| `--redis-url <URL>` | none | Shared seen-set + dead-peer cache across instances (e.g. `redis://redis:6379`); falls back to per-instance state if unreachable |
| `--query-timeout <SECS>` | `5` | Timeout for individual DHT queries (seconds) |
| `--aggressive` | off | VPS preset: sampler-qps=1000, sampler-loops=64, concurrency=512, lookup-concurrency=256, dht-qps=4000, max-nodes=8192, query-timeout=3 |
| `--blocklist <FILE>` | none | Blocklist file (IP or CIDR per line, `#` comments) |
| `--log <FILTER>` | env `RUST_LOG` | tracing filter override |

The daemon logs structured crawl stats (routing-table size, `announced_hashes`,
hashes sampled, fetch success, `fetch_in_flight`, `queue_depth`, records
persisted) every 30s and shuts down
gracefully on SIGTERM/SIGINT: it drains in-flight fetches, flushes the database
batch, and persists the routing table. `announced_hashes` is the number of
infohashes other DHT nodes have announced to us; it stays near 0 on a NAT host,
which tells us passive announce capture is not worth building.

### `query`

```
crawler query <NAME> [--db <DB>]
crawler query <NAME> [--db <DB>] --failures
```

Prints matching `name / size / file count`. With `--failures`, prints an
aggregate breakdown of metadata fetch failures by their dominant `failure_reason`
from the `scanned` table instead.

### `purge`

```
crawler purge [--db <DB>] [--state-dir <DIR>] [--yes]
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
  in the `scanned` table, so it survives restarts). A fetch aborts early if its
  first ~24 dials all fail to connect (no handshake reached), and IPs that fail
  to connect repeatedly are cached as dead for ~10 minutes so the same
  unreachable peers are not re-dialed for every hash. Every failed fetch records
  its dominant `failure_reason` (`timeout`, `connect_refused`, `no_bep10`,
  `no_ut_metadata`, `metadata_rejected`, `sha1_mismatch`, `empty_peers`,
  `deadline`, `early_abort`, `other`) so you can diagnose why hashes fail with
  `crawler query anything --failures`. All verified torrents store their raw
  `info` dictionary for offline re-analysis.
- `--min-seen 1` (default) emits every unique hash immediately; the in-memory
  bloom filter caches known-blocked verdicts so re-sampled dead hashes skip the
  database instead of re-fetching. Raising it culls the single-sighting tail at
  the cost of delaying rare releases. The popularity value also controls
  processing order regardless of the threshold.
- Failed hashes retry after exponential backoff starting at 1 minute (capped at
  6 hours); hashes that failed with no peers (`empty_peers`) retry after a fixed
  60 seconds, since their swarm may appear quickly.
- `--instances N` runs N independent DHT nodes on `port`..`port+N-1`, each with
  its own routing table (and state dir), all feeding one database — this
  multiplies discovery breadth. Use with `--aggressive` on a VPS. Each instance
  also runs a continuous routing grower (throttled `get_peers` on random
  targets) so its table keeps growing toward `--max-nodes` throughout the crawl.
- `--no-restrict-ips` lifts irontide's one-node-per-IP routing restriction;
  on NAT hosts where many peers share egress IPs this can grow the routing
  table with more distinct node IDs.
- `--blocklist` lets you avoid dialing peers in given networks (e.g. honeypot
  ranges); one IP or CIDR per line, `#` comments allowed.

## Privacy & footprint

See [`docs/PRIVACY.md`](../docs/PRIVACY.md) for how the crawler is visible on
the DHT, why there is no "invisible" mode, and the operational controls
(BEP 43 read-only mode, a drop-inbound firewall recipe, and VPN/egress options)
that actually reduce or relocate your exposure.

## Licensing

This binary links `irontide-dht`, which is **GPL-3.0-or-later**; that license
propagates to `crawler`. This is fine for local/personal use but must be
reconsidered before any public distribution.
