## Why

The throughput change (`dht-crawler-throughput`) fixed the fetch pool (verified rate rose from ~35/hr to ~48/hr sustained), but discovery is still the ceiling. Measured over a 2.9h PM2 session on the NAT host:

- **48 torrents/hr**, verify rate 0.30%, fetch pool at ~4.6/s vs ~42/s capacity (**90% headroom**).
- Only **1.4%** of sampled hashes are unique (72k unique / 5.0M sampled). We re-see the same hashes because the routing table is small (~263 nodes) and few nodes respond to BEP 51, so the sampler re-queries the same ~20–30 nodes that return the same ~400–600 hashes.
- The routing table **stalls** because nothing grows it in steady state: `sample_infohashes` responses feed ≤8 closer nodes only from BEP 51-capable nodes, and the startup warmup does just 16 `get_peers` then stops.

The fix is to **discover more nodes, continuously**: run more independent DHT instances (each with its own routing table and sampler), and give each instance a background routing-table grower so its table climbs toward the configured cap. More BEP 51-capable nodes per table → more distinct hashes → more torrents. No Redis, no irontide patch.

## What Changes

- **Continuous routing grower**: replace the one-shot 16-query warmup with a background task per instance that keeps issuing `get_peers` on random targets (throttled), injecting newly-discovered nodes into that instance's routing table via the DhtLookup cascade.
- **Higher routing table cap**: `--max-nodes` default 2048 → 4096 (aggressive 4096 → 8192) so growing tables have headroom.
- **`--no-restrict-ips` flag**: disable irontide's one-node-per-IP routing restriction, which on NAT can suppress diversity (many peers share egress IPs). Off by default; opt-in.
- **More bootstrap nodes**: expand the default list so cold starts (and each new instance) seed from more entry points.
- **PM2: 4 instances**: `ecosystem.config.cjs` runs `--instances 4`, so four independent routing tables and samplers feed the shared fetch pool + database.

## Capabilities

### New Capabilities

- `routing-grower`: a background, per-instance task that continuously issues `get_peers` on random keyspace targets to grow the routing table toward the configured cap.
- `multi-instance` (extended): previously added; now enabled by default in PM2 at 4 instances.

### Modified Capabilities

- `discovery` (previous change): gains the continuous grower and the `--no-restrict-ips` option.
- `cli` (previous change): `--max-nodes` default raised; new `--no-restrict-ips` flag.
- `architecture` (previous change): the crawler spawns a grower task per instance alongside each sampler.

## Impact

- **Code**: `discovery/mod.rs` gains a grower loop; `crawler.rs` spawns one grower per instance; `cli.rs` adds the flag and default change; `discovery/mod.rs` bootstrap list expanded.
- **Dependencies**: none added.
- **Operations**: 4 instances consume 4 UDP ports (6881–6884) and ~4× sampling load; routing tables grow larger (more memory per instance). `--no-restrict-ips` is opt-in.
- **Performance (expected)**: unique discovery rate up ~4× from instances plus more from each denser routing table; torrents/hr toward ~200–300 at 4 instances.
