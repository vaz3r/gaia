# Tasks

## 1. Phase A — bandwidth optimization (D49, D50)

- [x] 1.1 Add `RESPONSE_K` (16) in `crawler/crates/gaia-dht/src/routing_table.rs`
- [x] 1.2 Use `RESPONSE_K` (not `K`) in the 5 response paths in `crawler/crates/gaia-dht/src/actor.rs` (FindNode :1318, GetPeers :1361, GetItem :1453/:1497, SampleInfohashes :1684)
- [x] 1.3 Keep table `K=80`; run gaia-dht suite (247 tests)
- [x] 1.4 `PARALLEL_DIALS` 16→4, `MAX_PEERS_PER_HASH` 50→16, `FETCH_TIMEOUT` 5s→3s, `EARLY_ABORT_DIALS` 64→24 in `crawler/src/fetch/mod.rs`

## 2. Phase B — layout restructure (D52, D53)

- [x] 2.1 `git mv dht-crawler crawler`; `git mv vendor/gaia-* crawler/crates/`
- [x] 2.2 Root `Cargo.toml` members → `["crawler", "crawler/crates/gaia-bencode", ...]`
- [x] 2.3 `crawler/Cargo.toml` path deps → `crates/gaia-*`
- [x] 2.4 Update `Dockerfile` COPY paths, `docker-compose.yml` (service/container `crawler`, volume name stable), `run.sh`, `ecosystem.config.cjs`, `.gitignore`, `.env.example`, `README.md`
- [x] 2.5 Update `crawler/src/cli.rs` (binary name + RUST_LOG filter), `crawler/src/fetch/wire.rs` (client-id string)
- [x] 2.6 Update `benchmark/*.sh` compose-dir default, `openspec/config.yaml`

## 3. Verify

- [x] 3.1 `cargo test` (crawler 34 + gaia-dht 247 + core/wire/bencode) clean
- [x] 3.2 `cargo clippy --all-targets -- -D warnings` clean
- [x] 3.3 `cargo build --release` clean
- [x] 3.4 Docker build clean
- [x] 3.5 Deploy to remote-dev; measured ~1.41 MB/s (was 1.57 MB/s) at ~468/hr — see follow-up for the fetch-volume lever
