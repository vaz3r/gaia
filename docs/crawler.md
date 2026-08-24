# DHT Crawler — Architecture & Internals

This document describes how the BitTorrent DHT crawler in `craw/` works. It covers the
high-level design, the data flow end-to-end, each module's responsibility, the wire
protocols it speaks, the database schema, configuration, and operations.

---

## 1. What it does

The crawler is a **Mainline DHT (Kademlia) participant** that:

1. Joins the BitTorrent DHT and maintains a routing table of peers.
2. Continuously **harvests torrent infohashes** (`get_peers` / `announce_peer` traffic).
3. For each discovered infohash, **locates peers** that have the torrent and **fetches the
   torrent metadata** (the bencoded `info` dictionary) using the BEP 10 extension protocol
   (`ut_metadata`).
4. **Verifies** the metadata against the infohash (SHA-1 of the info dict) and **persists**
   the parsed torrent record plus sightings to PostgreSQL.

The goal is high-throughput metadata collection: discover infohashes passively, resolve
them to full torrent descriptions, and store them for downstream analytics.

---

## 2. High-level architecture

```
                    ┌───────────────────────────────────────────────────────┐
                    │                        DHT network                    │
                    └───────────────────────────────────────────────────────┘
                                          ▲  │  ▲  │
                              inbound    │  │  │  │  outbound (find_node,
                              queries    │  │  │  │  get_peers, ping)
                                          │  ▼  │  ▼
                    ┌───────────────────────────────────────────────────────┐
                    │                     net::worker (UDP)                  │
                    │   SO_REUSEPORT sockets, one worker task per socket     │
                    └───────────────────────────────────────────────────────┘
                                          │  handle_datagram
                                          ▼
                    ┌───────────────────────────────────────────────────────┐
                    │                      Router                            │
                    │  parses KRPC, dispatches query/reply, TxTable, tokens  │
                    └───────┬───────────────────────────┬───────────────────┘
                            │                           │
              inbound get_peers /               inbound find_node
              announce_peer                     (routing table growth)
                            │                           │
                            ▼                           ▼
                    ┌───────────────┐            ┌──────────────┐
                    │   Harvester   │            │    Walker    │
                    │ bloom dedup   │            │ FIND_NODE    │
                    │ + channels    │            │ iterative    │
                    └───────┬───────┘            └──────┬───────┘
                            │                          │ insert_nodes
              discovery_tx  │  verify_tx / announce_tx  ▼
                            │                   ┌────────────────┐
                            │                   │  RoutingTable  │
                            │                   │  160 buckets   │
                            │                   └────────────────┘
                            ▼
                    ┌───────────────────────────────────────────────────────┐
                    │                verify::run_pipeline                    │
                    │   semaphore-limited, round-robin across DHT nodes      │
                    └───────┬───────────────────────────────────────────────┘
                            │ per infohash
                            ▼
                    ┌───────────────────────────────────────────────────────┐
                    │        verify_infohash (fetch_pool)                    │
                    │   1. source_peers: iterative get_peers peer lookup     │
                    │   2. try_fetch: TCP → uTP metadata fetch, race N peers │
                    │   3. sha1 verify + BatchWriter push                    │
                    └───────┬───────────────────────────────────────────────┘
                            │
                            ▼
                    ┌───────────────────────────────────────────────────────┐
                    │                    PostgreSQL                          │
                    │  torrents · verification_jobs · infohash_sightings ·   │
                    │  metrics · fetch_peer_outcomes                         │
                    └───────────────────────────────────────────────────────┘
```

### Concurrency model

The whole program is one Tokio runtime (`#[tokio::main]`). Communication between stages
uses bounded `mpsc` channels (capacity 65536). Long-running background tasks are spawned
with `tokio::spawn` and joined via a single `tokio::select!` in `main`.

The three main pipelines are decoupled:

| Channel | Producer | Consumer | Payload |
|---|---|---|---|
| `discovery_tx` | Harvester | `flush_sightings` | `(Infohash, Source)` → `infohash_sightings` |
| `verify_tx` | Harvester / scheduler | verify pipeline | `Infohash` → metadata fetch |
| `announce_tx` | Harvester | verify pipeline | `(Infohash, SocketAddr)` → direct fetch |

