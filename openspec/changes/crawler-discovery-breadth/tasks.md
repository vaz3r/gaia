# Tasks

## 1. Backoff inversion + graduation (D63, D64)

- [x] 1.1 `STALE_BACKOFF` 300s → 60s (healthy-0-new short backoff)
- [x] 1.2 `FAIL_BACKOFF` 10s → 30s (non-responsive long backoff)
- [x] 1.3 `STALE_GRADUATION` = 3; `STALE_LONG_BACKOFF` = 300s (long shelf only after 3 consecutive empties)
- [x] 1.4 Per-node `consecutive_stale` counter; reset on a response with new hashes or on a timeout/error
- [x] 1.5 Unit-test the graduation logic (existing node_stats tests updated)

## 2. Wider node spread (D65)

- [x] 2.1 `PICK_CANDIDATES` 64 → 256
- [x] 2.2 Per-loop `cursor`; `pick_target` rotates the ready list by the cursor each pick
- [x] 2.3 Tests pass with the new signature

## 3. Verify

- [ ] 3.1 `cargo test` (crawler + gaia-dht) clean
- [ ] 3.2 `cargo clippy --all-targets -- -D warnings` clean
- [ ] 3.3 Deploy; measure distinct-node sampling rate (hashes_sampled delta / 20) rising from ~12/sec toward ~73/sec
- [ ] 3.4 Measure `unique/hr` and `verified/hr ÷ unique/hr` (D66): confirm the 3-5x unique gain converts to verified/hr without a yield collapse
- [ ] 3.5 Open `crawler-discovery-breadth` spec package
