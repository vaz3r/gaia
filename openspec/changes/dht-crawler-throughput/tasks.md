# Tasks

## 1. Tier A — Unblock and speed the fetch pipeline (D13, D14)

- [ ] 1.1 Raise `--lookup-concurrency` default 64→256 (aggressive 256→512); raise `--qps` default 5000→8000 (aggressive 12000)
- [ ] 1.2 Instrument stats: add `fetch_in_flight` and `queue_depth` counters; log them in the stats loop
- [ ] 1.3 Early dead-hash abort: if the first ~24 dials all end in connect timeout/refused with zero successful handshakes, abort the fetch before the deadline
- [ ] 1.4 Shorten `FETCH_DEADLINE` 20s→12s
- [ ] 1.5 Tests: early-abort logic (pure helper), deadline honored

## 2. Tier B — Raise verify rate (D15)

- [ ] 2.1 Implement in-run dead-peer cache (`IpAddr → last-failure`), TTL ~10 min, skip after ≥2 failures
- [ ] 2.2 Wire the cache into `fetch_one` dial candidate selection
- [ ] 2.3 Tests: cache skips repeated dead IPs, TTL expiry allows retry

## 3. Tier C — Multi-instance + routing warmup (D16, D17)

- [ ] 3.1 Add `--instances N` (default 1); per-instance UDP port `port+i` and `state-dir/instance-i/`
- [ ] 3.2 Spawn N samplers sharing one `Storage` and one fetch pool
- [ ] 3.3 Add routing warmup: ~16 throttled `get_peers` on random targets at startup
- [ ] 3.4 Tests: instance port/state-dir computation, warmup target generation

## 4. Tier D — Retry robustness (D18)

- [ ] 4.1 Backoff base 5m→60s (cap 6h unchanged)
- [ ] 4.2 `empty_peers` failures get a short 60s retry; other reasons keep exponential backoff
- [ ] 4.3 Tests: backoff base, empty_peers short window

## 5. Integration, docs, verification

- [ ] 5.1 README: `--instances`, new defaults, dead-peer cache, retry behavior
- [ ] 5.2 Live smoke test on the NAT host (single instance): confirm utilization rises, verify rate improves, queue drains
- [ ] 5.3 `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release` all green
- [ ] 5.4 Commit the change set