---

## 3. Module map

| Module | Path | Responsibility |
|---|---|---|
| `main` | `src/main.rs` | Wire everything together, spawn tasks, select loop |
| `config` | `src/config.rs` | Env-driven configuration (`CRAW_*`) |
| `net` | `src/net/mod.rs` | UDP socket binding (`SO_REUSEPORT`) + worker loops |
| `net::rate_limit` | `src/net/rate_limit.rs` | Per-IP token-bucket rate limiter |
| `router` | `src/router.rs` | KRPC dispatch, outbound queries, TxTable, sybil replies |
| `dht::routing_table` | `src/dht/routing_table.rs` | Kademlia XOR routing table |
| `dht::node_id` | `src/dht/node_id.rs` | Node-ID generation (BEP 42 + random), sybil pools |
| `dht::walker` | `src/dht/walker.rs` | Iterative `find_node` routing-table population |
| `harvest` | `src/harvest/mod.rs` | Infohash dedup (Bloom) + channel fan-out |
| `harvest::bloom` | `src/harvest/bloom.rs` | Bloom filter |
| `krpc::codec` | `src/krpc/codec.rs` | Bencode encode/decode |
| `krpc::message` | `src/krpc/message.rs` | KRPC message model (`ping`, `find_node`, `get_peers`, `announce_peer`) |
| `krpc::token` | `src/krpc/token.rs` | HMAC-SHA1 write tokens |
| `krpc::tx_state` | `src/krpc/tx_state.rs` | In-flight query/response correlation (txid) |
| `verify` | `src/verify/mod.rs` | Verification pipeline orchestration |
| `verify::peer_source` | `src/verify/peer_source.rs` | Iterative peer lookup (`get_peers`) |
| `verify::fetch_pool` | `src/verify/fetch_pool.rs` | TCP/uTP metadata fetch, peer racing |
| `verify::wire` | `src/verify/wire.rs` | BEP 3 / BEP 10 / BEP 9 wire protocol |
| `verify::verify` | `src/verify/verify.rs` | SHA-1 integrity check |
| `verify::peer_cache` | `src/verify/peer_cache.rs` | Negative cache of bad peers |
| `storage::pg` | `src/storage/pg.rs` | PostgreSQL pool |
| `storage::jobs` | `src/storage/jobs.rs` | `verification_jobs` scheduler + retry/backoff |
| `storage::batch_writer` | `src/storage/batch_writer.rs` | Buffered multi-row writes |
| `storage::sightings` | `src/storage/sightings.rs` | `infohash_sightings` writer |
| `storage::torrents` | `src/storage/torrents.rs` | Info-dict parsing + `torrents` writer |
| `storage::peer_outcomes` | `src/storage/peer_outcomes.rs` | `fetch_peer_outcomes` writer |
| `storage::metrics_writer` | `src/storage/metrics_writer.rs` | Periodic metrics flush to DB |
| `storage::backfill` | `src/storage/backfill.rs` | Import legacy `metadata.bin` / `discovered.txt` |
| `storage::janitor` | `src/storage/janitor.rs` | Periodic cleanup of `dead` / `verified` rows |
| `storage::identity` | `src/storage/identity.rs` | Persistent node ID + sybil identities |
| `trace` | `src/trace.rs` | Sampled per-infohash lifecycle tracing |
| `metrics` | `src/metrics.rs` | Atomic counters + snapshot |

---

## 4. DHT layer

### 4.1 Node identity

Each DHT node has a 20-byte `NodeId` (`[u8; 20]`) and a set of **sybil identities**
(`CRAW_SYBILS`, default 16). Identities are persisted per node in
`node_<i>/identity.json`.

- The **self ID** is generated per **BEP 42** (CRC32C-based) when an external IP is
  configured, otherwise uniformly random.
- Sybil IDs are split: 1/3 BEP 42, 2/3 random (`SybilPool::Bep42` / `SybilPool::Random`).
- BEP 42 IDs place the first 3 bytes in a prefix derived from the public IP, which makes
  the node resilient to IP-spoofing-based poisoning and helps the node be "close" to its
  own IP space.

