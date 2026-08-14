## Context

Building on `crawler-bandwidth-and-layout` (committed 09f0d69). RESPONSE_K + tighter dials only cut bandwidth ~10% because the real cost is fetch TCP traffic: 175,886 attempts at 0.16% success, ~12KB downloaded per attempt. The waste is (1) fetching nearly every unique hash under `min_seen=1`, and (2) requesting all metadata pieces upfront so stalled peers cost a large partial download. Adds decisions D54-D56.

## Goals / Non-Goals

**Goals:**
- Halve fetch volume by requiring 2+ node sightings (min_seen=2 default) while keeping the corroborated live tail.
- Cap per-fetch download by requesting metadata pieces incrementally, so a stalled peer costs ≤2 pieces.
- Land at ~same verified/hr at ~half the bandwidth.

**Non-Goals:**
- No content filtering.
- No change to passive announce intake or the announce-hint fast path (announced hashes stay exempt from min_seen).
- No change to `--scale` (10), instance count (4), or Docker/Gluetun/Redis architecture.

## Decisions

### D54 — `--min-seen` default 1 → 2 (REVERTED after measurement)
Sampled hashes were to be emitted only after 2+ distinct BEP 51 responses reported them.
- *Measured outcome:* the sampler counts sightings **per loop**, not fleet-wide, so "2 sightings" became ~190x stricter than 2 distinct fleet-wide nodes. Unique discovery collapsed (312k/hr → ~3.7k/hr) and verified torrents fell to ~20/hr. This over-restriction is a sampler-level artifact, not a real corroboration gate.
- *Decision:* reverted to `--min-seen 1`. A real fleet-wide corroboration gate would need a Redis occurrence counter per sampled hash — at 250k unique/hr that is too many round-trips on the hot path. Deferred.

### D55 — Incremental metadata piece requests (KEPT)
`fetch/wire.rs` requests piece 0 first, then the next piece only after a piece's data arrives, instead of requesting every piece upfront.
- *Measured outcome:* per-fetch download dropped ~23% (12.6KB → ~9.6KB). Kept.
- *Trade-off:* one extra round-trip per live metadata fetch; negligible vs the 3s dead-peer timeout.

### D56 — Keep hinted (announce) hashes exempt
Announced hashes (carrying a live peer hint) keep bypassing both min_seen and the get_peers lookup, dialing their hinted peer directly.
- *Rationale:* an inbound announce_peer is a stronger liveness signal than two BEP 51 sightings; exempting them preserves the highest-success path.
- *Trade-off:* none — hinted hashes are the most likely to verify.

## Risks / Trade-offs

- **Incremental pieces add a round-trip**: only on live fetches; amortized against the dead-peer savings (~23% per-fetch download cut).
- **min_seen=2 rejected**: per-loop sighting counting makes it ~190x stricter than intended; a fleet-wide gate would cost too many Redis round-trips at this discovery rate.
- **Bandwidth remains bursty**: benchmark over ≥15 min; short windows are noisy.
- **Known bandwidth floor**: with 0.16% fetch success, bandwidth tracks fetch volume (~84 fetches/s) — TCP handshakes to ~91k dead peers reported by get_peers dominate. Reducing it further requires raising fetch success (better peer liveness signals) or reducing discovery, both beyond this change.
