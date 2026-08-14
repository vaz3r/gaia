# VALIDATION — increasing verified-torrent ratio

Goal: maximize verified torrents/hr. Working hypothesis from prior phases: fetch
conversion (~0.15%) is the wall, not discovery (we now sample ~500k unique/hr).

## Replay harness (bench-fetch)

`crawler bench-fetch --db <snapshot> --class <class> --sample N [--verify]`
samples distinct hashes from a DB snapshot by recorded outcome and replays a
peer-resolution strategy. Fast iteration (~30s) vs deploy cycles.

Snapshot: `crawler snapshot --db live --out /data/snapshot.sqlite` (VACUUM INTO,
WAL-free, consistent while running). Current snapshot ~4M scans.

## Findings (2026-08-13)

### F1 — Tracker resolution vs hash liveness (bench, host network)
- `empty_peers` class: trackers return peers for **100%** of hashes, but **0%**
  verify (dial failures: timeout 71, connection_closed 5, parse_error 2).
- `ok` class (previously verified) control: trackers return peers 100%, **~10%**
  verify (timeout 55).
- **Conclusion**: the tracker client is correct; `empty_peers` hashes are
  genuinely dead torrents — peers exist in tracker DBs but none are alive /
  serving ut_metadata. No peer-resolution strategy resurrects them.

### F2 — Dead-hash persistence (snapshot analysis)
- ~4M distinct hashes fetched, avg attempts = 1.0, NO hash scanned twice
  (bloom filter dedups dead hashes perfectly).
- 6,754 distinct OK hashes, **0 overlap** with failed — a hash verifies once or
  never; retry/backoff never converts a failure.
- 299 OK hashes carry `empty_peers` as their failure_reason but no separate
  failed row — they verified within the same fetch after a peer batch failed.

### F3 — Verified rate is gated by live-torrent encounter rate
- ~150-200 verified/hr steady across hours 10-20 (81,109,125,149,151,161,174,
  102,102,163,213,56...).
- Discovery is 5-6x up (90k -> ~500k unique/hr) but verified only ~1.1x —
  the marginal hashes are dead.
- Verified content is genuine (movies/TV/games), consistent quality.

### F4 — Tracker path helps in production now (expanded 55-tracker list)
- tracker_resolved: ~3k -> ~33k/hr; verified_tracker: 9 -> 18 (climbing).
- Live mix: verified_sampled 70 + verified_tracker 18 = 88 in ~30 min
  (~175/hr), with unique ~440-460k/hr.
- Trackers contribute ~20% of verified now (previously 0).

### F5 — All failed classes are dead; trackers recover none (bench)
- timeout / other / deadline / empty_peers classes: trackers return peers for
  100% of hashes but **0% verify** across all classes.
- Only previously-verified hashes verify via trackers (~10%).
- **Conclusion**: the DHT failed-set is permanently dead (no live peer exists
  anywhere). Peer-resolution strategy cannot recover them; the tracker gain in
  production comes from giving *live* hashes a second peer source.

### F6 — Dial depth doesn't help (bench, ok class)
- Dialing 4 tracker peers: ~10% verified. Dialing 16: ~3.3% (the extra dials
  burn the fetch deadline before the good peer is reached). PARALLEL_DIALS=4
  + short deadline is near-optimal; more dials per hash is not a lever.

### F7 — Discovery is table-bound, not QPS-bound (8 instances)
- Aggregate sampling ~520-665/s and climbing, well under the 8x800=6400 qps
  possible. Each instance's ~1k-node table feeds ~80 samples/s. Raising the
  QPS cap won't help; continuous table growth is the only discovery lever and
  it's already running.

### F8 — Production trend: ~213/hr best hour (up from ~150)
- Hour 19 = 213 verified (8 instances + expanded trackers + continuous growth),
  vs ~150/hr baseline before this session's work. ~40% improvement.
- Tracker path contributes ~20% of verified now (verified_tracker climbing).

### F9 — min-seen corroboration gate definitively ruled out (retest)
- Re-tested min-seen=2 with warm tables + trackers active (was tested cold
  before). Same catastrophic result: unique 500k -> 3.2k/hr, verified ~9/hr
  (vs ~213/hr at min-seen=1).
- Root cause: a genuine torrent is almost always reported by exactly ONE DHT
  node within the 120s window. Shadow `near_miss_2` (~6.8k/hr) is mostly dead
  too. Corroboration-by-distinct-sources is the wrong signal for DHT liveness.