The router responds to inbound `find_node` / `get_peers` with a mix of sybil identities
(announced at the node's public IP) and real routing-table entries
(`closest_phantom`), which lets the crawler influence what other DHT nodes store about it.

### 4.2 Routing table (`routing_table.rs`)

A classic Kademlia routing table:

- **160 buckets**, each a `VecDeque` of at most `K = 8` nodes.
- Bucket index is derived from the XOR distance between `self_id` and the node's ID.
- `insert` de-duplicates by **node ID** and updates the address if the ID is already
  present; when a bucket is full it evicts the oldest entry (FIFO).
- `closest(target, n)` returns the `n` nodes with smallest XOR distance to a target
  (used to bootstrap a `get_peers` lookup).
- `random_nodes(n)` samples uniformly across non-empty buckets (used by the walker).

A Kademlia XOR bucket table concentrates uniform IDs in the *far* buckets, so a healthy
table is typically **~100–400 nodes**, not 160×8 = 1280 (documented in the
`flood_of_new_ids_reaches_kademlia_equilibrium` test).

The table is persisted to `node_<i>/routing_table.bin` (bincode) every 60 seconds and
restored on startup, so a restart does not start cold.

### 4.3 Walker (`walker.rs`)

The walker keeps the routing table fresh. On an interval (`CRAW_WALKER_INTERVAL_MS`,
default 250ms):

1. Pick `CRAW_WALKER_ALPHA` random nodes from the routing table.
2. Pick a lookup **target**: 10% of the time it targets the node's own ID-space
   (`self`); otherwise it targets a random sybil ID.
3. Send `find_node` queries to those nodes, rate-limited per IP.
4. On response, ingest the responder and the returned `nodes` (compact node info,
   optionally `nodes6`) into the routing table.

The walker uses a non-blocking `JoinSet`: each tick it reaps completed queries
(`try_join_next`) and spawns the next batch, so steps overlap instead of blocking on
`QUERY_TIMEOUT` (5s).

### 4.4 KRPC protocol (`krpc/`)

The crawler speaks **KRPC** (BEP 5) over UDP:

- **Bencode** codec (`codec.rs`) with `decode_prefix` (returns value + consumed bytes)
  and `encode_to_bytes`.
- **Message** model (`message.rs`): `{t, y, q/a|r|e}` — transaction id, type
  (`q` query / `r` response / `e` error), method and arguments.
- **Transactions** (`tx_state.rs`): a `DashMap` keyed by 2-byte txid. `send_query`
  registers an entry with a `oneshot` reply channel; when a response arrives,
  `handle_reply` looks up the txid and wakes the waiter. Timeouts are enforced with
  `tokio::time::timeout`; stale entries are swept by `cleanup_tx` (10s tick, 30s TTL).
- **Write tokens** (`token.rs`): HMAC-SHA1 over the requester's IP + a time epoch,
  with secret rotation and previous-secret overlap for tolerance. Used to validate
  `announce_peer` requests.

The router answers four query types (`router.rs`):

| Query | Response |
|---|---|
| `ping` | `{id}` |
| `find_node` | `{id, nodes}` (8 closest, mixing sybils + real table) |
| `get_peers` | `{id, token, nodes}` (harvest the queried infohash) |
| `announce_peer` | `{id}` (validate token, harvest with a direct peer address) |

---

## 5. Harvesting (`harvest/`)

Every inbound `get_peers` and `announce_peer` is a *sighting* of a torrent infohash. The
`Harvester`:

- De-duplicates infohashes with a **Bloom filter** (two rotating generations: `current`
  and `previous`). A rotation swaps and clears the current filter when it reaches
  `bloom_capacity` (default 1,000,000), so genuinely new infohashes that fall out of the
  current window are still accepted for a while.
- `announce_peer` sightings use a **separate** smaller bloom filter so that a prior
  `get_peers` first-sighting does not suppress the higher-value direct fetch.
- Routes each *unique* infohash to:
  - `verify_tx` (metadata fetch queue), and
  - `discovery_tx` (sighting record for `infohash_sightings`).
- `announce_peer` sightings additionally carry a **direct peer address** and are routed
  to `announce_tx`, which triggers an immediate direct-fetch attempt (the announcer is
  almost certainly a peer that has the torrent).

Sources are tagged `get_peers` or `announce_peer` (`Source`).

---

## 6. Verification pipeline (`verify/`)

### 6.1 Orchestration (`verify/mod.rs`)

`run_pipeline` consumes `verify_tx` and `announce_tx`:

- A **semaphore** limits concurrent verifications to `CRAW_FETCH_LIMIT` (default 128).
- Infohashes are dispatched **round-robin** across the configured DHT nodes
  (`CRAW_NODES`, default 1).
- `announce_tx` items (direct fetches) are preferred and stored in an
  `AnnouncePeerCache` (TTL 600s) so that a later `get_peers` verification of the same
  infohash can re-use the known-good announcer as a peer candidate.

For each infohash, a spawned task runs `verify_infohash`, then handles the result:

| Result | Action |
|---|---|
| `Success(meta)` + SHA-1 pass | `push_torrent` + `push_verified` (batch writer) |
| `Success(meta)` + SHA-1 fail | `push_failed(ih, "sha1_mismatch")` |
| `NoPeers` | `push_failed(ih, "no_peers")` |
| `SourceTimeout` | `push_failed(ih, "source_timeout")` |
| `MetadataFailed` | `push_failed(ih, "no_metadata")` |

### 6.2 Peer discovery (`verify/peer_source.rs`)

`source_peers` locates candidate peers for an infohash using an **iterative `get_peers`
lookup**:

- Start from the `K = 8` closest routing-table nodes to the infohash (or random nodes if
  the table is empty).
- Each round queries up to `ALPHA = 3` not-yet-queried candidates concurrently
  (`get_peers`), up to `MAX_ROUNDS = 8`.
- Responses yield compact `values` (6-byte IPv4 + port), filtered through `is_routable`
  (drops loopback/private/link-local/multicast/100.64/10 CGNAT), and `nodes` (new
  candidates, sorted by XOR distance and de-duplicated by ID, truncated to K).
- Returns up to `race_peers` peers as `SourceResult::Peers`, or
  `NoPeers` / `AllTimeout` if nothing was found.

### 6.3 Metadata fetch (`verify/fetch_pool.rs`)

`verify_infohash` takes the peer list, prepends any cached/direct announcer, truncates to
`race_peers` (default 8), and **races** the peers concurrently in a `JoinSet`. Each peer
goes through `try_fetch`:

1. **TCP connect + handshake** (`connect_tcp`, `TCP_TIMEOUT = 5s` shared between connect
   and handshake).
2. On TCP failure, **fall back to uTP** (`connect_utp`, `UTP_TIMEOUT = 5s`) when a uTP
   socket is available (`CRAW_UTP_ENABLED`). The fallback is **sequential**, not a
   parallel race.
3. `fetch_metadata(fetch_timeout)` with `CRAW_FETCH_TIMEOUT_MS` (default 25000ms).

The first successful metadata wins; the rest are aborted. Every peer outcome is recorded
to `fetch_peer_outcomes` (transport, result, client string). Failures are broken down into
granular counters (`fetch_connect_timeout`, `fetch_connect_io`, `fetch_handshake`,
`fetch_no_extension`, `fetch_reject`, `fetch_bad_piece`, `fetch_io`, …). Connect I/O
failures are logged at a ~1/500 sample rate.

### 6.4 Wire protocol (`verify/wire.rs`)

The peer connection implements the BitTorrent peer wire protocol:

- **BEP 3 handshake**: 68 bytes — `19` + "BitTorrent protocol" + 8 reserved bytes (bit
  `0x10` = extension support) + infohash + peer ID. The peer must echo the same infohash
  and set the extension bit, else `Handshake` / `NoExtension`.
- **BEP 10 extension handshake**: an extended message (`msg_id = 20`, `ext_id = 0`)
  advertising `m = {ut_metadata: OUR_UT_METADATA_ID}` (we advertise ID **1**). The peer's
  response gives us its own `ut_metadata` ID and `metadata_size`, plus client info
  (`v`), `reqq`, and the full extension map (used for diagnostics).
- **BEP 9 metadata transfer** (`fetch_metadata`): send `{msg_type:0, piece:N}` requests
  using the **peer's** `ut_metadata` ID, receive `{msg_type:1, piece:N, total_size}` data
  (or `msg_type:2` reject), and reassemble pieces until `metadata_size` bytes are
  collected (piece size 16 KiB, cap 4096 pieces).

> **Extension-ID direction (important).** Per BEP 10, the `m` map in a handshake
> advertises the IDs that the *sender* of that handshake uses when *sending* extension
> messages. So:
> - we send metadata requests using the **peer's** ID (`self.ut_metadata`), but
> - the peer sends metadata responses using **our** ID (`OUR_UT_METADATA_ID = 1`).
>
> The response-filtering check must compare against our own ID. A regression that
> compared against `self.ut_metadata` (the peer's ID) silently skipped every valid
> metadata response and timed out — collapsing the metadata success rate from ~74% to
> ~3%. See commit `fix(verify): correct BEP 10 ut_metadata extension ID direction` and
> the `metadata_response_uses_our_advertised_id` regression test.

### 6.5 Integrity check (`verify/verify.rs`)

The fetched metadata (the bencoded `info` dictionary) is hashed with SHA-1 and compared to
the infohash. A mismatch means the peer sent garbage and the result is discarded.

---

## 7. Storage layer (`storage/`)

### 7.1 Schema

| Table | Purpose |
|---|---|
| `infohash_sightings` | Every infohash sighting: first/last seen, per-source counts, total seen |
| `torrents` | Parsed torrent metadata (name, piece length, sizes, files) keyed by infohash |
| `verification_jobs` | The work queue: status + retry count + backoff + last error |
| `metrics` | Time-series of `(ts, metric_name, metric_value)` |
| `fetch_peer_outcomes` | Per-peer fetch outcomes (transport, result, client) |

`verification_jobs.status` values: `pending`, `verifying`, `verified`, `failed`, `dead`.

### 7.2 Job lifecycle & retries (`jobs.rs`)

The `VerifyStore`:

- `claim_due` atomically claims a mix of **fresh** (`pending`, ~70%) and **retry**
  (`failed` and past `next_retry_at`, ~30%) jobs, marking them `verifying`
  (`FOR UPDATE SKIP LOCKED`).
- Backoff schedule: 60s, 300s, 1800s, 7200s, 43200s.
- After `MAX_RETRIES = 4` failures (or a second `no_peers` failure), a job is marked
  `dead`.
- A scheduler re-injects claimed jobs every 15s and resets stale `verifying` rows every
  5 minutes (crash recovery).

> **Note:** successfully-verified infohashes are *deleted* from `verification_jobs`
> (see `batch_writer.rs` `delete_verified`), not left as `status = 'verified'`. The
> authoritative "verified" record is the row in `torrents`. The `verified` status string
> still exists for the legacy/backfill path; the janitor periodically deletes old
> `verified` rows.

### 7.3 Batch writer (`batch_writer.rs`)

The hot path does **zero DB round trips**. Results are pushed into in-memory buffers and
flushed every 1s as multi-row UPSERTs:

- `push_torrent` / `push_verified` / `push_failed` only lock a `Mutex<Vec>`.
- `flush` drains both buffers and emits chunked (5000 rows) `INSERT … ON CONFLICT DO
  UPDATE` statements for `verification_jobs` and `torrents`, plus `DELETE` for verified
  infohashes. A single `flushing` flag prevents concurrent flushes.
- Retry-count computation happens at flush time (one `SELECT` per chunk to read current
  `retry_count`, then a single upsert), preserving the backoff schedule.

This was a major throughput win: previously each verification performed multiple
round trips; buffering cut the hot-path DB cost to a single `Mutex::push`.

### 7.4 Other writers

- `sightings.rs` — batches `infohash_sightings` upserts (256/chunk) every 500ms.
- `peer_outcomes.rs` — batches `fetch_peer_outcomes` inserts every 30s.
- `metrics_writer.rs` — flushes the atomic metric snapshot to `metrics` every 60s.
- `janitor.rs` — every 4h deletes `dead` rows older than 1 day and stale `verified` rows.
- `backfill.rs` — `--backfill` mode imports legacy `metadata.bin` / `discovered.txt`.

---

## 8. Networking (`net/`)

- `bind_reuseport` creates `worker_threads` UDP sockets bound to the same address with
  `SO_REUSEADDR` + `SO_REUSEPORT`, spreading inbound traffic across cores.
- Each socket gets a `worker` task that loops `recv_from` → `handle_datagram`, draining
  any immediately-available datagrams with `try_recv_from` (busy-poll) before awaiting.
- The router round-robins sends across the socket pool (`next_socket`).
- `rate_limit.rs` provides a per-IP token bucket (`CRAW_RATE_LIMIT`, default 8/s, burst
  64) used by the walker to avoid hammering individual hosts.

---

## 9. Configuration

All configuration is env-driven (`config.rs`). Defaults in parentheses.

| Env var | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | (required) | PostgreSQL connection string |
| `CRAW_BIND` | `0.0.0.0:6881` | UDP bind address |
| `CRAW_EXTERNAL_IP` | — | Public IP (enables BEP 42 IDs) |
| `CRAW_BOOTSTRAP` | 5 public routers | Comma-separated bootstrap hosts |
| `CRAW_WORKERS` | ncpu | UDP sockets / worker tasks per node |
| `CRAW_SYBILS` | 16 | Sybil identities per node |
| `CRAW_TOKEN_WINDOW` | 300s | Write-token epoch window |
| `CRAW_BLOOM_CAPACITY` | 1,000,000 | Bloom filter size |
| `CRAW_WALKER_ALPHA` | 16 | Nodes queried per walker step |
| `CRAW_WALKER_INTERVAL_MS` | 20 | Walker step interval |
| `CRAW_FETCH_LIMIT` | 128 | Max concurrent verifications |
| `CRAW_RACE_PEERS` | 8 | Peers raced per infohash |
| `CRAW_DATA_DIR` | `data` | Node state directory |
| `CRAW_RATE_LIMIT` | 8.0 | Per-IP outbound query rate |
| `CRAW_TRACE_SAMPLE_RATE` | 0.0 | Lifecycle-trace sampling |
| `CRAW_DEBUG_IH` | — | Always-trace a specific infohash (hex) |
| `CRAW_PARSE_NODES6` | false | Parse `nodes6` compact IPv6 nodes |
| `CRAW_NODES` | 1 | DHT nodes to run (ports `PORT_BASE + i`) |
| `CRAW_PORT_BASE` | 6881 | First UDP port |
| `CRAW_UTP_ENABLED` | true | Enable uTP fallback for metadata fetch |
| `CRAW_FETCH_TIMEOUT_MS` | 8000 | Metadata fetch timeout |

---

## 10. Observability

- **Structured logs** via `tracing`. A 15s `report_loop` prints a full metrics snapshot,
  including `verified_per_hour` and `unique_per_hour` (instantaneous deltas).
- **Metrics** are atomic counters (`metrics.rs`), snapshotted and flushed to the `metrics`
  table every 60s.
- **Lifecycle tracing** (`trace.rs`): per-infohash structured events (`discovered`,
  `source_start`, `source_response`, `fetch_start`, `connect_result`, `ext_handshake`,
  `metadata_piece`, `metadata_done`, `job_update`, …). Sampling is controlled by
  `CRAW_TRACE_SAMPLE_RATE`, with `CRAW_DEBUG_IH` for targeted tracing. This was
  instrumental in diagnosing the BEP 10 extension-ID bug.

Key success-path metrics to watch:

- `verify_success / verify_attempts` — end-to-end success rate (healthy ≈ 15–20%).
- `tcp_metadata_ok / tcp_connect_ok` and `utp_metadata_ok / utp_connect_ok` — metadata
  yield per transport after a successful connect (healthy ≈ 70%+).
- `source_peers_returned`, `source_no_peers`, `source_timeout` — peer-discovery health.
- `fetch_connect_timeout`, `fetch_connect_io`, `fetch_io` — fetch failure breakdown.

---

## 11. Deployment

The crawler is a single Rust binary cross-compiled for the target architecture
(`aarch64-unknown-linux-gnu` on the Oracle ARM VM; the dev box is x86-64):

```sh
cargo build --release --target aarch64-unknown-linux-gnu
```

It runs in Docker with the binary bind-mounted at `/usr/local/bin/craw`, `network_mode:
host`, and a volume for `CRAW_DATA_DIR`. Environment is supplied via `docker compose`
(`CRAW_FETCH_LIMIT`, `CRAW_UTP_ENABLED`, `DATABASE_URL`, …). State files:

- `node_<i>/identity.json` — node + sybil identities.
- `node_<i>/token_secret.bin` — write-token secret.
- `node_<i>/routing_table.bin` — routing-table snapshot (restored on restart).

A local PostgreSQL is used as the operational datastore (the crawler connects via
`127.0.0.1`); migrations run automatically on startup.

### Useful commands

```sh
# Run with migrations + all background tasks
craw

# One-shot import of legacy data, then exit
craw --backfill

# Cross-compile + deploy (example)
cargo build --release --target aarch64-unknown-linux-gnu
scp target/aarch64-unknown-linux-gnu/release/craw zerone:/tmp/craw
# then swap the bind-mounted binary and restart the container
```

---

## 12. Historical note: the BEP 10 regression

The single largest performance regression in this codebase's history was a one-line
extension-ID direction bug in `verify/wire.rs`. The crawler's metadata success rate
collapsed from ~74% (and ~15–20% end-to-end success) to ~3% (and ~0.5% end-to-end) — a
~30× drop — because the response filter compared the incoming extension message ID
against the *peer's* `ut_metadata` ID instead of *our own* advertised ID. This caused
every valid metadata response to be skipped, after which the fetch timed out.

The tell was in the lifecycle traces: `metadata_timeout_with_skipped skipped_non_ext=1`
on every failure (exactly one skipped message = the valid response), and
`metadata_size` was advertised but no data ever arrived. The fix introduced the
`OUR_UT_METADATA_ID` constant and a regression test
(`metadata_response_uses_our_advertised_id`) that simulates a peer replying with our
advertised ID and asserts the client does not skip it.

A second, smaller regression was the routing-table **inbound auto-insert**: every inbound
query added the querying node to the routing table, causing hundreds of millions of
insert calls and evicting responsive walker-discovered nodes. Reverting it (routing-table
populated only by walker responses) reduced insert churn by orders of magnitude and
improved peer-discovery quality.

---

## 13. Key constants (quick reference)

| Constant | Value | Where |
|---|---|---|
| `K` (routing bucket size) | 8 | `routing_table.rs` |
| `ALPHA` (get_peers fanout) | 3 | `peer_source.rs` |
| `MAX_ROUNDS` (get_peers) | 8 | `peer_source.rs` |
| `QUERY_TIMEOUT` (DHT) | 5s | `walker.rs`, `peer_source.rs` |
| `TCP_TIMEOUT` | 5s | `fetch_pool.rs` |
| `UTP_TIMEOUT` | 5s | `fetch_pool.rs` |
| `OUR_UT_METADATA_ID` | 1 | `wire.rs` |
| `PIECE_SIZE` | 16384 | `wire.rs` |
| `MAX_PIECES` | 4096 | `wire.rs` |
| `MAX_MESSAGE_LEN` | 16 MiB | `wire.rs` |
| `MAX_RETRIES` | 4 | `jobs.rs` |
| `CHANNEL_CAPACITY` | 65536 | `main.rs` |
| `ROUTING_SNAPSHOT_INTERVAL` | 60s | `main.rs` |
| `ANNOUNCE_CACHE_TTL` | 600s | `verify/mod.rs` |
| `PeerCache` TTL | 600s | `peer_cache.rs` |
| Backoff schedule | 60/300/1800/7200/43200s | `jobs.rs`, `batch_writer.rs` |
