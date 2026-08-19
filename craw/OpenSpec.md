# OpenSpec: Crawler v2 (High-Performance DHT Sybil Crawler)

## 1. Overview and Design Philosophy

Crawler v2 is a high-performance BitTorrent DHT crawler and metadata verifier designed to comfortably clear 7,000+ verified infohashes per hour on modest hardware. 

The core philosophy revolves around three principles:
1. **Inbound-Driven Discovery**: Infohashes are harvested purely from incoming `get_peers` and `announce_peer` queries. Discovery volume is driven by an *active* self-lookup walk engine that inserts our phantom Sybil IDs into real nodes' routing tables.
2. **Decoupled Pipelines**: Discovery and Verification are completely separate. The discovery engine yields raw infohashes and feeds a bounded queue; the verification engine races connections to fetch metadata. One never blocks the other.
3. **Batched & Bounded**: Hot paths use multi-socket UDP with `SO_REUSEPORT`, kernel-level batching (`recvmmsg`), zero-copy KRPC decoding, and batch database writes.

### Critical Protocol Realities Addressed
*   **BEP42 vs. Keyspace Coverage (The Phase 2 Strategy)**: BEP42 ties node IDs to external IPs. With a single IP, all compliant IDs share a prefix and cannot be distributed across the keyspace. To resolve this and hit 7,000/hr, Phase 2 implements a **Hybrid approach (Option C)**: We run BEP42-compliant IDs (using our explicitly resolved external IP) alongside random, non-BEP42 IDs spread broadly across the keyspace. We will actively measure the inbound `get_peers` rate and pruning rate for both pools. If modern nodes aggressively prune the non-BEP42 IDs and throughput drops, we will pivot to provisioning multiple public source IPs (Option A) to achieve genuine keyspace distribution.
*   **KRPC Completeness**: Proper handling of `ping` (to avoid being pruned) and cryptographic `token` generation (to receive subsequent `announce_peer` messages) are mandatory.

---

## 2. Architecture

```mermaid
flowchart TD
    subgraph DHT Layer
        Walk[Self-Lookup Walker\nActive outbound find_node] --> Sybil
        Sybil[Sybil Swarm\nHybrid: BEP42 + Random Keyspace IDs] 
        Sybil --> Net[Multi-Socket UDP I/O\nSO_REUSEPORT, recvmmsg]
    end

    Net --> Router[KRPC Message Router & Tx State]
    
    subgraph KRPC State
        Router <--> TxTable[Shared Transaction Table\nTxID, Retries, Timeouts]
        Router <--> TokenGen[Token Generator\nHMAC(IP + Secret + Time)]
    end

    subgraph Harvest Engine
        Router --> PING[ping responder]
        Router --> GP[get_peers responder\nReturns Token + Extracts IH]
        Router --> AP[announce_peer responder\nExtracts IH]
        Router --> FN[find_node responder\nAnswers with phantom IDs]
        FN -.-> Sybil
        PING -.-> Sybil
    end

    GP --> Bloom[Bloom Filter Dedupe\n2-filter rotation]
    AP --> Bloom
    
    Bloom --> Queue[Bounded MPSC Channel\nBackpressure Boundary]

    subgraph Storage: Discovery
        Queue --> DiscWriter[Discovery Batch Writer\nINSERT ON CONFLICT]
    end

    subgraph Verification Pipeline
        Queue --> VQ[Verification Job Queue]
        VQ --> Sourcing[Peer Sourcing\nActive get_peers fanout]
        Sourcing --> Pool[Fetch Pool\nRace N peers, global/local sems]
        Pool --> Wire[TCP-First Wire Client\nBEP3 + BEP10 + BEP9]
        Wire --> Verify{Exact Raw Bytes\nSHA1 == Infohash?}
    end

    subgraph Storage: Metadata
        Verify -- Pass --> MetaWriter[Metadata Batch Writer]
        Verify -- Fail --> Retry[Backoff Retry Queue\n1m -> 5m -> 30m -> drop]
    end
```

---

## 3. Module Breakdown (Rust)

To keep the project maintainable, the crawler is split into targeted modules (approx. 3000-4000 LOC total):

*   **`src/krpc/`**
    *   `codec.rs`: Zero-copy bencode codec over `BytesMut`. Fuzz-tested against malformed packets to prevent worker panics.
    *   `message.rs`: KRPC message definitions.
    *   `tx_state.rs`: **Shared Transaction table** mapping outgoing queries (TxID $\rightarrow$ query type, target, timestamp, callback). Handles timeouts and retry policies. *Must be shared across all `SO_REUSEPORT` sockets*, as responses may arrive on any socket.
    *   `token.rs`: **Token generation** via HMAC(requesting IP + secret + time window) for `get_peers` responses. Includes time window overlap support for smooth secret rotation.
