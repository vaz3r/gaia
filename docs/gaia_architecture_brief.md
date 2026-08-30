# Gaia Crawler Architecture Brief

## 1. Crawler Definition

Gaia is a high-throughput BitTorrent Distributed Hash Table (DHT) crawler written in Rust and powered by Tokio and PostgreSQL. 

Its primary output consists of verified BitTorrent metadata (the `info` dictionary payload of a torrent file) and discovery sighting metrics, both persisted to PostgreSQL. 

Gaia interacts with the BitTorrent mainline DHT network using the UDP-based KRPC protocol to locate peers and info-hashes. When a new info-hash is discovered, it initiates outbound TCP and uTP connections using the BitTorrent wire protocol and the BEP-9 extension protocol to fetch the metadata directly from active peers. 

A torrent is considered successfully processed when the SHA-1 digest of the downloaded metadata exactly matches the requested info-hash, at which point the metadata is saved to the database.

Gaia deliberately **does not** participate in file downloading, uploading, or general BitTorrent swarming beyond acquiring the metadata dictionary.

The application lifecycle follows a standard pattern: load configuration from the environment, connect to the database and run migrations, spawn non-blocking worker tasks (UDP workers, harvesters, verification pipelines, DB writers, and janitors), and run indefinitely until a graceful shutdown signal (e.g., SIGINT) initiates a drain of pending database batches.

## 2. End-to-End Data Flow

The following traces a single info-hash from discovery to database storage:

1. **DHT UDP Packet Reception**: UDP packets are received by `net::worker` (`apps/crawler/src/net/mod.rs`), which loops over `recv_from` and passes buffers to the Router.
2. **KRPC Parsing and Routing**: The `Router::handle_datagram` (`apps/crawler/src/router.rs`) uses `krpc::scanner::scan` to peek at the packet. Valid DHT requests (like `ping` or `find_node`) are answered via fast paths. 
3. **DHT Walking and Outbound Queries**: Concurrently, `Walker::run` (`apps/crawler/src/dht/walker.rs`) periodically queries the network to keep the local `RoutingTable` populated with active nodes.
4. **Info-Hash Discovery**: When the router receives `get_peers` or `announce_peer`, it extracts the info-hash and calls `do_harvest`.
5. **Deduplication and Eligibility Checks**: The event is passed to `Harvester::harvest` (`apps/crawler/src/harvest/mod.rs`), which checks it against rotating Bloom filters. If previously seen, it is dropped.
6. **Fresh-Item Queue**: If novel, the info-hash is sent via `fresh_verify_tx`. If a direct peer IP is known (from `announce_peer`), it also pushes the peer to `announce_tx`.
7. **Verification Scheduling**: The central loop in `verify::run_pipeline` (`apps/crawler/src/verify/mod.rs`) merges items from the fresh queue, the announce queue, and the retry queue into a unified task pipeline.
8. **Peer Discovery and Selection**: `verify_infohash` (`apps/crawler/src/verify/fetch_pool.rs`) first checks the `AnnouncePeerCache` or the injected direct peer. If none exist, it delegates to `source_peers` (`apps/crawler/src/verify/peer_source.rs`) to query the DHT for active peers.
9. **TCP/uTP Connection**: `try_fetch` attempts to connect to the peer. Using `race_transports`, it races TCP (`WireSession::connect_tcp`) and uTP (`connect_utp`) connection attempts in `apps/crawler/src/verify/fetch_pool.rs` and `apps/crawler/src/verify/wire.rs`.
10. **BitTorrent Metadata Exchange**: Upon connection, `WireSession::fetch_metadata` executes the BEP-9 extension handshake and downloads the pieces of the metadata dictionary.
11. **Metadata Validation and SHA-1 Verification**: The downloaded payload is checked in `verify::check` (`apps/crawler/src/verify/verify.rs`) to ensure `Sha1::digest(metadata) == infohash`.
12. **Database Persistence**: If valid, `run_pipeline` sends the data to `BatchWriter::push_torrent` (`apps/crawler/src/storage/batch_writer.rs`), which periodically flushes to the PostgreSQL `torrents` table.
13. **Failure, Retry, Expiration and Cleanup**: If fetching fails, `push_failed` is called. The job is tracked in the `verification_jobs` table. The scheduler (`VerifyStore::run_scheduler` in `apps/crawler/src/storage/jobs.rs`) will pick it up later based on exponential backoff until it hits max retries, after which `janitor::run` (`apps/crawler/src/storage/janitor.rs`) eventually cleans up dead rows.

