## Why

Measured on remote-dev (scale=3): **175,886 fetches → 276 verified (0.16% success)**. The crawler fetches nearly every unique hash (312k/hr) because `--min-seen 1` emits on first sighting. ~99% of that fetch traffic (TCP handshakes + partial metadata to dead peers) is pure waste — the single largest remaining bandwidth/CPU cost.

An earlier `--min-seen 2` attempt failed because the sampler's `SeenCounts` was **per sampler loop**: "2 sightings" silently meant "the same loop saw it twice", which at 96+ loops is ~190x rarer than "2 different loops saw it once". Discovery collapsed (312k → 3.7k unique/hr) and was reverted.

This change implements a **fleet-wide-in-process liveness gate** with correct distinct-source semantics: a hash is fetched only after N distinct DHT nodes reported it within a rolling window. It also adds a **shadow mode** that validates the filter before it goes live — the lesson from the min_seen=2 incident being to never trust a liveness threshold estimate without first observing it against real traffic.

## What Changes

- **Phase A — shared distinct-source liveness counter**:
  - Replace per-loop `SeenCounts` with one shared per-process counter across all sampler loops: `DashMap<[u8;20], SmallVec<(Id20, Instant); 4>>` keyed per hash, **upsert by source node ID** (a node reporting repeatedly updates its timestamp in place, never consumes extra slots). Cap = max distinct sources tracked (8).
  - A hash is emitted to the fetcher only when `distinct sources within window >= --min-seen`.
  - **Entry lifetime governed by `max(--min-seen, --min-seen-shadow)`**, so shadow mode can observe a hash accumulating past the live threshold without the live emit deleting it.
- **Phase B — rolling window + eviction**:
  - `--liveness-window` (default 120s): a report older than the window is expired on encounter.
  - Per-hash source cap 8 (evict oldest distinct source on overflow).
  - **Global backstop** `--liveness-max-entries` (default 100k, oldest-first) plus a periodic sweep task that evicts expired/overflow entries — one-hit-wonders that are never re-read cannot accumulate.
- **Phase C — shadow mode (validation-first)**:
  - `--min-seen-shadow N` logs what *would* be emitted under `min-seen=N` while the live path keeps running at its current setting. Standalone debug log + counters (`shadow_filtered`, `shadow_emitted`, `shadow_near_miss_{1,2}`), **no DB/schema change**.
  - Near-miss buckets (hashes that reached 1 or 2 distinct sources then expired) detect the STALE_BACKOFF coupling: two early sightings going stale before a third lands. If near-misses cluster at the window edge, **tune the window, not min-seen**.
- **Phase D — flip the default** (after shadow validation): `--min-seen` 1 → 3.

## Capabilities

### New Capabilities

- `liveness-gate`: shared cross-loop distinct-source counter; fetch only after N distinct nodes report within a rolling window.
- `shadow-mode`: observe a candidate `--min-seen` against live traffic without enabling it; near-miss bucketing for window tuning.

### Modified Capabilities

- `discovery` (previous changes): per-loop `SeenCounts` → shared `LivenessCounter`; min_seen default 1→3 (after shadow validation).
- `fetch` (previous changes): receives only corroborated hashes (fetch volume toward the live tail).
- `cli` (previous changes): `--min-seen-shadow`, `--liveness-window`, `--liveness-cap`, `--liveness-max-entries`.

## Impact

- **Expected**: same verified/hr at a fraction of the bandwidth — fetch volume drops from ~13.5 fetches/s toward the corroborated tail, attacking the 0.16% success waste at its source (dead-hash dials), not its symptom (per-fetch bytes).
- **Bandwidth**: fetch volume reduction dominates; RESPONSE_K + incremental-pieces already shaved per-response/per-stall costs.
- **Memory**: ~0.25-0.9 MB/process (pending hashes ≈ unique-new/hr × window ≈ 24/s × 120s ≈ 2,900 entries; per-entry ~80-120B typical, ~300B worst case). Bounded by the global backstop.
- **Risk**: a wrong window/threshold could delay rare-but-live hashes; mitigated by shadow mode's near-miss buckets and window-first tuning. The liveness gate only gates *sampled* hashes — announced/peer-hinted hashes (live by construction) stay exempt.
