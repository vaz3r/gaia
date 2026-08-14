## Why

Measured on remote-dev after the bandwidth/layout change (`crawler-bandwidth-and-layout`, committed 09f0d69): bandwidth is **~1.41 MB/s with only ~468 torrents/hr** — dominated by **fetch TCP traffic** (~1.0 MB/s download, 83 fetches/s × ~12KB each). The failure data is damning:

- **175,886 fetches attempted → 276 verified (0.16% success)**.
- `connect_timeout` (peers exist in get_peers but never answer TCP) and `empty_peers` (no live values) dominate; `peer_errors_other` and `no_ut_metadata` also burn handshakes.
- `min_seen=1` means we fetch **nearly every unique hash** (178k unique → 175k fetches), and `fetch/wire.rs` requests **every metadata piece upfront**, so a stalled/honeypot peer costs us downloading a large chunk of metadata we'll never assemble.

The bandwidth is pure waste: fetching dead hashes and downloading partial metadata from peers that stall. Verification (~468/hr) is unchanged whether we make 175k attempts or ~half that, because the live hashes are the same.

## What Changes

- **Phase A — fetch selectivity (measured, then reverted)**:
  - `--min-seen` 1 → 2 was tried to gate fetches on 2+ node sightings. **Reverted**: the sampler counts sightings per loop (not fleet-wide), so it became ~190x stricter than intended — unique discovery collapsed 312k/hr → 3.7k/hr and verified fell to ~20/hr.
- **Phase B — cap per-fetch download (kept)**:
  - `fetch/wire.rs` now requests metadata pieces **incrementally** (piece 0 first, next only after data arrives) instead of requesting every piece upfront. A stalled peer costs ~1-2 pieces, not the whole metadata. Measured per-fetch download cut ~23% (12.6KB → 9.6KB).

## Outcome

Measured on remote-dev (steady state): ~285-292/hr verified at ~0.81 MB/s download / 1.24 MB/s total (baseline 1.41 MB/s / ~468/hr). The incremental-piece cap is a real per-fetch win, but bandwidth remains dominated by fetch volume: ~84 fetches/s at 0.16% success, with ~91k TCP handshakes to dead peers that get_peers reports. Raising fetch success (better liveness signals) is the remaining lever.
