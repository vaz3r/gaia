## Context

Building on the memory-fix commit (365d640) and `crawler-fetch-selectivity` (af43f11). Discovery is the binding constraint (~12/sec of ~73/sec node-sampling ceiling), caused by an inverted backoff: healthy-0-new nodes get 300s while non-responsive nodes get 10s. Adds decisions D63-D66.

## Goals / Non-Goals

**Goals:**
- Invert the backoff so healthy-exhausted nodes are re-queried soon and non-responsive nodes are backed off harder.
- Graduate to the long backoff only after N consecutive empty responses.
- Widen node spread (PICK_CANDIDATES + rotating cursor) so loops cycle the whole table.
- Raise distinct-node sampling toward the ~73/sec ceiling and verified/hr toward ~1,500.

**Non-Goals:**
- No change to the liveness gate (separate change, correct as-is).
- No content filtering.
- No change to the fetch pool (not the current constraint).

## Decisions

### D63 — Backoff inversion (healthy-0-new short, timeout long)
A healthy node returning 0 new hashes is re-queried after a short `STALE_BACKOFF` (60s); a node that times out/errors is backed off `FAIL_BACKOFF` (30s).
- *Rationale:* a healthy exhausted node may pick up hashes shortly; re-querying soon is correct. A non-responsive node is likely offline/overloaded; probing it every 10s wastes budget. The old assignment (300s for healthy, 10s for dead) was inverted.
- *Trade-off:* a temporarily-quiet healthy node is queried more often (cheap UDP); a flaky node is re-checked less often (acceptable — it wasn't yielding).

### D64 — Stale graduation (N consecutive empties)
The long shelf (300s) applies only after `STALE_GRADUATION` = 3 consecutive 0-new responses; a response with new hashes resets the counter.
- *Rationale:* at ~1.8% yield, a single 0-new is the common case even for productive nodes. Treating one data point as a verdict is the same category of bug as the per-loop min_seen counting error. Requiring 3 consecutive empties tolerates variance before deprioritizing.
- *Trade-off:* slightly more per-node state (a u32 counter); negligible.

### D65 — Rotating sampler cursor
Each sampler loop keeps a `cursor` over the routing table and rotates the ready list by it each pick, so consecutive picks advance through the whole table instead of re-selecting the same high-score nodes.
- *Rationale:* the "no ready node" starvation occurs when loops converge on a few ready nodes; a rotating window ensures broad coverage even when few nodes are marked ready.
- *Trade-off:* a cursor + rotate per pick (O(n) copy of ready nodes, already done for shuffle); negligible at ~300 qps.

### D66 — Yield-on-new-candidates measurement gate
Track `verified/hr ÷ unique/hr` alongside `unique/hr` after deploy; if the ratio drops materially as unique discovery rises, the marginal candidates are lower quality and the projection (3-5x → ~1,500/hr) is too optimistic — re-evaluate discovery breadth (window, backoff) before adding more.
- *Rationale:* the same "assumption that needs a number" discipline applied elsewhere; wider coverage may not convert 1:1 to verified hashes.
- *Trade-off:* requires reading two stats lines; already available.

## Risks / Trade-offs

- **More queries per healthy node**: cheap UDP, bounded by sampler_qps.
- **Marginal-candidate quality**: measured via D66; tune backoff/window, not min_seen, if yield drops.
- **Cursor interaction with interval map**: ready-list filtering still respects per-node intervals; the cursor only reorders, never skips.