## 3. Runtime Architecture

Gaia is built on a heavily concurrent `tokio` multi-threaded runtime.

- **Long-running tasks and worker loops**: The `main` function spawns distinct async loops for UDP sockets (`net::worker`), DHT crawling (`walker.run`), info-hash deduplication (`run_harvester`), metadata fetching (`run_pipeline`), database batching (`BatchWriter::run`, `SightingWriter::run`), metrics flushing, retry scheduling, and periodic janitor cleanups.
- **Shared state**: Application state is split into specialized structures. The `RoutingTable` and `TxTable` (in-flight transactions) are wrapped in `Arc<Mutex>`. Connection limiters and caches use concurrent lock-free maps (e.g., `DashMap`). The database connection pool (`PgPool`) is cloned natively.
- **Channels and queues**: Tokio `mpsc` bounded channels route data asynchronously between components: `harvest_tx` for incoming info-hashes, which routes to `discovery_tx`, `fresh_verify_tx`, `announce_tx`, and `verify_tx`.
- **Semaphores and concurrency controls**: `tokio::sync::Semaphore` bounds system load. The `pipeline_limit` restricts concurrent metadata extractions. `fetch_limit` restricts global outbound peer fetches. `ConnLimiter` issues per-IP permits to avoid hammering multi-port seedboxes.
- **Timers and maintenance loops**: `tokio::time::interval` is heavily utilized for periodic batch flushing, rate limiter sweeps, cache expirations, and janitor runs.
- **Blocking Work**: File and database operations do not use explicit native blocking threads, as `sqlx` and configuration loading are entirely async-aware. 
- **Shutdown**: A `tokio::sync::broadcast` channel signals a graceful shutdown on SIGINT, allowing `BatchWriter` and `SightingWriter` to flush pending SQL transactions before the process exits.

## 4. Major Components

- **Network/UDP layer** (`net/mod.rs`): Initializes UDP sockets utilizing `SO_REUSEPORT`, allowing multiple `tokio` worker tasks to independently poll the same port for high throughput.
- **KRPC scanner/parser** (`krpc/scanner.rs`, `krpc/message.rs`): A specialized parser that scans incoming bytes for known Bencode keys without allocating a full AST, optimizing response latency for high-frequency queries.
- **Router** (`router.rs`): Central traffic cop for the DHT. Answers `ping` and `find_node` requests, delegates responses to pending outgoing transactions, and extracts discovery payloads.
- **DHT walker** (`dht/walker.rs`): A proactive background crawler that pings unverified nodes to ensure the local routing table stays relevant.
- **Routing table** (`dht/routing_table.rs`): An in-memory, bucket-based tree relying on XOR metric distance for Node ID positioning and closest-node lookups.
- **Info-hash discovery** (`harvest/mod.rs`): Holds dual `BloomFilter` instances to filter out duplicated info-hashes quickly before they can flood downstream channels.
- **Verification scheduler** (`storage/jobs.rs`): Checks Postgres for info-hashes that previously failed to verify but are scheduled for retry.
- **TCP metadata fetcher** (`verify/wire.rs`, `verify/fetch_pool.rs`): Orchestrates connection timeouts and races, falling back gracefully, and natively understands the BitTorrent wire protocol.
- **Database/storage layer** (`storage/batch_writer.rs`): Protects PostgreSQL from insert amplification by bundling hundreds of inserts into single periodic transactions.
- **Metrics and health reporting** (`metrics.rs`): An allocation-free struct containing `AtomicU64` counters injected across the system to record operational telemetry.
- **Cleanup processes** (`storage/janitor.rs`): Automates database size management by continually removing terminal job states from the `verification_jobs` table based on configurable TTLs.

## 5. Backpressure and Resource Controls

Gaia is designed to remain stable under high network load through several bounding mechanisms:

