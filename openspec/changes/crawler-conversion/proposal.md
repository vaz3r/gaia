## Why

The crawler is memory-stable (leak fixed, `6b4f95c`) and discovery is strong (~86-125k unique hashes/hr), but **verified torrents are stuck at ~150/hr** — fetch conversion is 0.15-0.25%. The failure mix on ~100k fetch attempts/hr:

| bucket | share | note |
|---|---|---|
| `empty_peers` | 56% | get_peers found nobody who knows the hash |
| `timeout` | 20% | dial/handshake timeouts |
| `other` | 11% | unclassified peer errors (~34k/15min into `peer_errors_other`) |
| `none` | 7% | no dominant failure recorded |
| `deadline` | 6% | 8s overall fetch deadline hit |

Two levers, in descending order of leverage:

1. **Passive-intake (announce) volume is nearly zero.** The announce-first path is implemented and live (DhtEvent::Announced → fetch with a live peer dial hint), but the node attracts only ~5 announced hashes/min (`hashes_announced` = 464 after ~1.5h) vs ~6,000 sampled hashes/hr. Announced hashes are live by construction and fetch at far above the sampling conversion, but the firehose is dry. If we can raise announce volume, verified/hr rises without touching the dead-hash tail. The design doc for `dht-crawler-passive-intake` predicted the firehose "grows with uptime" (reputation); we have not verified whether that is happening or what is suppressing inbound announces (token rejection, read-only flag, node reputation/ID churn, table size, port reachability).

2. **The `other` failure bucket is 11% and unclassified.** `classify_error`/`classify_peer_error` only recognize ~6 error strings; everything else lands in `other`. ~34k `peer_errors_other`/15min dwarfs most named buckets. Naming the top variants (connection_reset, handshake_failed, parse_error, protocol_error) tells us whether there's a fixable cause (e.g., a handshake bug, a parse bug on a common client) or inherent noise.

This change is measurement-first: instrument and classify before changing behavior. We already learned (min_seen=2 collapse, announce drain at ~1.9% cut) that unmeasured selectivity changes are risky.

## What Changes

- **Phase 1 — failure classification**: extend `classify_error`/`classify_peer_error` with granular buckets (connection_reset, handshake_failed, parse_error, protocol_error, connection_closed, peer_abort), persist the dominant failure reason as before, and add a periodic roll-up so the dashboard shows the new buckets. No behavior change — pure observability, so the classification is validated against live traffic first.
- **Phase 2 — announce volume audit**: instrument the announce path with per-stage counters (announces received, token-rejected, suppressed-by-read-only, deduped-by-Redis, emitted) and a rolling rate, so we can see *where* announces are lost and whether the firehose is actually growing with uptime. Audit whether the node presents a stable identity/reputation (node_id.json persistence is in place), whether tokens are being rejected en masse, and whether inbound announces are reachable at all.
- **Phase 3 — announce yield fix** (conditional on Phase 2 findings): the highest-leverage intervention identified by the audit — likely node-reputation/identity improvements, token handling, or a second ingestion path (e.g., subscribing to announced peers via `get_peers` responses the node already serves). This is deliberately deferred until the audit says what is actually suppressing the firehose.
- **Phase 4 — selective conversion tuning** (conditional, after 1-3): only if the `other` classification reveals a fixable subclass or the announce path plateaus below target, revisit the liveness gate / fetch selectivity with measured thresholds.

## Capabilities

### New Capabilities

- `fetch-failure-taxonomy`: granular, persistent classification of peer-fetch failures (connection_reset / handshake_failed / parse_error / protocol_error / connection_closed / peer_abort) with dashboard roll-up, so the 11% `other` bucket is explained before any behavioral change.
- `announce-volume-observability`: per-stage counters and a rolling rate on the passive-intake path (received → validated → deduped → emitted) plus the audit of what suppresses inbound announces.

### Modified Capabilities

- `fetch` (previous changes): no behavioral change in Phase 1-2; error classification becomes the richer taxonomy.
- `discovery` (previous changes): `run_passive_intake` gains stage counters; any Phase-3 fix builds on the existing event stream.
- `architecture` (previous changes): unchanged (4 instances, compose config); the announce audit may touch `own_id`/node-identity handling.

## Impact

- **Expected**: classify the 11% `other` bucket (cheap, zero-risk); identify why the announce firehose is dry and, if a fix is found, lift verified/hr well above the current ~150/hr by adding a live-by-construction hash stream alongside sampling.
- **Bandwidth**: unchanged in Phase 1-2 (observability only). A Phase-3 announce fix adds negligible traffic (announces are already inbound).
- **State**: no schema change in Phase 1-2 (dominant failure reason already persisted); a Phase-3 change may add counters only.
- **Risk**: low for Phases 1-2 (measurement-first, no behavior change). Phase 3 is gated on findings.
