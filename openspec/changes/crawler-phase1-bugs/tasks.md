## 1. Fetch correctness

- [x] 1.1 Hold the lookup permit across the `get_peers` stream: move `lookup_permits.acquire()` before the `let mut peers = {...}` block, keep the binding alive through the `'outer` recv loop, drop after
- [x] 1.2 Change `EARLY_ABORT_DIALS` 24 → 6
- [x] 1.3 In `dial_peers` `Ok(Err(e))` arm: classify via `FetchFailureKind::from_error`; post-handshake kinds reset the counter + set `any_handshake`; connect-level kinds increment the counter and mark the peer dead (in-process + `shared.dead_add`)
- [x] 1.4 Wrap the tracker `dial_peers` call in a batch loop over all tracker peers (chunks of `PARALLEL_DIALS`, deadline + `MAX_PEERS_PER_HASH` guarded, return on success)
- [x] 1.6 Ensure `ConnectRefused`/`ConnectionReset`/`ConnectionClosed` now also `dead_peers.record_failure` (covered by 1.3)
- [x] Unit tests: permit lifetime (concurrency bounded), classification (refused vs metadata-failed), early-abort reachable, tracker loop drains all peers

## 2. Redis dead-set expiry

- [x] 2.1 Convert `dht:dead` from set+whole-key-EXPIRE to a sorted set (`ZADD dht:dead <now> <ip>`) with per-member prune (`ZREMRANGEBYSCORE ... now-ttl`) before contains/add
- [x] 2.2 `dead_contains` reads via `ZSCORE` presence; keep the `dead_add(ip, ttl)` API surface
- [x] 2.3 Unit test: entries expire per-member; continuous adds do not resurrect old members

## 3. Dashboard rate + aggregate metrics

- [x] 3.1 Change the dashboard "Verified (per hr)" card to use `/api/admin/monitor/rates?metric=metadata_verified&range=` (windowed) instead of dividing by snapshot age
- [x] 3.2 In `stats_loop`, sum all instances' `node_count()` + actor stats for `routing_nodes`, `active_lookups`, `announce_tokens`, `pending_queries`, `announces_*`, `lookups_received`, `announced_hashes`; keep `instance_nodes` per-instance
- [x] 3.3 Verify aggregate values appear in the stats log + crawl_stats_history snapshot

## 4. Verification + commit

- [x] 4.1 `cargo test` (crawler suite against Postgres) green; clippy clean; release build
- [x] 4.2 Deploy crawler + dashboard; verify: aggregate stats logged, dead-set expires, no verified/hr regression over a clean window
- [x] 4.3 Commit
