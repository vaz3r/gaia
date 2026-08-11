# Tasks

## 1. Verify-rate tuning (D23)

- [x] 1.1 `PARALLEL_DIALS` 16 → 32
- [x] 1.2 `MAX_PEERS_PER_HASH` 50 → 100
- [x] 1.3 `FETCH_DEADLINE` 12s → 20s
- [x] 1.4 `FETCH_TIMEOUT` 7s → 10s
- [x] 1.5 `CONNECT_TIMEOUT` 3s → 5s
- [x] 1.6 `EARLY_ABORT_DIALS` 24 → 64 (proportionate to higher parallel dials)

## 2. Integration, verification

- [x] 2.1 `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release` all green
- [x] 2.2 Deploy to PM2 (4 instances), confirm verify rate rises
- [ ] 2.3 Commit the change set
