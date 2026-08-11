## Context

Building on `dht-crawler-throughput` (committed). The fetch pool is fixed and healthy; discovery breadth is now the ceiling. Root cause: the routing table stalls in steady state because nothing actively grows it, and a single instance has one routing table. This change adds decisions D19–D22.

## Goals / Non-Goals

**Goals:**
- Grow each instance's routing table continuously toward the configured cap so more BEP 51-capable nodes are discovered.
- Run 4 independent instances by default in PM2 to multiply discovery breadth.
- Give operators a `--no-restrict-ips` opt-in for NAT environments where one-node-per-IP suppresses routing diversity.
- Keep dependencies and architecture unchanged (no Redis, no irontide patch).

**Non-Goals:**
- No Redis / distributed coordination (deferred; the routing table cannot be injected into irontide anyway).
- No irontide vendoring/patching.
- No fetch-pool changes (it has 90% headroom).
- No schema changes.

## Decisions

### D19 — Continuous routing grower per instance
Replace the one-shot 16-`get_peers` startup warmup with a background task that continuously issues `get_peers` on random 20-byte targets, throttled (default ~1 query per 300ms per instance). Each `get_peers` runs a DhtLookup that walks toward the target and injects discovered nodes into the routing table, growing it steadily.
- *Rationale:* `sample_infohashes` alone feeds ≤8 nodes only from BEP 51-capable nodes; steady-state the actor does no `find_node` sweeps. `get_peers` cascades are the reliable way to discover arbitrary nodes.
- *Alternatives considered:* relying on the sampler's passive closer-node feedback — insufficient (stalls at ~263). Issuing `find_node` directly — irontide exposes no public `find_node`; `get_peers` on random targets is the available mechanism that triggers the same DhtLookup node discovery.
- *Trade-off:* each grower query consumes a slice of the shared DHT QPS budget; throttled to keep it bounded.

### D20 — Raise the routing table cap
`--max-nodes` default 2048 → 4096 (aggressive 4096 → 8192). The current 263-node table is far below the cap, so this alone does not grow it, but it provides headroom once the grower fills tables.
- *Rationale:* no downside; the actor's bucket hygiene caps memory. Headroom prevents the grower from hitting a hard stop.

### D21 — `--no-restrict-ips` opt-in
Add a `--no-restrict-ips` flag that sets `DhtConfig::restrict_routing_ips = false`. Default (restrict on) keeps one node per IP; the flag lifts it for NAT environments where many peers share egress IPs, potentially growing the table with more distinct node IDs.
- *Alternatives considered:* always disabling — rejected: one-node-per-IP is a standard hygiene measure against spoofing; make it explicit.
- *Trade-off:* may admit duplicate-egress/spoofed nodes; opt-in for operators who accept that on NAT.

### D22 — PM2 runs 4 instances
`ecosystem.config.cjs` sets `args = "run ... --instances 4 ..."`. Four independent node IDs, routing tables, and samplers feed one shared fetch pool and database.
- *Rationale:* each instance discovers its own subset of BEP 51 nodes; N instances ≈ N× unique hashes, and the shared fetch pool has headroom to convert them.
- *Trade-off:* 4 UDP ports and ~4× sampling/fetch load; acceptable on this host.

## Risks / Trade-offs

- **Grower QPS** competes with sampler + fetch lookups → throttle (300ms default) and rely on the shared budget; tune from stats.
- **Larger routing tables** use more memory per instance (~linear in nodes); 4096 nodes is modest.
- **`--no-restrict-ips`** may admit spoofed nodes → opt-in, documented.
- **4 instances** multiply load; `--aggressive` with 4 instances is VPS-grade.

## Migration Plan

No schema change. Existing databases and routing state are reused (each new instance creates its own `state-dir/instance-N/`). Rollback: set `--instances 1` and remove `--no-restrict-ips`; no external systems touched.
