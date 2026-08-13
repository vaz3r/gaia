## Why

Measured on remote-dev (scale=3, min_seen=1): the crawler samples only **~12 distinct DHT nodes/sec out of a ~73/sec ceiling** (~17% utilization), yielding ~96,600 unique hashes/hr and ~150-200 verified/hr. The binding constraint is the **discovery loop**, not the fetch pool (fetch in-flight is ~10-13% of 1536) and not the liveness gate (which filters emission, not discovery).

Root cause: the sampler's backoff logic is **inverted** relative to good DHT etiquette.

- A node that responds promptly but returns **0 new hashes** (healthy, exhausted-for-now) gets `STALE_BACKOFF` = **300s** — shelved for 5 minutes even though it may pick up new hashes shortly.
- A node that **does not respond at all** (timeout/error — possibly offline) gets `FAIL_BACKOFF` = **10s** — re-queried every 10s forever, burning budget on dead nodes.

At ~1.8% unique yield, ~98% of responses are healthy-but-0-new, so ~98% of the routing table goes into 300s backoff, the ready pool collapses, and `pick_target` starves. This is the discovery-breadth gap keeping us far below bitmagnet's real-world ~1,500/hr.

## What Changes

- **Phase A1 — backoff inversion**: healthy-0-new → short backoff (60s); timeout/no-response → longer backoff (30s). A healthy exhausted node is re-queried soon (it may have new hashes); a non-responsive node is backed off harder.
- **Phase A2 — graduation**: a node only earns the long 300s shelf after **3 consecutive** 0-new responses (`STALE_GRADUATION`). One unlucky miss no longer costs a productive node its short-backoff status (the "single data point as verdict" bug).
- **Phase B — wider spread**: `PICK_CANDIDATES` 64 → 256, plus a **per-loop rotating cursor** over the routing table so loops cycle through the whole table instead of re-selecting the same high-score nodes.

## Capabilities

### New Capabilities

- `backoff-inversion`: healthy-0-new nodes get a short backoff; non-responsive nodes get a longer one.
- `stale-graduation`: long backoff only after N consecutive empty responses.
- `rotating-sampler-cursor`: per-loop cursor cycles through the full routing table.

### Modified Capabilities

- `discovery` (previous changes): backoff semantics + node-spread across the routing table.
- `sampler` (previous changes): `PICK_CANDIDATES` and per-loop cursor.

## Impact

- **Expected**: distinct-node sampling rises from ~12/sec toward the ~73/sec ceiling, lifting unique hashes/hr (~3-5x) and, if marginal candidates verify at similar rates, verified/hr toward bitmagnet's ~1,500/hr target. The liveness gate (separate change) cuts the wasted-fetch tail.
- **Bandwidth**: more distinct nodes queried (cheap UDP); fetch volume unchanged by this change.
- **Risk**: the 3-5x unique-rate gain may not convert 1:1 to verified/hr if wider coverage surfaces lower-quality hashes. **Mitigation:** measure `verified/hr ÷ unique/hr` (yield-on-new-candidates) alongside `unique/hr` post-deploy, and tune window/backoff rather than min_seen if it drops.