- **Bounded channels**: Every `tokio::sync::mpsc` channel is bounded by configuration (e.g., `harvest_channel_capacity`). Saturated channels cause packets to be dropped with the drop count logged in metrics.
- **Semaphores**: Concurrency is limited strictly via `pipeline_limit` and `fetch_limit`. `ConnLimiter` prevents opening too many sockets to the same IP.
- **Rate limits**: A custom `RateLimiter` ensures outbound DHT queries from the Walker and Router stay below `rate_limit_per_sec` and `rate_limit_burst`.
- **Timeouts**: Socket reads and connection attempts are governed strictly by configurable durations like `tcp_timeout_secs`, `utp_timeout_secs`, and `metadata_timeout_secs` using `tokio::time::timeout`.
- **Retry limits**: Missing or unavailable peers trigger backoff retries in `verification_jobs`. Operations abort permanently upon exceeding `max_retries` or `no_metadata_max_retries`.
- **Queue capacities**: The `AnnouncePeerCache` and `PeerCache` are hard-bounded by `max_entries` and purge stale items over `ttl`.
- **Socket buffers**: UDP sockets rely on non-blocking reads; the OS network stack `SO_RCVBUF` handles momentary UDP bursts.
- **Deduplication caches**: The `Harvester` uses a size-bounded Bloom filter that rotates cleanly upon saturation to cap memory footprints.

*(Evaluation of whether these limits are currently optimal is deferred.)*

## 6. State and Persistence Model

- **Important in-memory state**: `RoutingTable` (network map), `TxTable` (outgoing query correlation), `BloomFilter` (recently seen info-hashes), `ConnLimiter` (IP backpressure state), and buffered Postgres `BatchWriter` chunks.
- **Important database entities**: `torrents` (successfully retrieved metadata), `infohash_sightings` (discovery metrics), `verification_jobs` (retry state machine), `fetch_peer_outcomes` (connection telemetry), and `metrics`.
- **Verification job states**: Governed implicitly inside `verification_jobs` via the `status` enum (`pending`, `verifying`, `verified`, `failed`, `dead`).
- **Torrent lifecycle states**: First seen (Harvested) -> Enqueued for Verify -> Verified (metadata persisted) or Failed (retried). 
- **What survives a restart**: All database tables, the cryptographic `identity.json`, the token generator `token_secret.bin`, and the local routing state `routing_table.bin`.
- **What is lost on restart**: Everything buffered in channels or the `BatchWriter` queues, the current Bloom filter history, and in-flight TCP/uTP connections.
- **How duplicate work is prevented**: Short-term: Bloom filters reject instant duplicates. Long-term: Postgres UPSERT (`ON CONFLICT`) merges duplicate sightings, and the `BatchWriter` prevents verifying already-verified jobs.

## 7. Configuration Map

Configuration is loaded from environment variables parsed inside `apps/crawler/src/config.rs`.

- **DHT/network settings** (`DhtConfig`): `dht.sybil_count`, `dht.rate_limit_per_sec`, `dht.walker_alpha`. Injected into `Router`, `Walker`, and `RateLimiter`.
- **Crawl/query rate settings** (`DhtConfig`): `dht.source_k`, `dht.source_alpha`, `dht.source_max_queries`. Used by `source_peers` for fetching DHT nodes.
- **Verification settings** (`FetchConfig`): `fetch.global_fetch_limit`, `fetch.pipeline_limit`, `fetch.race_peers`. Bound semaphores in `verify::run_pipeline`.
- **TCP and metadata timeouts** (`FetchConfig`): `fetch.tcp_timeout_secs`, `fetch.utp_timeout_secs`, `fetch.metadata_timeout_secs`. Used in `fetch_pool.rs`.
- **Concurrency limits**: Global tokio thread count configured via `worker_threads`.
- **Channel capacities**: Global `channel_capacity`, `harvest.harvest_channel_capacity`, `fetch.fresh_channel_capacity`. 
- **Database settings** (`StorageConfig`): `storage.pg_pool_max_connections`, `storage.batch_flush_interval_secs`.
- **Cleanup intervals** (`StorageConfig` / `CacheConfig`): `storage.janitor_interval_secs`, `cache.peer_cache_cleanup_interval_secs`.
- **Metrics/health settings**: `report_interval_secs`, `storage.metrics_flush_interval_secs`.

## 8. Observability

- **Counters / Gauges**: The `Metrics` struct (`apps/crawler/src/metrics.rs`) holds `AtomicU64` values for tracking critical paths (e.g., `inbound_find_node`, `sha1_mismatch`, `fetch_attempts`). Gauges track queue saturation (`verify_channel_depth`).
- **Logs**: Structured JSON/text logging is provided via the `tracing` ecosystem. Specific life-cycle markers (`trace_lifecycle!`) follow an info-hash from `discovered` to `sha1_check`.
- **Histograms**: Handled downstream by aggregating database insertions on `fetch_peer_outcomes`.
- **Database Metrics**: The application natively loops to push counter values to the `metrics` Postgres table.
- **Unmeasured areas**: OS-level UDP receive buffer overflow (e.g., `SO_RCVBUF` drops) is not directly recorded in application metrics. 

