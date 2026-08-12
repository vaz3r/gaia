## Why

The node-diversity phase (`dht-crawler-node-diversity`) raised unique discovery ~26x (to ~2-4k/hr) but torrents/day stayed at ~500-700. The bottleneck moved to **fetch yield**: ~79% of fetch attempts fail on dead/absent peers (`timeout` 37k, `empty_peers` 16k, `deadline` 5k of ~73k attempts → ~1% success).

Root cause is architectural: we are a **pure active sampler** on DHT BEP 51 samples, which are dominated by dead torrents. Bitmagnet's winning mechanism — confirmed in its source (`internal/protocol/dht/responder/responder.go`) — is being a **full DHT participant that passively ingests `announce_peer`/`get_peers` traffic**: every inbound `announce_peer` proves a hash is live *right now* and tells us exactly which peer to dial. Those hashes fetch at a far higher success rate because the announcing peer is a live dial target.

Research (mainline, librqbit-dht): no Rust crate provides both BEP 51 and an inbound-announce event API. Owning irontide in-project (renamed `gaia-*`, GPL-3.0-or-later like our project) is the only way to get the passive-intake architecture natively.

This change absorbs irontide as our own library and adds the passive-intake path on top of the existing sampler.

## What Changes

- **Phase 0 — absorb irontide as `gaia-*` workspace members**: copy the 4 irontide crates (`bencode`, `core`, `wire`, `dht`) into `vendor/`, rename packages (`gaia-*`), rewrite internal `use irontide_*` → `gaia_*`, convert to path deps. Upgrades become merges from upstream.
- **Phase 1 — inbound event stream in `gaia-dht`**: a `broadcast` channel on `DhtHandle` emitting `DhtEvent::Announced { info_hash, peer_addr }` on every validated inbound `announce_peer`, and `DhtEvent::LookedUp { info_hash, from_addr }` on inbound `get_peers`. This is bitmagnet's `PutHash` exposed as a first-class API.
- **Phase 2 — announce-first fetch path**: the crawler subscribes per instance; each announce becomes a `FetchRequest` with a **live peer dial hint**. `fetch_one` dials the hinted peer directly (skipping `get_peers` discovery); if it verifies SHA-1, we win with no discovery traffic. Sampled hashes remain the secondary source.
- **Phase 3 — stable node identity + table growth**: `own_id` persisted per instance (`node_id.json`) so the node builds DHT reputation and the announce firehose grows with uptime; `--max-nodes 8192` + `--no-restrict-ips` in compose to grow the table toward thousands.
- **Phase 4 — get_peers PutHash reuse**: deferred/cut. The announce-first path is the primary lever; reusing discovered peers for other hashes adds complexity for marginal yield and can be a follow-up.

## Capabilities

### New Capabilities

- `passive-intake`: inbound `announce_peer`/`get_peers` event stream from the DHT node, fed into the fetch pipeline with live-peer dial hints.
- `owned-dht-library`: the `gaia-*` workspace crates (absorbed irontide) maintained in-project, enabling native DHT extensions.

### Modified Capabilities

- `fetch` (previous changes): announce-first fast path dials the hinted peer before any lookup; higher-priority queue ordering for hinted requests.
- `discovery` (previous changes): BEP 51 sampling stays as secondary source; `own_id` persisted for reputation.
- `architecture` (previous changes): 4 instances each with a passive-intake subscriber; compose flags for table growth.
- `cli` (previous changes): `--max-nodes 8192`, `--no-restrict-ips` wired in compose.

## Impact

- **Expected**: announce-derived hashes fetch at far above the ~1% sampling rate (the announcing peer is live), lifting torrents/day toward bitmagnet-scale as node reputation grows.
- **Bandwidth**: announce-first fetches skip get_peers discovery for the hashes that matter most — fewer wasted lookups and dead dials per verified torrent.
- **State**: `node_id.json` per instance enables cross-restart reputation; routing-table persistence unchanged.
- **Risk**: owning irontide means a permanent in-repo dependency and GPL-3.0 code (~1.4 MB); upgrades require merging upstream changes into `vendor/gaia-*`.