*   **`src/dht/`**
    *   `node_id.rs`: Generation of **BEP42-compliant** Node IDs (tied to external IP) and random keyspace IDs.
    *   `routing_table.rs`: K-buckets and Sybil identities.
    *   `walker.rs`: The active self-lookup engine ($\alpha$-bounded `find_node` queries).
*   **`src/net/`**
    *   `SO_REUSEPORT` socket management (1 per core).
    *   `recvmmsg`/`sendmmsg` buffer pools and rate-limiters (token bucket) per target IP.
*   **`src/harvest/`**
    *   Dual harvest extraction logic (`get_peers` + `announce_peer`).
    *   Rotating Bloom filter deduplication.
*   **`src/verify/`**
    *   `peer_source.rs`: Active fanout to find more peers for verification.
    *   `wire.rs`: BEP3, BEP10, and BEP9 metadata fetch extension protocols. **TCP-only initially.** Extracts `metadata_size` from BEP10 handshake. Note: If uTP/MSE are added later, significantly more time must be allocated as hand-rolling is complex.
    *   `verify.rs`: Slices the **exact raw bytes** of the received info-dictionary for SHA1 verification to prevent bencode key-ordering mismatches.
    *   `fetch_pool.rs`: 2-level semaphore racing fetch logic with layered timeouts.
*   **`src/storage/`**
    *   `discovery_writer.rs`: 500ms multi-row flush using staging tables or `INSERT ... ON CONFLICT` (since pure `COPY` lacks upsert semantics).
    *   `metadata_writer.rs`: Verified metadata flusher.
    *   `retry_queue.rs`: Exponential backoff state.
*   **`src/main.rs`**
    *   Wiring, async runtime setup, metrics (queue depths, inbound query rates, token issuance, prune rate, keyspace prefix yields), and configuration.

---

## 4. Implementation Phases & Milestones

### Phase 1: Foundation (KRPC & Net)
- Initialize the cargo workspace.
- Build the zero-copy bencode decoder/encoder (`src/krpc`). Implement **fuzz testing** for malformed KRPC payload handling.
- Build the UDP socket layer (`src/net`) using `SO_REUSEPORT` and batching.
- Implement the KRPC router and **Shared Transaction Table** to track outbound TxIDs across all sockets.
- Implement the `ping` responder.
- Implement HMAC token generation for `get_peers` responses.
- **M1 Milestone:** KRPC codec + UDP socket layer can parse and respond to `ping`, `find_node`, and `get_peers` with valid KRPC (including tokens). Robust against fuzzing. Sybil swarm is not yet required; can respond with empty/self nodes.

### Phase 2: Sybil DHT & Harvest Engine
- Identify external IP. Implement **Hybrid** Node ID generation (BEP42 + random) to solve the keyspace coverage paradox.
- Implement K-bucket routing tables.
- Implement the Self-Lookup Walk Engine to inject IDs. Add rate limiting per source IP.
- Add the dual-harvest extraction logic and rotating Bloom filter deduplication.
- Measure inbound query yield by ID type (BEP42 vs random) to determine if Option A (multiple IPs) is required.
- **M2 Milestone:** Harvester collects both `get_peers` and `announce_peer` infohashes, dedupes via rotating Bloom filter, and batches to storage/metrics. Measured inbound rate is meaningful. *No stdout printing.*

### Phase 3: The Verification Pipeline
#### Phase 3A: TCP-Only Vertical Slice
- Build the BEP3 handshake and BEP10 extension handshake (parsing `metadata_size`).
- Build BEP9 metadata piece request/reassembly over **TCP only**.
- Implement SHA1 validation against the **exact raw bytes** of the payload.
- Implement Peer Sourcing (active `get_peers` fanout).
- Build the Fetch Pool: async racing with 2-level semaphores and layered timeouts.
- **M3 Milestone:** TCP-only verification works end-to-end on a local test swarm (or live swarm).

#### Phase 3B: Protocol Coverage Expansion (Optional/Deferred)
- *If TCP success rates are insufficient to reach the 7k/hr target*:
- Add uTP (BEP29) and/or MSE/PE support (allocate heavy dev time or evaluate existing crates).

### Phase 4: Storage, Observability & Finalization
- Finalize concrete database schemas and migrations.
- Implement batch upsert semantics (`INSERT ON CONFLICT` or staging tables).
- Implement the exponential backoff retry queue for failed fetches.
- Configure resource limits.
- Solidify observability: queue depths, verification attempts vs. successes vs. failures.
- **M4 Milestone:** Production deployment reaching the measured 7,000+ verified infohashes/hour target.
