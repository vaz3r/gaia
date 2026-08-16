# Design: Bitmagnet-Style DHT Crawler Modernization

## 1. Direct Peer Resolution Architecture

### Current Problem
In current GAIA, `sampler.rs` collects `res.samples` and emits them to `hash_tx`. The metadata fetcher (`fetch/mod.rs`) calls `get_peers_seeded(info_hash, lookup_seed)`, which starts a full iterative `DhtLookup` graph walk (querying up to 64 nodes across 4 tree levels). In 69.3% of cases, the walk finds no nodes or times out, resulting in `empty_peers`.

### Bitmagnet Solution
In Bitmagnet (`internal/dhtcrawler/get_peers.go`), when node `N` announces or reports `H`, Bitmagnet sends a single direct UDP query:
```
client.GetPeers(ctx, req.node, req.infoHash)
```
If `N` has peers stored for `H`, it immediately responds with `values: [<compact peer info>]`. Bitmagnet extracts those peers and dials TCP `ut_metadata` immediately.

### GAIA Implementation
1. Add `DhtHandle::direct_get_peers(&self, target: SocketAddr, info_hash: Id20) -> Result<Vec<SocketAddr>>` to `gaia-dht`.
2. In `crawler::fetch`, when `FetchRequest` has `lookup_seed: Some(addr)`, first invoke `direct_get_peers(addr, info_hash)`.
3. If peers are returned, bypass `DhtLookup` and dial them directly.

---

## 2. Decaying Bloom Filter & Dead Hash Lifecycles

### Current Problem
`seen_bloom` is a static filter where `terminal_dead` hashes are permanently stored. A torrent that had 0 peers when first sampled is permanently ignored on all subsequent encounters.

### Bitmagnet Solution
Bitmagnet uses a `StableBloomFilter` (BoomFilters) with 10M capacity and time-based decay, where old bits are randomly decremented, providing automatic probabilistic eviction of old items.

### GAIA Implementation
1. Implement a Dual-Generational / Decaying Bloom Filter in `crawler/src/bloom.rs`:
   - Two bloom filters (`current` and `previous`), swapping generation every $T$ hours (e.g. 24h).
   - Alternatively, a counting/stable decaying bloom filter.
2. Remove the line in `sampler.rs` that permanently caches `terminal_dead` hashes into `seen_bloom`.

---

## 3. Opportunistic Inbound Node Ingestion

### Current Problem
GAIA's routing table size plateaus at ~2,240 nodes because new nodes are only learned when queries explicitly return closer nodes in responses.

### Bitmagnet Solution
`responderNodeDiscovery` intercepts every incoming message (ping, find_node, get_peers, announce_peer) and pushes `(sender_id, sender_addr)` into the discovered nodes pipeline.

### GAIA Implementation
1. In `gaia-dht/src/actor.rs`, whenever a query datagram is deserialized:
   ```rust
   if let Some(sender_ip) = msg.sender_ip {
       let sender_id = msg.body.sender_id();
       // add to discovery channel
   }
   ```
2. The actor automatically pings or inserts valid responsive nodes into its Kademlia buckets.

---

## 4. Multi-Stage Pipeline Decoupling

### Pipeline Architecture
```
[Sampler Loop] ──► infohash_rx ──► [DB Triage Batcher] ──► triage_rx
                                                                │
                                      ┌─────────────────────────┘
                                      ▼
                        [Direct KRPC Peer Resolver]
                                      │
                                      ▼
                        [TCP ut_metadata Fetch Pool]
                                      │
                                      ▼
                        [Batched Postgres Writer]
```

This prevents slow TCP handshakes from stalling the fast DHT sampling loop.
