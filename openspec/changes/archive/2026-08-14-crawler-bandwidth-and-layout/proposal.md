## Why

The scale change (`dht-crawler-scale`, committed 2d0e19f) raised discovery and verification ~10x (routing table to thousands, ~800-1,000 torrents/hr) but **bandwidth ballooned to ~1.57 MB/s with abysmal efficiency**: only **262 verified / 97,912 fetches (0.27% success)**. Two distinct leaks were identified in review:

1. **UDP response inflation from K=80 (the scale change's side effect).** Raising the routing-table `K` to 80 also raised the number of nodes returned in inbound-query responses. Five response paths in `gaia-dht` (`actor.rs`: FindNode :1318, GetPeers :1361, BEP44 GetItem :1453/:1497, SampleInfohashes :1684) now emit up to **80 CompactNodeInfo (~2KB per response) instead of 8 (~208B)** — a 10x inflation on every answer we send. With 4,000+ routing nodes across 4 public-IP instances answering many inbound queries, this is the single largest avoidable bandwidth leak.

2. **Fetch pipeline dials dead peers at scale.** `PARALLEL_DIALS=16`, `MAX_PEERS_PER_HASH=50`, `FETCH_TIMEOUT=5s`, `EARLY_ABORT_DIALS=64`, and `--scale 10` (5120 concurrent) means ~97k fetches with 99.1% failure — dominated by `connect_timeout` (72,930) and `empty_peers` (74,057). Each failed fetch dials up to 16 peers over TCP.

Separately, the codebase layout needs restructuring: the `dht-crawler/` folder should be `crawler/`, and the owned `gaia-*` crates (currently at root `vendor/`) should live inside it as internal workspace members (`crawler/crates/gaia-*`) per Rust workspace conventions.

## What Changes

### Phase A — Bandwidth optimization (keep discovery, cut waste)

- **A1 — decouple response-payload size from table K**: add a `RESPONSE_K` constant (16) used only in the five inbound-response paths; the routing table keeps `K=80` for capacity (that is what grew discovery). Responses return ≤16 nodes per BEP 5 semantics. Cuts outbound UDP ~10x with zero discovery loss.
- **A2 — tighten fetch dial budgets**: `PARALLEL_DIALS` 16→4, `MAX_PEERS_PER_HASH` 50→16, `FETCH_TIMEOUT` 5s→3s, `EARLY_ABORT_DIALS` 64→24. Live-hash success is unchanged (the first live peer still wins); dead-hash TCP churn drops ~4x. `--scale` stays 10.

### Phase B — Restructure to `crawler/` with internal `crawler/crates/gaia-*`

- **B1** — `git mv dht-crawler crawler`; `git mv vendor/gaia-* crawler/crates/`.
- **B2** — workspace members + path deps point at `crawler/crates/gaia-*`.
- **B3** — update every reference: Dockerfile COPY paths, compose service/container names + volume, run.sh, ecosystem.config.cjs, .gitignore, .env.example, cli binary name + RUST_LOG filter, wire client-id string, benchmark scripts' compose-dir default, openspec config. Keep the DB/Redis volume name stable so data persists.
- **B4** — full test/clippy/build + Docker build + deploy + benchmark.

## Capabilities

### New Capabilities

- `bandwidth-efficient-responder`: inbound query responses return ≤16 nodes (RESPONSE_K) while the routing table holds K=80 — the 10x UDP inflation removed.
- `crawler-layout`: `crawler/` app dir with internal `crawler/crates/gaia-*` workspace members.

### Modified Capabilities

- `fetch` (previous changes): tighter dial budgets (4 parallel, 16 peers, 3s timeout) cut dead-peer churn ~4x.
- `dht` (previous changes): response payload size decoupled from table capacity.
- `architecture` (previous changes): directory + workspace layout rename.

## Impact

- **Expected**: same verified/hr at ~1/3 the bandwidth (~0.5 MB/s vs 1.57 MB/s). The ~99% dead-peer fetch waste and the 10x response inflation are both pure waste — cutting them does not reduce live discovery or verification.
- **Bandwidth**: outbound UDP responses ~10x smaller; fetch TCP churn ~4x lower; total ~0.3-0.5 MB/s (~1.5 TB/mo total, well within Oracle's 10 TB/mo free egress).
- **Risk**: lower `PARALLEL_DIALS` could miss a slow-but-live peer only if all 4 first dials fail; mitigated by `FETCH_DEADLINE` and retry backoff. Folder rename touches many files; verified by full test/build/Docker/deploy.
