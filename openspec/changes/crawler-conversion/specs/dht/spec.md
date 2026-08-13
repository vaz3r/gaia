# dht spec — announce funnel observability (D65)

## Scope

Measure the passive-intake funnel so we can see where inbound announces are lost and whether the firehose grows with uptime.

## Requirements

- Actor-side counters at the announce handler (`actor.rs:1421-1453`):
  - `announces_received` — every inbound `announce_peer` query received
  - `announces_token_rejected` — token validation failed (`peer_store.rs:64`)
  - `announces_suppressed_readonly` — read-only mode gate
- Intake-side counters in `run_passive_intake` (`discovery/mod.rs:172`):
  - `announces_deduped_redis` — Redis seen-set collision
  - `announces_emitted` — sent to the fetch pipeline with a peer hint
- Rolling announce rate logged per stats tick.
- Audit checklist (D66): `node_id.json` persisted + stable across restarts; inbound DHT traffic reachable through the tunnel; routing-table size vs announce rate correlation.

## Acceptance

- Funnel counters all nonzero and internally consistent (received ≥ validated ≥ deduped + emitted).
- A 24h trace either confirms announce rate grows with uptime or identifies the loss stage (large `token_rejected` share is a smoking gun).
- Written finding (task 2.6) recommending the Phase-3 fix.
