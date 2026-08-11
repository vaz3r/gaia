## Why

The optimization change (`dht-crawler-optimization`) unblocked the fetch pool, kept all torrents, and fixed discovery, but a measured 10-minute run on the NAT host exposed the next layer of limits:

1. **The fetch pool runs at ~17% utilization.** Observed spawn rate was 4.3 fetches/sec against a theoretical ~25/sec (512 slots ÷ 20s deadline). The limiter is gating before dialing: `lookup_concurrency` (64) bounds how many `get_peers` lookups may be *started* concurrently, and the DHT actor's shared query budget is consumed by the 32 sampler loops. ~3,900 of 6,468 unique hashes sat unfetched in the queue.
2. **Verify rate is 0.58%** (15 verified / 2,595 fetched). BEP 51 returns the DHT long tail (dead/rare/adult content); most sampled hashes have no serving peers. Of 2,186 failures: timeout 42%, deadline 21%, empty_peers 9%, other 9%.
3. **Wasted work on dead hashes.** 9,148 peer connect timeouts across 2,595 fetches means the same dead IPs are re-dialed repeatedly. 21% of hashes burn the full 20s deadline.
4. **Retry policy is too slow for the long tail.** Backoff base is 5 minutes; a hash that fails with `empty_peers` (no peers at all right now) can't be retried for 5m even though its swarm may appear within a minute.
5. **Discovery breadth is capped at one node.** A single DHT node/port/state-dir samples the keyspace through one routing table; `announced_hashes` (5 in 10 min) confirmed passive announces are negligible on NAT, so the only way to raise discovery breadth is more active samplers.

This change fixes all five across four tiers: unblock/speed the pipeline (A), raise the verify rate (B), widen discovery with multiple instances + routing warmup (C), and tighten retry robustness (D).

## What Changes

- **Tier A — pipeline**: raise `--lookup-concurrency` default 64→256 (aggressive 256→512) and `--qps` 5000→8000; instrument stats with in-flight fetch count and queue depth; early-abort a hash whose first ~24 dials all fail (connect timeout/refused, zero successful handshakes) instead of burning the full deadline; shorten `FETCH_DEADLINE` 20s→12s.
- **Tier B — verify rate**: keep `--min-seen 2`; add an in-run dead-peer cache (short-TTL, e.g. skip an IP for ~10 min after repeated connect failures) so the same unreachable peers are not re-dialed for every hash.
- **Tier C — discovery**: multi-instance crawling — a `--instances N` option runs N DHT nodes on distinct UDP ports and state dirs, each with its own sampler, all feeding one SQLite DB through a shared writer; plus a routing-table warmup phase that issues `get_peers` on random targets early on to grow the table faster.
- **Tier D — retry**: shorten backoff base 5m→60s (cap stays 6h), and give `empty_peers` failures a shorter retry window (they may gain peers quickly).

## Capabilities

### New Capabilities

- `pipeline`: concurrency-unblocked metadata fetch pool with early dead-hash abort, in-flight/queue instrumentation, and a dead-peer cache.
- `multi-instance`: run several DHT nodes + samplers against one database to multiply discovery breadth.
- `routing-warmup`: targeted `get_peers` on random keyspace early in the run to grow the routing table faster.

### Modified Capabilities

- `discovery` (previous change): gains routing warmup and the multi-instance plumbing.
- `fetch` (previous change): gains early-abort, dead-peer cache, and the new deadlines/budgets.
- `storage` (previous change): backoff policy changes (base 60s, `empty_peers`-aware).
- `architecture` (previous change): `run`/`crawler` gains instance orchestration.

## Impact

- **Code**: `crawler.rs` gains instance orchestration; `fetch/mod.rs` gains early-abort + dead-peer cache + instrumentation; `discovery/mod.rs` gains warmup + multi-instance config; `stats.rs` gains in-flight/queue counters; `storage/model.rs` gains the new backoff policy.
- **Dependencies**: none added.
- **Operations**: multi-instance consumes N UDP ports and multiplies sampling/fetch load; the `--aggressive` preset scales budgets accordingly. One SQLite DB shared by N instances relies on WAL + a single writer.
- **Performance (expected)**: fetch utilization toward saturation (~4.3→25 fetches/s), verify rate up from 0.58% (min-seen 2 + dead-peer cache + early abort), discovery breadth multiplied by N instances, dead hashes retried within ~1 min.
