# Gaia Architecture (Current State)

## 1. Primary Components
Gaia is composed of five interconnected subsystems:

1. **UDP Net & Routing (`net/`, `krpc/`, `router.rs`)**: High-throughput packet ingestion. Leverages Linux `recvmmsg` across `SO_REUSEPORT` sockets for kernel-level packet batching.
2. **Kademlia DHT (`dht/`)**: Maintains network topology. Powered by a Multi-Sybil routing table architecture and an active DHT Walker.
3. **Harvester Pipeline (`harvest/`, `bep51.rs`)**: Sits between the UDP router and the verifiers. Uses rotating Bloom filters to deduplicate incoming infohash sightings and BEP-51 samples.
4. **Metadata Verifier (`verify/`)**: The outbound engine. Resolves raw infohashes to `.torrent` dictionaries via TCP and uTP BEP-9 connections. Spoofs sybil identities to minimize query latency.
5. **Storage & Janitor (`storage/`)**: Flushes results asynchronously to Postgres via a non-blocking `BatchWriter`. Cleans terminal/dead rows on an interval to bound database growth.

## 2. Inbound Data Flow
Data flows from raw UDP datagrams to persistent storage through the following pipeline:

1. **Ingest (`net/mmsg.rs`)**: Linux kernel hands a batch of datagrams to the crawler.
2. **Parse (`krpc/scanner.rs`)**: Zero-copy scanner validates the packet is structurally sound Bencode.
3. **Route (`router.rs`)**: The router decodes the KRPC query. `ping` and `find_node` are answered synchronously.
4. **Harvest (`harvest/mod.rs`)**: `get_peers`, `announce_peer`, and `sample_infohashes` results are intercepted. The 20-byte target infohashes are tested against a Bloom Filter.
5. **Queue (`main.rs`)**: Novel infohashes are piped to the `fresh_channel` Tokio MPSC channel.
6. **Verify (`verify/wire.rs`)**: Verifier tasks pull hashes from the queue, connect to the declaring peer, and perform the BitTorrent ext-handshake to download metadata.
7. **Persist (`storage/batch_writer.rs`)**: Verifiers push the result (Success or Fail) to the `BatchWriter` which executes bulk `INSERT` statements into PostgreSQL.

## 3. Worker Threads and Concurrency
The application runs on a shared, work-stealing Tokio multi-threaded async runtime.

- **Workers**: Typically bound to CPU core count (`CRAW_WORKERS`).
- **Network Bound Tasks**: `verify::run_pipeline` limits concurrency through a token bucket (`CRAW_GLOBAL_FETCH_LIMIT`).
- **UDP Receive Loops**: Spawns one dedicated `tokio::spawn` loop per worker thread, bound to a dedicated UDP socket via `SO_REUSEPORT`.
- **Database Threads**: PostgreSQL connection pool limits concurrent outbound queries (`storage.pg_pool_max_connections`).

## 4. Unbounded Growth Risks
- **`fresh_channel` queue**: Heavily loaded by BEP-51. Currently bound to 65,536 elements (`CRAW_FRESH_CHANNEL_CAPACITY`). When full, the Harvester actively drops new hashes.
- **Routing Table**: Bound by `K=8` per bucket, per Sybil. The total memory is explicitly capped by the static 160-bit routing tree depth.
- **Postgres Database**: `verification_jobs` bounded by the background Janitor loop which deletes terminal rows older than a day.

## 5. Security & Trust Boundaries
- **Packet Validation**: Every UDP datagram is treated as untrusted. Malformed bencode drops early without allocation.
- **Amplification Prevention**: Rate limiter uses a Dashmap to enforce strict token/IP quotas on inbound requests to prevent reflection attacks.
- **Sybil Identities**: Configured locally (`identity.json`). The DHT does not inherently trust the crawler's Sybils, but responds to them neutrally according to standard Kademlia routing.

## 6. State and Persistence Model
- **Important in-memory state**: `RoutingTable` (Kademlia tree), `TxTable` (transaction mappings), `BloomFilter` (recently seen hashes), `RateLimiter` (IP backpressure).
- **Important database entities**: `torrents` (metadata payload), `infohash_sightings` (discovery timestamps), `verification_jobs` (retry state machine), `metrics`.
- **Survives Restart**: Postgres data, `identity.json`, `token_secret.bin`, and the local `routing_table.bin` snapshot.
- **Lost on Restart**: In-flight fetch connections, `fresh_channel` queues, current Bloom filter state.

## 7. Configuration Map
Loaded from environment variables (`.env`) via `config.rs`.

- **Sybil Multiplier**: `CRAW_SYBILS` controls how many identities the crawler broadcasts (Directly controls discovery throughput).
- **Concurrency**: `CRAW_GLOBAL_FETCH_LIMIT` and `CRAW_PIPELINE_LIMIT` control outbound verification limits.
- **Channels**: `CRAW_FRESH_CHANNEL_CAPACITY` controls queue depth before dropping.

## 8. Observability
- **Metrics**: Lock-free counters (`AtomicU64`) in `metrics.rs` snapshot every 15s to Postgres.
- **Logs**: Structured `tracing` spans cover the infohash lifecycle.
- **Tools**: `./deploy/scripts/health.sh` calculates real-time N-minute sliding window throughput from the database.

## 9. Repository Map

```text
apps/crawler/src/
  main.rs               # Application entry point and Tokio wiring
  config.rs             # Environment variable parsing and validation
  metrics.rs            # Atomic lock-free observability counters
  router.rs             # UDP Packet router and DHT response generator
  dht/
    bep51.rs            # Active network sampling worker
    routing_table.rs    # Multi-Sybil In-memory XOR node tree
    walker.rs           # Background table-filling DHT crawler
  krpc/
    scanner.rs          # Fast-path Bencode parser for UDP hot path
    message.rs          # Full KRPC message definitions and types
  net/
    mod.rs              # SO_REUSEPORT UDP worker loops
  harvest/
    mod.rs              # Infohash deduplication via Bloom filters
  verify/
    fetch.rs            # Connection orchestration (TCP/uTP racing)
    wire.rs             # BitTorrent wire protocol and BEP-9 exchange
    peer_source.rs      # DHT active-peer lookup mechanism
  storage/
    batch_writer.rs     # Buffered, asynchronous Postgres writer
    jobs.rs             # Failure retry scheduler for infohashes
```

## 10. Technical Lead Summary
**Five Important Facts:**
1. High UDP throughput is supported by utilizing `recvmmsg` and `SO_REUSEPORT` to spread packets across multiple Tokio tasks in the Linux kernel natively.
2. Outbound peer interactions attempt to aggressively lower latency by concurrently racing TCP and uTP transport connections (`try_fetch`).
3. Postgres insert performance is protected by aggressive in-memory batching (`BatchWriter`) rather than writing individual rows.
4. The router uses `closest_sybil()` mathematically spoof outbound query origins to minimize Kademlia hops.
5. In-memory Bloom filters deduplicate info-hashes quickly; without this, the verification queue would easily be overwhelmed by duplicate `get_peers` requests.

**Three Areas for Deeper Inspection:**
1. **OS-Level UDP Packet Drops**: Determine if sudden query bursts overwhelm `SO_RCVBUF` despite the rate limiter.
2. **Postgres Write Locks**: Assess the efficiency of the PostgreSQL `ON CONFLICT` updates for `infohash_sightings` under maximum sustained throughput.
3. **uTP vs TCP Success Ratios**: Evaluate the necessity of maintaining the custom uTP stack based on real-world metadata pull success rates.
