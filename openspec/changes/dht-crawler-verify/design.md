## Context

Building on `dht-crawler-discovery` (committed). Discovery is now ~3x; the verify rate (0.65%) is the ceiling. The fetch pool is 90% idle, so making each fetch more thorough costs nothing in throughput. This change adds decision D23.

## Goals / Non-Goals

**Goals:**
- Raise the fraction of fetched hashes that verify by trying more peers per hash and giving slow-but-live peers more time.
- Keep the pool's fast-fail behavior (early abort, recv timeout) intact so dead hashes still free slots quickly.

**Non-Goals:**
- No change to empty_peers (27% of failures) — those hashes have no peers to dial; addressed by retry timing, not dial budget.
- No schema, dependency, or architecture changes.

## Decisions

### D23 — More thorough per-hash fetches
With the pool at ~10% utilization, each fetch SHALL dial 32 peers concurrently (up from 16), try up to 100 distinct peers (up from 50), run for up to 20s (up from 12s), give each peer up to 10s (up from 7s) and each TCP connect up to 5s (up from 3s). `EARLY_ABORT_DIALS` rises to 64 to stay proportionate.
- *Rationale:* the dominant failure is `timeout` — peers exist but are unreachable within the current tight window. More concurrent dials and a longer window directly attack that; the pool's headroom means no throughput penalty.
- *Alternatives considered:* raising `--concurrency` (512) — unnecessary, the pool is idle. Raising min_seen to 3 — filters junk but also delays rare releases; not this change.
- *Trade-off:* a dead hash can now occupy a slot for up to 20s, but the early-abort (64 failed dials with no handshake) still cuts most of those short, and the pool is far from saturated.

## Risks / Trade-offs

- **Longer per-fetch time** → slot held longer; acceptable while the pool has headroom. If the pool later saturates, `FETCH_DEADLINE` can be lowered.
- **More concurrent TCP dials per hash** → more outbound SYNs; modest, standard for crawlers.

## Migration Plan

No schema change; constants only. Restart PM2 to pick up the new binary. Rollback: restore the previous constants.
