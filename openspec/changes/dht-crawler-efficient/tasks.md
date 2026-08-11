# Tasks

## 1. Phase A — cut the bandwidth bleed (D28)

- [ ] 1.1 `--qps` 8000 → 2000, `--sampler-qps` 2000 → 400 (defaults + aggressive)
- [ ] 1.2 `PARALLEL_DIALS` 32 → 8, `MAX_PEERS_PER_HASH` 100 → 25, `FETCH_DEADLINE` 20s → 10s
- [ ] 1.3 Routing growers 100ms → 1s in `crawler.rs`

## 2. Phase B — Redis shared dedup (D29, D30)

- [ ] 2.1 Add `redis` dependency; new `redis.rs` module (URL-based, graceful fallback)
- [ ] 2.2 Shared `SEEN` set in the sampler (emit → SADD; skip if already in shared set)
- [ ] 2.3 Shared dead-peer cache in `fetch` (fleet-wide IP skip, TTL)
- [ ] 2.4 `--redis-url` flag; if absent/error, fall back to in-memory behavior

## 3. Phase B — discovery per byte (D31)

- [ ] 3.1 `pick_target` spreads across the full routing table (wider sampling set)
- [ ] 3.2 Confirm get_peers lookups feed nodes into routing (already via actor.rs:1126)

## 4. Phase C — verify quality (D32)

- [ ] 4.1 `--min-seen` default 2 → 3
- [ ] 4.2 Compose: 4 instances; add a `redis` service; wire `--redis-url`; `--min-seen 2`

## 5. Phase D — per-instance stats

- [ ] 5.1 Log per-instance routing nodes / sampled / unique in the stats loop

## 6. Integration, deploy, verify

- [ ] 6.1 `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release` all green
- [ ] 6.2 Deploy to remote-dev (4 instances + redis), confirm bandwidth drops and unique rises
- [ ] 6.3 Commit the change set
