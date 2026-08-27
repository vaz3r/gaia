## Remaining Crawler Architecture Roadmap

Based on our intensive codebase reviews, the metrics from `health.sh`, and the uncommitted workspace state, here is the master table of features and fixes that we have scoped and planned, but have not yet executed.

### Phase 1: Pipeline Unblocking & Stabilization (Immediate ROI)
These fixes address the immediate bottlenecks saturating the `fresh_channel_depth` and wasting worker cycles.

| Feature / Fix | Description | Impact | Artifact / Status |
| :--- | :--- | :--- | :--- |
| **Commit Retry Storm Fix** | Commit the `no_metadata_max_retries = 1` logic currently sitting uncommitted in the workspace. | Structurally prevents the `no_metadata` retry storm from accumulating. | Scoped in `plan_retry_storm.md` |
| **DB Pruning Execution** | Run a one-time `psql` query to kill the ~263k phantom jobs stuck in the retry loop. | Instantly clears the backlog, allowing the crawler to process fresh/valid retries. | Scoped in `plan_retry_storm.md` |
| **TCP / uTP Concurrent Race** | Replace sequential fallback in `try_fetch` with a `tokio::select!` that races them but waits out early failures. | Eliminates the 5s sequential wait penalty on the 98.4% of peers that timeout on uTP. | Scoped in `plan_retry_storm.md` |
| **Fix `_permit` Scope Bug** | Drop the per-IP `limiter.acquire()` permit *before* `fetch_metadata` starts in `try_fetch`. | Currently, the permit is held for the full 25s download, starving the IP limiter. Dropping it early fixes this. | Scoped in `plan_two_stage.md` |

### Phase 2: Pipeline Concurrency & Quality (Throughput Multipliers)
These features fundamentally change how work is queued and filtered, aiming to push the crawler past the 30k/hr ceiling.

| Feature / Fix | Description | Impact | Artifact / Status |
| :--- | :--- | :--- | :--- |
| **Peer Reputation Filter** | Add `if cache.is_bad(addr)` inside `source_peers` before yielding peers to the fetch pool. | Forces the DHT to keep searching for *alive* peers, bypassing 92% of dead nodes natively. | Scoped in `plan_two_stage.md` |
| **Two-Stage Pipeline** | Add `pipeline_limit=15000` to bound DHT lookups, and move the 1,200 `fetch_limit` into `verify_infohash` just before TCP connect. | Unblocks the pipeline: thousands of infohashes can search DHT concurrently without starving the 1,200 TCP sockets. | Scoped in `plan_two_stage.md` |

### Phase 3: Active Sybil Weaponization (Discovery Engine)
These features transform the crawler from a passive observer to an active DHT mapper to radically increase high-quality inbound traffic.

| Feature / Fix | Description | Impact | Artifact / Status |
| :--- | :--- | :--- | :--- |
| **Multi-Sybil Routing Table** | Overhaul `RoutingTable` to maintain an array of 128 routing tables (one for each Sybil ID). | Retains hundreds of thousands of discovered nodes globally, rather than dropping them at `K=8`. | Scoped in `plan_sybil_bep51.md` |
| **Sybil Swarm Outbound** | Increase Sybils to 128. Rotate them aggressively in `walker.rs` and `peer_source.rs`. | Seeds 128 identities into remote tables, attracting massive `announce_peer` traffic. | Scoped in `plan_sybil_bep51.md` |
| **Proximity Sybil Routing** | Make `source_peers` start DHT lookups using the specific Sybil ID closest to the target infohash. | Starts lookups immediately adjacent to the target, cutting lookup hops from 15s to ~500ms. | Scoped in `plan_sybil_bep51.md` |
| **BEP-51 Infohash Sampler** | New background loop (`bep51.rs`) querying high-uptime nodes for `sample_infohashes`. | Pumps raw, actively-seeded infohashes straight into the `fresh_verify_tx` channel. | Scoped in `plan_sybil_bep51.md` |

---

## Next Steps
All of these items have detailed, step-by-step implementation plans already written and approved (or pending approval). 

**Recommended Action:** Start with **Phase 1 (Retry Storm & uTP Race)** to immediately stabilize the retry queue and free up TCP/uTP connection time, followed immediately by the **Two-Stage Pipeline** to unleash the full concurrency of the hardware.
