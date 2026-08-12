## Context

Building on `crawler-fetch-selectivity` (af43f11). The crawler fetches ~13.5 hashes/s with 0.16% success because `--min-seen 1` emits on first sighting. The prior `--min-seen 2` attempt collapsed discovery because `SeenCounts` is per sampler loop, silently meaning "same loop saw it twice". This change implements the correct distinct-source liveness gate fleet-wide-in-process, with a shadow mode to validate the threshold before flipping it. Adds decisions D57-D62.

## Goals / Non-Goals

**Goals:**
- Gate sampled-hash fetches on N *distinct* DHT nodes reporting the hash within a rolling window (correct cross-loop semantics).
- Provide a shadow mode that observes a candidate threshold against live traffic before enabling it (validation-first, per the min_seen=2 lesson).
- Keep announced/peer-hinted hashes exempt (live by construction).
- Cut fetch volume toward the corroborated tail without reducing verified/hr.

**Non-Goals:**
- No content filtering.
- No change to `--scale` (3), instance count (4), or Docker/Gluetun/Redis architecture.
- No Redis on the hot path (measured: ~344 reports/s would be 1.4% of Redis INCR capacity, but synchronous round-trips would stall a ~20-sample response by ~26ms — the counter stays in-process).
- No DB/schema change for shadow mode (standalone debug log + counters).

## Decisions

### D57 — Shared distinct-source liveness counter (in-process)
One `DashMap<[u8;20], SmallVec<(Id20, Instant); 4>>` shared across all sampler loops in the process, created once (`crawler.rs:101`, same pattern as `SharedBloom`). Reports are **upserted by source node ID** — a repeat from the same node updates its timestamp in place, never a new slot.
- *Rationale:* per-loop `SeenCounts` was the min_seen=2 bug; a shared map fixes "3 sightings" to mean 3 distinct nodes across the fan-out. Keying by source node ID (the node we sampled, `pick_target` returns `(target, node_addr)` with `target` = the node's own ID) is what makes distinctness real. Process-wide is correct because the 4 instances have distinct routing tables/IDs, so their reports are genuinely distinct sources.
- *Trade-off:* one map shared across instances means the source key is the node ID, not the instance — which is the right abstraction (sources are nodes, not processes).

### D58 — Dedupe-on-insert prevents source crowding
Cap = max distinct sources tracked (8). If node A reports 5x and only 1 slot is consumed, nodes B and C always have room; a genuinely-new 9th source evicts the *oldest* distinct source.
- *Rationale:* appending every report lets a chatty node crowd out distinct competitors, producing undercounts unrelated to the window (the ring-crowding bug). Upsert-by-source eliminates it; A/B/C provably coexist up to cap.
- *Trade-off:* evict-oldest-distinct-source on overflow is a backstop that rarely fires at min_seen=3 (entries are removed at threshold first); it matters mainly under shadow accumulation and adversarial high-fanout hashes.

### D59 — Entry lifetime = max(--min-seen, --min-seen-shadow)
An entry is NOT removed when the live threshold is reached if shadow mode is testing a higher threshold. It stays, accumulating further reports, until it reaches the higher threshold, expires from the window, or falls out via the global backstop.
- *Rationale:* emission-triggered removal would break shadow mode — under a live default of 1, every entry would be emitted+deleted at its first report, and the shadow counter would never observe a 2nd or 3rd source. Lifetime must be governed by the higher of the two thresholds.
- *Trade-off:* entries live longer under shadow (still bounded by window + backstop).

### D60 — Rolling window + global backstop
`--liveness-window` (default 120s): a report older than the window expires on encounter. Per-report expiry (each report has its own timestamp). A periodic sweep task enforces the **global backstop** `--liveness-max-entries` (default 100k, oldest-first) so one-hit-wonders never revisited cannot accumulate — the steady-state ~2,900-entry estimate cannot drift.
- *Rationale:* `(count, last_seen)` can't expire individual reports; a per-report timestamp list can. The sweep complements lazy on-encounter expiry, which alone misses one-hit-wonders.
- *Trade-off:* a sweep task adds a small periodic cost; the window is the primary eviction, the backstop is the memory guard.

### D61 — Shadow mode (validation-first, standalone debug log)
`--min-seen-shadow N` logs what would be filtered under `min-seen=N` while the live path continues at its current setting. Counters: `shadow_filtered` (expired below shadow threshold), `shadow_emitted` (reached shadow threshold), `shadow_near_miss_1` / `shadow_near_miss_2` (expired having reached exactly 1 or 2 distinct sources). Standalone debug log + counters; **no DB/schema change**.
- *Rationale:* the min_seen=2 incident showed we must observe a liveness threshold against real traffic before trusting it. A debug log answers "how many would be cut" and "does the sample look like garbage or plausible-live" without new schema. A DB-backed entry only earns its cost if we need retroactive filtered-vs-verified correlation — which we'll know only after the 24h log, not before. Upgrade to DB-backed only if the log demands it.
- *Trade-off:* debug log + counters only; near-miss correlation is qualitative for now.

### D62 — Near-miss bucketing detects STALE_BACKOFF coupling
Shadow mode buckets expired entries by max distinct sources reached (1, 2) and their time near the window edge, to detect the coupling where two early sightings legitimately go stale (STALE_BACKOFF=300s deprioritizes a node that returned no new hashes) before a third distinct source lands.
- *Rationale:* if near-misses cluster at the window edge, the fix is **tune the window (D60), not min-seen (D57)** — loosening min-seen to compensate for a windowing problem would mask the real issue.
- *Trade-off:* requires reading the near-miss counters against the window value; explicit in the validation criteria.

## Risks / Trade-offs

- **Window/threshold mismatch** could delay rare-but-live hashes; near-miss buckets + window-first tuning mitigate.
- **Memory** bounded by backstop + window; ~0.25-0.9 MB/process in steady state (re-derived from the corrected type and process-wide rates).
- **Sampled-only gate**: announced/peer-hinted hashes exempt (live by construction), so the highest-success path is untouched.
- **Shadow adds no DB surface**: a feature that may be retired once the threshold is confirmed costs nothing to roll back.
