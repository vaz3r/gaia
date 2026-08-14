# Tasks

## 1. Fetch selectivity (D54 — reverted after measurement)

- [x] 1.1 `--min-seen` default 1 → 2 in `cli.rs` + compose
- [x] 1.2 Measured on remote-dev: discovery collapsed (312k→3.7k unique/hr, ~20 verified/hr) because the sampler counts sightings per loop, not fleet-wide
- [x] 1.3 REVERTED to `--min-seen 1`; documented why a fleet-wide gate is deferred

## 2. Download cap (D55 — kept)

- [x] 2.1 `fetch/wire.rs`: request piece 0, then the next piece only after a piece arrives (incremental)
- [x] 2.2 SHA-1 verification gate and reject/mismatch handling unchanged
- [x] 2.3 Measured: per-fetch download 12.6KB → 9.6KB (~23% cut)

## 3. Verify

- [x] 3.1 `cargo test` (34) + clippy clean
- [x] 3.2 `cargo build --release` clean
- [x] 3.3 Deploy to remote-dev; measured ~285-292/hr at ~0.81 MB/s down / 1.24 MB/s total (baseline 1.41 MB/s / ~468/hr)

## 4. Resource usage fix (post-hoc, scale=10 caused OOM)

- [x] 4.1 Root cause: `--scale 10` on a 4-core host → 5120 concurrent fetches + 320 sampler loops × 4 instances, and `SEEN_CAP=1M` per loop → ~41 GB worst-case seen-map heap
- [x] 4.2 `SEEN_CAP` 1_000_000 → 16_384 (shared bloom is the real re-fetch guard)
- [x] 4.3 `--scale` 10 → 3 (concurrency 1536, loops 96) for the 4-core/16GiB host
- [x] 4.4 compose: `mem_limit: 6g`, `pids_limit: 8192` so a runaway can't OOM the host