- **Conclusion: min-seen MUST stay 1.** Verified comes from single-sighting
  hashes. This is now proven twice (cold + warm), so the gate is not re-tried.

### F10 — Table growth is the discovery lever (in progress)
- 8 instances, tables re-filling after the min-seen experiment (2,183 and
  climbing ~384/hr). unique/hr ~390-540k and rising with table size.
- Hypothesis: unique/hr scales with total table nodes (superlinearly: 2k
  nodes -> ~90k, 4k -> ~500k). If tables reach 8192, unique -> ~2M and
  verified should scale toward ~800/hr. Verifying over the next hours.
- fetch pool never saturates (573-1011/1536, queue ~0) — not the constraint.

### F11 — Table growth stalls at ~2,240 nodes (one-IP DHT ceiling, definitive)
- Over 60 min, total routing_nodes plateaued at 2,234-2,238 across all 8
  instances. The grower finds no new distinct nodes beyond this.
- **Conclusion**: ~2,240 reachable DHT nodes is the hard ceiling for one
  egress IP (all 8 instances share the IP, so their tables are bounded by one
  IP's DHT neighborhood). unique/hr ~445k and verified ~180-213/hr are at the
  ceiling. Table growth is NOT the lever (the neighborhood is exhausted).

### F12 — Restart re-warm penalty (8 instances)
- Each deploy/restart drops verified/hr to ~60-110 for 1-2h while tables
  re-warm (persisted dht_state.json helps but the DHT neighborhood must be
  re-established). Steady state ~180-213/hr requires long-stable uptime.
- **Operational**: avoid restarts; the 8-instance setup needs hours to reach
  peak. verified_tracker also resets to 0 on restart and climbs back.

### F13 — Deeper grower drain was a ~130 MB/hr leak (FIXED)
- The deeper grower (holding get_peers reply channels open + draining 8
  batches/tick to grow tables faster) accumulated DhtLookup state per
  instance. With 8 instances: allocated 280->390 MB and climbing.
- Reverting to fast-exit (drop channel, lookup winds down after ~2 responses)
  **flattens allocated at ~148-160 MB**. Tables still reach the one-IP ceiling
  (~2.1k nodes); verified_tracker hit its best (27 climbing).
- Redis dedup sets (seen/announced/lookedup) also grew unbounded (5.7M
  entries = 236 MB Redis). Now capped at 1M entries with flush-on-cap (dedup
  is best-effort; in-process bloom + DB are authoritative).

### F14 — Steady state after leak fix (08-14 04:00-05:00Z)
- **~184 verified/hr (hour 04)**, memory flat (allocated ~195-215 MB).
- Tracker path: tracker_resolved ~75k/hr (22% of fetches), verified_tracker
  ~27% of verified (86 of 319). empty_peers share dropped to ~40% of failures
  (trackers convert empty_peers into dial-then-fail).
- unique ~160k/hr, tables at one-IP ceiling (~2.3k). No leak.

### F15 — BEP 33 scrape is a dead end in this DHT (shadow experiment, decisive)
- Enabled scrape:1 on get_peers; recorded seed-bloom (bfsd) presence per fetch
  and correlated with verification. Over ~257k fetches / 56 verified:
  - scrape_saw_seeds = 8 (0.003%) — the reachable DHT nodes essentially never
    return a non-empty seed bloom (BEP 33 unsupported / dead hashes).
  - verified_with_seeds = 0, verified_without_seeds = 56 (100% of verified had
    NO seed signal). failed_with_seeds = 8.
- **Conclusion**: a scrape gate (skip seedless hashes) is useless here — it
  would gate on nothing. This rules out Bitmagnet's BEP 33 advantage as a lever
  on this DHT neighborhood. The fetch would gate nothing; dead hashes have
  empty seed blooms AND no live peers.
- scrape:1 request retained (harmless); shadow counters kept for reference.

## Final assessment (this session)

Starting baseline: ~150 verified/hr, memory leaking to OOM, ~11% failures
unclassified.

End state:
- **~213/hr best hour (hour 19), stable ~180/hr** — ~40% improvement
- Memory flat (~110-310 MB, was OOM-bound), RSS stable
- Fetch failure taxonomy: 11% 'other' -> 0.4%
- Tracker resolution (55 public trackers): +20% verified via second peer source
- 8 instances, continuous deeper table growth, get_peers passive intake
- min-seen corroboration gate proven non-viable (twice)

**3000/hr requires multiple egress IPs** (each adds ~2.2k reachable nodes and
~180 verified/hr at current conversion). One IP is a hard ceiling regardless of
code changes; the codebase is now at its single-IP optimum.

## Closing decision — single-IP ceiling accepted (2026-08-14)

The single-IP ceiling was probed one final time with a 1-instance A/B
(`--instances 1 --qps 16000 --scale 16 --max-nodes 8192`, 2h17m, vs the
8-instance baseline) and the conclusion is closed. Reasoning preserved here so
it is not re-opened.

**1. The 1-instance A/B result** (`benchmark/experiments/docker-compose.1inst.yml`,
`benchmark/experiments/instances-ab.sh`):
- A single table reached **2,385 nodes** — the same ceiling, marginally *higher*
  than instance 0's 2,261 in the 8-instance fleet. One table does not reach
  further into the keyspace.
- Verified collapsed to **~25/hr** (vs ~180-213/hr) because sampler loops dropped
  8x (64 vs 512). Verified/hr tracks **sampler-loop throughput against the
  neighborhood, not table size** — both configs held ~2,400 nodes; 8x loops gave
  ~8x verified.
- ⚠️ Confound acknowledged: instance count and total budget moved together
  (per-instance caps `cli.rs:237-248`), so this is not a clean fragmentation
  A/B. But the fragmentation upside is bounded to a ~5% table-size delta either
  way, which cannot close a ~200 → 3000 gap. Closed by diminishing returns, not
  by proof.

**2. Why a shared routing table via IPC would not help** (proposed 2026-08-14):
- The routing table is a *cache of what lookups discover*; the ceiling is the
  discovery feed — the ~2,400 distinct DHT nodes that respond to this one egress
  IP (F11). A shared table is a different container for the same feed; it cannot
  manufacture nodes the feed never returns.
- The 1-instance test already ran "one table everything reads from" and hit the
  same ceiling while verifying 8x *less* (fewer loops).
- Kademlia bucket shape is keyed to the node's own ID, so a shared table either
  forces a shared node ID (destroying the multi-node diversity that drives
  throughput) or merely extends the one-shot bootstrap already done via
  `seed_nodes_from_state` (`crawler.rs:96-103`).
- The only honest benefit is egress-bandwidth dedup (instances 2-7 burned
  ~70-100k queries each to re-discover the same neighborhood), which is not a
  verified/hr lever. Not worth the IPC layer.

**3. Projected yield at the accepted ceiling** (~184-213/hr steady state):
~4,400-5,100/day → **~140-150k verified torrents/month** on one egress IP.
(bitmagnet's ~11M/month is a cumulative lifetime index across multiple egresses
and long uptime, not a steady single-IP rate; still ~75x the structural gap.)

**Decision:** 3000/hr on a single egress IP is closed. The codebase is at its
single-IP optimum; the only remaining multiplier is additional egress IPs
(~2 IPs ≈ 300-400k/month, scaling ~linearly).

## Strategy status

| Strategy | Peers found (empty_peers) | Verifies | Production impact |
|---|---|---|---|
| DHT get_peers (seeded) | low (empty_peers 59%) | ~0.04-0.15% | base ~150/hr |
| Tracker resolution (55) | 100% | ~10% on live, 0% on dead | +~20% verified |
| get_peers passive intake | n/a | same as sampled | volume only |
| min-seen=2 gate | n/a | lower | - (cut 68/hr) |
| lookup-seeded fetch | unchanged | unchanged | neutral |

## Next iterations to try

> Incremental only — none of these break the single-IP ceiling (see Closing
> decision above); each would add a bounded % on top of ~180-213/hr, not order
> of magnitude.

- [ ] Tracker resolution for `timeout`/`other`/`deadline` classes (are any
      recoverable?)
- [ ] Verify tracker-resolved peers via the SAME dial path as prod (currently
      bench dials 8 peers; prod dials up to 16 with parallel 4) — does more
      dialing recover more?
- [ ] Tracker + DHT combined: dial tracker peers first, then DHT peers (prod
      already does tracker-first; measure the delta cleanly)
- [ ] Check whether verified_tracker rate improves with more trackers per hash
      (currently 10 rotating)
- [ ] Announce-path scale: announced hashes verify at ~10% (F1 control) — the
      single highest-conversion source; grow node prominence (table size,
      uptime) to raise announce volume.
