## Why

The discovery change (`dht-crawler-discovery`) lifted unique discovery ~3x (6.9/s → ~20/s) and pushed sustained throughput to ~80/hr, but the verify rate is now the binding constraint. Measured over the last hour on the NAT host:

- Verify rate ~0.65% (67 verified / ~9,700 fetched).
- Failure mix: **timeout 57%**, empty_peers 27%, other 15%, deadline ~1%.
- The fetch pool is **90% idle**: in-flight ~37 of 512 slots, capacity ~42 fetches/s vs ~10/s used.

The timeout failures dominate: `get_peers` finds peers, but with only 16 parallel dials, 50 peers/hash max, a 3s connect timeout, 7s per-peer timeout, and a 12s deadline, we simply don't try enough peers or give slow-but-live ones enough time to answer. Because the pool has enormous headroom, we can afford to make each fetch far more thorough at near-zero throughput cost.

## What Changes

- `PARALLEL_DIALS` 16 → **32** (dial twice as many peers concurrently per hash).
- `MAX_PEERS_PER_HASH` 50 → **100** (try twice as many distinct peers before giving up).
- `FETCH_DEADLINE` 12s → **20s** (more wall-clock time to iterate peer batches).
- `FETCH_TIMEOUT` 7s → **10s** (per-peer connect+fetch window).
- `CONNECT_TIMEOUT` 3s → **5s** (slow TCP connects on NAT get a chance).
- `EARLY_ABORT_DIALS` 24 → **64** (proportionate to the higher parallel-dial count, so a hash is still abandoned fast when all dials fail, but not prematurely).

## Capabilities

### Modified Capabilities

- `fetch` (previous change): per-hash fetch budget and dial concurrency increased; slow-peer connect window widened.

## Impact

- **Code**: `fetch/mod.rs` and `fetch/wire.rs` constant changes only; no new deps, no schema change.
- **Performance (expected)**: verify rate up meaningfully (more peers tried per hash, slow peers reachable), throughput bounded only by the pool (which has headroom). Empty-peer and other failures are unchanged by this tuning.
- **Operations**: each fetch holds a slot a bit longer (20s deadline) but the pool is far from saturated, so no throughput loss.
