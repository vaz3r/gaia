# Peer Infohash Discovery: Protocol Analysis & Extraction Techniques

## Executive Summary
A common operational question in DHT crawler architecture is:
> *Can we directly ask a known active BitTorrent peer to list all the infohashes they are currently seeding or downloading?*

The short answer is **no, not via a single RPC command**, because the core BitTorrent wire protocol was explicitly designed without a `"list_torrents"` request for peer privacy reasons. The wire handshake (`<pstrlen><pstr><reserved><info_hash><peer_id>`) requires the initiator to know and supply the specific 20-byte target `info_hash` before any payload packets can be exchanged.

However, using a combination of **protocol exploitation**, **DHT proximity targeting**, and **database correlation**, we can uncover a significant portion of what these stable peers are seeding. This document details the implementation specifications for **Techniques A, B, C, and D**.

---

## Technique A: BEP 11 Peer Exchange (PEX) Chaining

### Overview
BEP 11 defines Peer Exchange (`ut_pex`), an extension protocol (negotiated via BEP 10) that allows peers within a swarm to gossip IP:port endpoints of other swarm participants.

### Mechanism
While `ut_pex` does not tell you what *other* swarms a peer belongs to, it enables **Swarm Graph Traversal**:
1. When our crawler successfully handshakes with a stable peer on `infohash_1`, the peer sends a `ut_pex` message containing added peers (`added`) and flags (`added.f` indicating seed vs leecher status).
2. Many of these added peers belong to dedicated high-bandwidth seedboxes or multi-torrent automation setups (e.g. Autobrr, Sonarr).
3. By cross-referencing incoming PEX peer sets across multiple torrents, we can cluster IP subnets and autonomous systems (ASNs) that seed the same release groups.

### Implementation Architecture
```
[Crawler Engine] 
       │ (TCP/uTP wire connection on Known Infohash)
       ▼
[Target Stable Peer] ─── sends ut_pex payload ───► [PEX Buffer]
                                                          │
   ┌──────────────────────────────────────────────────────┴────────┐
   ▼                                                               ▼
[Seed/Leech Flag Extraction]                       [Cross-Swarm Sighting Correlator]
(Identifies persistent seeds)                      (Indexes shared peers across releases)
```

---

## Technique B: DHT get_peers and find_node Proximity Targeting

### Overview
In the Kademlia DHT (BEP 5), each node is identified by a 160-bit Node ID and maintains a routing table of neighbors ordered by XOR distance ($d(x, y) = x \oplus y$).

When a client seeds or downloads a torrent:
1. It calculates $d(\text{NodeId}, \text{InfoHash})$.
2. It sends `announce_peer` or `get_peers` RPCs to the $K$ closest nodes to that `InfoHash`.
3. If a stable peer is acting as a long-lived DHT routing node (e.g. running on port 6881), **it frequently acts as a storage bucket for infohashes located near its own Node ID**.

### Exploitation Strategy: "Sybil Clamping"
To extract the infohashes that a specific stable peer is managing or tracking:
1. **Target Identification**: Extract the stable peer's 160-bit DHT Node ID via a KRPC `ping` query.
2. **Virtual Neighbor Generation**: Spawn virtual Sybil nodes whose Node IDs are mathematically adjacent ($XOR \approx 0$) to the target peer's Node ID.
3. **Continuous get_peers Probing**: Periodically query the target peer for tokens and peers on infohashes with the same prefix bits.
4. **Announce Sniffing**: Because the peer believes our Sybil nodes are its closest Kademlia neighbors, it forwards incoming `announce_peer` messages to us, revealing what other nodes (and itself) are announcing in that keyspace segment.

---

## Technique C: BEP 52 / BitTorrent v2 & Extension Handshake Probing

### Overview
BEP 10 (Extension Protocol) establishes an extensible framing layer over the standard BitTorrent wire protocol. When establishing a connection, both clients exchange an extension dictionary in JSON-like bencode:
```bencode
d1:m d11:ut_metadata i2e 6:ut_pex i1e 10:ut_comment i6ee 1:v 13:μTorrent 3.6e
```

### Potential Vectors
1. **BEP 52 (BitTorrent v2 Hash Trees)**:
   - In v2 swarms, payloads are indexed by SHA-256 Merkle root hashes (`file_tree`).
   - Clients supporting hybrid v1/v2 swarms advertise capability flags in the extension handshake. If a peer accepts multi-hash discovery, root hashes can be enumerated.
2. **Private Extension Fingerprinting**:
   - Certain modified clients (e.g. BitComet, Xunlei) advertise proprietary extensions (`bc_tagged`, `channel_list`) during handshake.
   - If probed with custom extension handshake packets, non-compliant clients can leak recent download queues or internal channel tags.
3. **Zero-Byte Metadata Probing**:
   - By sending an extension message requesting piece `0` of an infohash prefix without completing the full file payload, we can probe whether a peer immediately serves metadata or drops connection, testing candidate infohashes at high speed.

---

## Technique D: Reverse DHT Sighting Correlation (Database Intelligence)

### Overview
This is the most reliable, production-ready technique and **can be queried immediately from GAIA's existing dataset**.

GAIA continuously harvests:
- Inbound DHT queries (`inbound_announce_peer`, `inbound_get_peers`).
- Raw peer IP addresses associated with infohashes during wire verification.

### Database Query Implementation
In PostgreSQL, the relationship between IP endpoints and discovered infohashes is tracked in `stable_peers` and `infohash_sightings`:

```sql
-- Find all torrents discovered or announced by a specific stable peer IP:
SELECT 
    encode(t.infohash, 'hex') AS infohash,
    t.name,
    t.total_size,
    t.file_count,
    s.last_seen,
    sp.metadata_provided_count
FROM stable_peers sp
JOIN infohash_sightings s ON s.peer_ip = sp.ip
JOIN torrents t ON t.infohash = s.infohash
WHERE sp.ip = '79.117.28.65'
ORDER BY s.last_seen DESC
LIMIT 50;
```

### Advantages of Technique D
- **Zero Protocol Overhead**: Requires no extra network packets or risk of getting rate-limited/blacklisted by remote peers.
- **Historical Depth**: Reconstructs everything a peer has seeded over days or weeks of crawler uptime.
- **Immediate Utility**: Can be surfaced directly in the dashboard UI as a **"Peer Seeded Torrents"** sub-view when clicking on a peer in the Stable Peers Explorer!