## 9. Repository Map

```text
apps/crawler/src/
  main.rs               # Application entry point and Tokio wiring.
  config.rs             # Environment variable parsing and validation.
  metrics.rs            # Atomic lock-free observability counters.
  router.rs             # UDP Packet router and DHT response generator.
  dht/
    walker.rs           # Background table-filling DHT crawler.
    routing_table.rs    # In-memory XOR node tree.
  krpc/
    scanner.rs          # Fast-path Bencode parser for UDP hot path.
    message.rs          # Full KRPC message definitions and types.
  net/
    mod.rs              # SO_REUSEPORT UDP worker loops.
  harvest/
    mod.rs              # Infohash deduplication via Bloom filters.
  verify/
    mod.rs              # Verification pipeline and job dispatch.
    fetch_pool.rs       # Connection orchestration (TCP/uTP racing).
    wire.rs             # BitTorrent wire protocol and BEP-9 exchange.
    peer_source.rs      # DHT active-peer lookup mechanism.
  storage/
    batch_writer.rs     # Buffered, asynchronous Postgres writer.
    jobs.rs             # Failure retry scheduler for infohashes.
    janitor.rs          # Background cleanup of terminal job rows.
```

## 10. Technical Lead Summary

**Five Important Facts:**
1. High UDP throughput is supported by utilizing `SO_REUSEPORT` to spread packets across multiple Tokio tasks natively, rather than relying on a single bottleneck socket.
2. The packet processor leverages a custom Bencode byte-scanner (`krpc::scanner`) to achieve zero-allocation routing for common queries (`ping`, `find_node`).
3. Outbound peer interactions attempt to aggressively lower latency by concurrently racing TCP and uTP transport connections (`try_fetch`).
4. Postgres insert performance is protected by aggressive in-memory batching (`BatchWriter`) rather than writing individual rows.
5. In-memory Bloom filters deduplicate info-hashes quickly; without this, the verification queue would easily be overwhelmed by duplicate `get_peers` requests.

**Three Areas for Deeper Inspection:**
1. **OS-Level UDP Packet Drops**: Determine if sudden query bursts overwhelm `SO_RCVBUF` and result in silent OS packet drops that the application metrics cannot observe.
2. **Database Contention on UPSERT**: Assess the efficiency of the PostgreSQL `ON CONFLICT` updates for `infohash_sightings` under maximum sustained throughput.
3. **Channel Backpressure Routing**: Evaluate whether a saturated `verify_channel` correctly applies backpressure up to the Harvester, or if it causes unnecessary CPU burn discarding packets.

**Contradictions / Discrepancies:**
- No significant contradictions were found between the implementation code, database migrations, and the expected behavior inferred from configuration keys.

**Glossary:**
- **Info-hash**: The 20-byte SHA-1 hash of a BitTorrent `info` dictionary, uniquely identifying a torrent.
- **KRPC**: The lightweight Bencoded RPC protocol that operates over UDP for DHT routing.
- **BEP-9**: The BitTorrent extension protocol for allowing peers to send metadata files to each other without needing a `.torrent` file.
- **Sybil**: Generating multiple phantom Node IDs bound to the same crawler instance to passively attract DHT queries.
- **Janitor**: The scheduled asynchronous task responsible for applying TTL limits to terminal job rows to constrain database size.

---
**Document Metadata:**
- **Git Commit Hash**: `18d1d89a6a9c9fcd14af0a89c98337f4b58c8ccb`
- **Working Tree Clean**: Yes
- **Files Inspected**: `apps/crawler/src/main.rs`, `apps/crawler/src/router.rs`, `apps/crawler/src/net/mod.rs`, `apps/crawler/src/harvest/mod.rs`, `apps/crawler/src/verify/mod.rs`, `apps/crawler/src/verify/fetch_pool.rs`, `apps/crawler/src/verify/wire.rs`, `apps/crawler/src/verify/verify.rs`, `apps/crawler/src/storage/batch_writer.rs`, `apps/crawler/src/storage/jobs.rs`, `apps/crawler/src/metrics.rs`, `apps/crawler/src/config.rs`, `apps/crawler/migrations/*.sql`
- **Existing Docs Used**: Analyzed solely from the provided Rust source tree and SQL schemas. No pre-existing external architecture docs were referenced.
- **Unanswered Questions**: What is the historical hit rate/success rate of the uTP fallback compared to TCP in production? Are metrics explicitly tracked for `janitor_deleted` confirming long-term stability?
