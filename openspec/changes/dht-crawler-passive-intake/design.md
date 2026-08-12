## Context

Building on `dht-crawler-node-diversity` (uncommitted). Unique discovery is fixed (~2-4k/hr, 26x baseline) but torrents/day plateaued at ~500-700 because ~79% of fetch attempts hit dead/absent peers — a pure active-sampler architecture on BEP 51 samples. Bitmagnet wins by passively ingesting `announce_peer`/`get_peers` as a full DHT participant. No stock Rust crate offers BEP 51 + an inbound-announce event API, so we absorb irontide as our own `gaia-*` library and build the passive-intake path. Adds decisions D39-D44.

## Goals / Non-Goals

**Goals:**
- Absorb irontide as maintained in-project `gaia-*` crates (enables native DHT extensions, no `[patch]` hacks).
- Surface inbound `announce_peer`/`get_peers` as a first-class event stream on `DhtHandle`.
- Add an announce-first fetch path that dials the live announcing peer directly, bypassing discovery for the hashes most likely to verify.
- Give the node a stable identity so the announce firehose grows with uptime.
- Raise torrents/day well past the ~500-700 plateau while keeping bandwidth low.

**Non-Goals:**
- No content filtering.
- No change to Docker/Gluetun architecture or Redis coordination.
- Phase 4 (get_peers PutHash peer reuse) deferred — the announce-first path is the primary lever.

## Decisions

### D39 — Absorb irontide as `gaia-*` workspace members
Copy `irontide-bencode/core/wire/dht` into `vendor/gaia-*`, rename packages, rewrite `use irontide_*` → `gaia_*`, convert to path deps, add to workspace members.
- *Rationale:* no other Rust crate provides BEP 51 + inbound-announce events; owning the library lets us add the event stream natively. GPL-3.0-or-later matches our license, so absorption is legally clean. `[patch.crates-io]` was rejected earlier by the user as maintenance-hostile.
- *Trade-off:* ~1.4 MB of GPL code in-repo and a permanent maintenance commitment (upgrades = merges from upstream).

### D40 — Inbound event stream on DhtHandle
`gaia-dht` gets a `broadcast::Sender<DhtEvent>` in the actor; `handle_query` emits `Announced { info_hash, peer_addr }` after a validated `announce_peer`, and `LookedUp { info_hash, from_addr }` on inbound `get_peers`. `DhtHandle::subscribe()` returns a receiver.
- *Rationale:* mirrors bitmagnet's responder `PutHash`, but as a real API. Announce events carry the live peer address, which is the key to high fetch success.
- *Trade-off:* broadcast lag drops the oldest event if the consumer is slower than the actor; consumers use a cheap bounded pipeline.

### D41 — Announce-first fetch path
Announce events become `FetchRequest { hash, occurrences: 1, peer_hint: Some(peer) }` on a high-priority channel. `fetch_one` dials the hinted peer directly (after blocklist/dead-peer checks); a SHA-1-verified result returns immediately. Sampled hashes stay the secondary source.
- *Rationale:* the announcing peer is live by construction; dialing it directly skips `get_peers` discovery and attacks the timeout/empty_peers failure wall at its root.
- *Trade-off:* a hint that fails still falls back to the normal get_peers path, so no coverage is lost.

### D42 — Stable node identity
Persist `own_id` per instance as `node_id.json`; pass `DhtConfig::own_id: Some(...)`. A stable ID builds DHT reputation so peers route `announce_peer`/`get_peers` to us over time — the passive firehose grows with uptime (bitmagnet's model).
- *Rationale:* a fresh random ID per restart never accumulates announce traffic; stability is what turns passive intake from a trickle into a stream.
- *Trade-off:* BEP 42 regeneration on IP change still applies after consensus; the persisted ID anchors startup reputation.

### D43 — Table growth for reputation
`--max-nodes 8192` and `--no-restrict-ips` in compose; keep 100ms growers. A larger table increases both BEP 51 sampling diversity and the announce volume we're trusted with.
- *Rationale:* bitmagnet operates at thousands of nodes; our ~640-node plateau limits both discovery breadth and inbound traffic.
- *Trade-off:* more routing nodes = more DHT query load; bounded by `--qps` and measured bandwidth.

### D44 — get_peers PutHash reuse deferred
Not implementing hash→peers reuse from our own lookups in this change.
- *Rationale:* the announce-first path is the primary lever; peer reuse adds complexity for marginal additional yield and can be a follow-up once announce volume is measured.

## Risks / Trade-offs

- **Owning the DHT**: permanent in-repo dependency on absorbed GPL code; upgrades are manual merges. Accepted in exchange for the passive-intake capability no other Rust crate offers.
- **Broadcast lag**: a slow consumer drops events (bounded by `Lagged`), losing announce signals. The crawler's pipeline is cheap (hash + addr into a channel), so this is unlikely.
- **Hint failure fallback**: hinted dials that fail fall back to get_peers, so no coverage is lost, only a little time.
- **Reputation ramp**: the announce firehose grows over hours/days of stable operation; initial measurements may understate the steady-state contribution.
