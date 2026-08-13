# Tasks

## 1. Phase 1 — fetch failure taxonomy (D63, D64)

- [ ] 1.1 Introduce `FetchFailureKind` enum (timeout, connect_refused, connection_reset, connection_closed, handshake_failed, no_ut_metadata, metadata_rejected, parse_error, sha1_mismatch, early_abort, deadline, empty_peers, other) with a single `from_error(&anyhow::Error)` classifier
- [ ] 1.2 Thread the typed kind through `FetchError` so classification happens at the error source in `fetch_from_peer`/the dial loop (replace the two string-sniffing `classify_error` + `classify_peer_error` fns)
- [ ] 1.3 Keep `failure_reason` in SQLite as the kind's string form (no schema change); dominant-failure selection unchanged
- [ ] 1.4 Add per-kind atomic counters to `CrawlStats` + a "peer failure breakdown" log line including the new buckets (connection_reset, connection_closed, handshake_failed, parse_error)
- [ ] 1.5 Add unit tests for `from_error` mapping each distinct message string to the expected kind
- [ ] 1.6 Run 24h on live traffic; compare `other` share before/after (target: `other` drops from ~11% to <3%, the rest lands in named buckets)

## 2. Phase 2 — announce volume audit (D65, D66)

- [ ] 2.1 Add actor-side counters: `announces_received`, `announces_token_rejected`, `announces_suppressed_readonly` at the announce handler (`actor.rs:1421-1453`)
- [ ] 2.2 Add intake-side counters: `announces_deduped_redis`, `announces_emitted` in `run_passive_intake`; log a rolling rate per stats tick
- [ ] 2.3 Verify `node_id.json` is persisted per instance and the ID is stable across restarts (BEP 42-compliant)
- [ ] 2.4 Verify inbound DHT reachability (announce/query traffic actually arrives through the tunnel on the DHT ports)
- [ ] 2.5 Correlate announce rate vs uptime over 24h to test the "grows with reputation" prediction (D66)
- [ ] 2.6 Produce a written finding: where in the funnel announces are lost, and the recommended Phase-3 fix (config vs code)

## 3. Phase 3 — announce yield fix (gated on Phase 2 findings)

- [ ] 3.1 Implement the audited fix (likely: stable identity/larger table config, or token handling, or a second ingestion path)
- [ ] 3.2 Verify with the funnel counters that emitted announces/rate rises
- [ ] 3.3 Measure verified/hr delta vs the Phase 1-2 baseline; confirm no regression in sampling discovery

## 4. Phase 4 — selective conversion tuning (gated on 1-3 results)

- [ ] 4.1 Only if `other` reveals a fixable subclass or announce plateaus below target: re-open liveness-gate/selectivity with measured thresholds (reference D57-D62 shadow data)
- [ ] 4.2 Never change a selectivity default without a shadow/AB measurement

## 5. Verify

- [ ] 5.1 `cargo test` + `cargo clippy --all-targets -- -D warnings` clean after each phase
- [ ] 5.2 Dashboard (`benchmark/liveness.sh`) shows the new failure buckets
- [ ] 5.3 Confirmed no behavior change in Phases 1-2 (verified/hr, unique/hr within noise of baseline)
