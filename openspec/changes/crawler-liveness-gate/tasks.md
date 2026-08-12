# Tasks

## 1. Phase A — shared liveness counter (D57, D58)

- [ ] 1.1 New `crawler/src/discovery/liveness.rs`: `DashMap<[u8;20], SmallVec<(Id20, Instant); 4>>` upsert-by-source; `--liveness-cap` (8) distinct sources; evict oldest distinct source on overflow
- [ ] 1.2 Create once in `crawler.rs` (SharedBloom pattern), clone into every `Sampler`
- [ ] 1.3 Replace per-loop `SeenCounts` in `SamplerLoop` with the shared counter
- [ ] 1.4 Emit when distinct sources within window >= `--min-seen`

## 2. Phase B — window + backstop (D59, D60)

- [ ] 2.1 `--liveness-window` (120s) per-report expiry on encounter
- [ ] 2.2 Entry lifetime = `max(min_seen, min_seen_shadow)`; live emit does not delete entry when shadow is higher
- [ ] 2.3 Global backstop `--liveness-max-entries` (100k) + periodic sweep task evicting expired/overflow entries
- [ ] 2.4 Memory sanity check in code docs: ~2,900 entries ≈ 0.25-0.9 MB/process

## 3. Phase C — shadow mode (D61, D62)

- [ ] 3.1 `--min-seen-shadow N` flag; counters `shadow_filtered` / `shadow_emitted` / `shadow_near_miss_1` / `shadow_near_miss_2`
- [ ] 3.2 Standalone debug log of filtered hash sample (no DB change)
- [ ] 3.3 Near-miss bucketing by max distinct sources reached; documented to tune window not min-seen

## 4. Phase D — flip default (after shadow validation)

- [ ] 4.1 `--min-seen` default 1 → 3 (after the 24h shadow run confirms the estimate)

## 5. Verify

- [ ] 5.1 Unit tests: cross-loop aggregation, upsert-by-source dedup, window expiry, cap eviction, backstop sweep, shadow lifetime
- [ ] 5.2 `cargo test` + `cargo clippy --all-targets -- -D warnings` clean
- [ ] 5.3 `cargo build --release` clean
- [ ] 5.4 Deploy with shadow; benchmark vs the ~13.5 fetches/s / 0.16% baseline
- [ ] 5.5 After 24h shadow: analyze filtered/near-miss counters; tune window if near-misses cluster at the edge; then flip min_seen=3 and re-benchmark
