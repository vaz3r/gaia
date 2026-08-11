# Tasks

## 1. Fetch pipeline unblock (D7)

- [x] 1.1 Release the lookup semaphore immediately after `get_peers()` returns the stream, before the peer-dial loop
- [x] 1.2 Confirm `concurrency` (default 512) now bounds in-flight fetches instead of `lookup_concurrency` (64)
- [x] 1.3 Unit/regression: fetch pool honors `--concurrency`; lookup permit covers only lookup initiation

## 2. Keep everything (D11 — classification removed, nothing filtered)

- [x] 2.1 Remove the movie/TV filter gate so every verified torrent is persisted
- [x] 2.2 Remove the `Category` enum and the classifier module entirely (no storage target for classification)
- [x] 2.3 Remove the `Skipped` fetch outcome and the `filtered_skip` counter; `records_persisted == metadata_verified`
- [x] 2.4 Schema migration: rebuild `torrents` as torrent-metadata-only (drop `category`/`title`/`year`/`season`/`episode`)
- [x] 2.5 Tests: metadata-only migration preserves rows and drops media columns, search still works

## 3. Discovery fixes and hardening (D8, D9)

- [x] 3.1 Cap effective BEP 51 re-query interval (`--sampler-max-interval`, default 60s) so 6h-interval nodes are re-queried
- [x] 3.2 Rewrite `pick_target`: random ready node, target = its own node ID, shuffle ready set before sampling
- [x] 3.3 Fix `choose_multiple`-returns-original-order convergence (all loops hitting one node on small tables)
- [x] 3.4 Bump defaults: `--sampler-loops 32`, `--sampler-qps 2000`, `--qps 5000`, `--min-seen 2`
- [x] 3.5 Raise the `--aggressive` preset (loops 64, sampler-qps 4000, qps 10000, concurrency 1024, lookup-concurrency 256)
- [x] 3.6 Tests: pick_target skips cooling nodes, prefers productive nodes, returns None when all cooling

## 4. Announcement measurement (D8 — observe before patching)

- [x] 4.1 Log `announced_hashes` (`handle.stats().peer_store_info_hashes`) in the stats loop — no irontide patch
- [x] 4.2 Document the decision: no vendoring/patching unless `announced_hashes` grows large; revisit-with-data in a future change

## 5. Fetch budget tuning (D10)

- [x] 5.1 `FETCH_DEADLINE 45s → 20s`
- [x] 5.2 `MAX_PEERS_PER_HASH 100 → 50`

## 6. Architecture: modular layout (D12)

- [x] 6.1 Split `main.rs`: thin `main` dispatcher + `cli` (args) + `crawler` (pipeline wiring, shutdown) + `query` + `purge`
- [x] 6.2 Move `write_loop` and `stats_loop` out of `main.rs` into `crawler`
- [x] 6.3 Reorganize `dht/` → `discovery/` (sampler module; announce capture deferred behind the `announced_hashes` gate)
- [x] 6.4 Reorganize `metadata/` → `fetch/` (pool + wire + parse); classifier module removed (D11)
- [x] 6.5 Split `storage.rs` → `storage/{mod,schema,model}.rs`
- [x] 6.6 Verify one-way dependencies; no module cycles; `cargo test` + `cargo clippy -- -D warnings` clean

## 7. Schema redesign: torrents = torrent metadata only (D11)

- [x] 7.1 `torrents` table → `info_hash BLOB PK, name TEXT NOT NULL, size_bytes INTEGER, file_count INTEGER, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL`
- [x] 7.2 Migration: rebuild `torrents`, drop `category`/`title`/`year`/`season`/`episode`; preserve torrent metadata; `scanned.info_bytes` retained for re-classification
- [x] 7.3 Update `TorrentRecord` and `query` output (name/size/file-count)
- [x] 7.4 Tests: migration preserves rows, media columns gone, search still works

## 8. Integration, docs, verification

- [ ] 8.1 README: updated defaults table (min-seen 2, sampler loops/qps/qps), keep-everything behavior, `announced_hashes` meaning, schema notes
- [ ] 8.2 Live smoke test on the NAT host: confirm `records_persisted == metadata_verified`, routing table grows, `announced_hashes` logged
- [ ] 8.3 `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release` all green
- [ ] 8.4 Commit the change set
