## Context

Memory stability is resolved (`6b4f95c`, shared sampler maps) and discovery is not the constraint (~86-125k unique/hr vs ~150 verified/hr). The wall is **fetch conversion (0.15-0.25%)**. Two measurement-first workstreams:

1. Classify the 11% `other` failure bucket into named causes.
2. Audit why the passive-intake (announce) firehose is nearly dry (~5 announced hashes/min), when announced hashes are live by construction and are the highest-conversion source.

The lesson from prior phases (min_seen=2 collapse in `crawler-liveness-gate`; the ~1.9% announce drain cut in `dht-crawler-node-diversity`) is that **selectivity changes must be measured against live traffic before landing**. This change therefore instruments and classifies first, and only changes behavior in a later phase gated on findings.

## Goals / Non-Goals

**Goals:**
- Name the ~34k/15min `peer_errors_other` bucket into a stable taxonomy persisted per-fetch, with dashboard roll-up.
- Measure the passive-intake funnel (received → validated → deduped → emitted) and the announce rate over time, so we can tell whether the firehose is genuinely growing with uptime (the phase-3 prediction) or suppressed somewhere.
- Provide a concrete, evidence-backed recommendation for raising verified/hr — either a targeted announce fix or a selectivity change.

**Non-Goals:**
- No behavioral change to fetch/discovery in Phases 1-2 (measurement only).
- No `--scale`/instance/Docker/Redis architecture change.
- No schema migration in Phases 1-2.
- No content filtering or metadata matching.

## Decisions

### D63 — Failure taxonomy is a stable enum, not string sniffing
Extend `classify_error`/`classify_peer_error` to a typed `FetchFailureKind` enum with variants:
`timeout`, `connect_refused`, `connection_reset`, `connection_closed`, `handshake_failed` (BEP 10), `no_ut_metadata`, `metadata_rejected`, `parse_error` (bencode/metadata), `sha1_mismatch`, `early_abort`, `deadline`, `empty_peers`, `other`.

- *Rationale:* the current `classify_error` does `msg.contains(...)` on 6 strings and dumps the rest into `other`. A typed enum with one classifier (single source of truth) removes the duplicated logic (`classify_error` + `classify_peer_error` currently disagree on what maps to which counter) and makes the taxonomy visible in one place.
- *Trade-off:* the enum is internal; `failure_reason` in SQLite stays a string for compatibility. A `to_string()`/`as_str()` keeps the DB and dashboard unchanged.

### D64 — Classify at the source, once
Classification happens exactly where the error is produced (in `fetch_from_peer`'s error path / the dial loop), not re-derived from the message string later. The dominant-failure selection (highest count) is unchanged.

- *Rationale:* `fetch_from_peer` is where the actual IO error variant is known; reconstructing from `anyhow`'s formatted message at a distance is lossy (this is why `other` is 11%).
- *Trade-off:* requires threading the typed kind through `FetchError`; modest refactor, pure-observability.

### D65 — Announce funnel counters on the passive-intake path
Add per-stage atomic counters + a rolling rate to `run_passive_intake` and the actor's announce handler:
`announces_received` (actor), `announces_token_rejected` (actor), `announces_suppressed_readonly` (actor), `announces_deduped_redis` (intake), `announces_emitted` (intake). Log the rate per stats tick.

- *Rationale:* with only `hashes_announced` today we cannot tell whether announces are (a) not arriving, (b) rejected at token validation, (c) suppressed by read-only, (d) deduped, or (e) emitted. The funnel pinpoints the loss stage.
- *Trade-off:* a handful of atomics + one log field; negligible cost.

### D66 — Audit node identity/reputation as the announce driver
Verify the node presents a stable BEP 42-compliant ID across restarts (`node_id.json` exists and is used), the routing table is large enough to be noticed (~2,000+ nodes), and inbound DHT traffic is actually reaching the node (firewall/port reachability through the tunnel). Compare the announce rate to the phase-3 "grows with uptime" prediction.

- *Rationale:* bitmagnet's model is that a full participant with a stable ID and a large table attracts announces. If our ID regenerates or our table is small/read-only-ish, the firehose stays dry regardless of the intake code.
- *Trade-off:* the audit may reveal the fix is configuration (persist ID, larger table) rather than code — which is a cheaper, preferred outcome.

### D67 — Behavioral changes are gated, phase-3+ only
Any fix that changes fetch or discovery behavior (announce yield intervention, selectivity tuning) lands only after Phases 1-2 produce measurements, as a follow-on change package.

- *Rationale:* measurement-first is the proven discipline here (min_seen=2, announce-drain lessons). We must not guess the announce suppression cause or tune selectivity without the funnel + taxonomy data.
- *Trade-off:* slower to a fix, but avoids repeating a reverted, unmeasured change.

## Design Notes

- **Where the `other` bucket currently comes from**: `fetch/mod.rs:442` (`Err(_)` JoinHandle failure → "other") and `fetch/mod.rs:458` (`classify_error` fallthrough → "other"). The `peer_errors_other` counter is the aggregate of the latter. D64 moves classification to the error source so these become real variants.
- **Passive-intake funnel stages**: `actor.rs:1450` emits `DhtEvent::Announced` only after token validation (`peer_store.rs:64`) and the read-only gate; `discovery/mod.rs:189` dedupes via Redis before emitting. Each of those is a measurement point.
- **Expected announce funnel shape if healthy**: received ≈ validated ≈ emitted (dedup is small early on). A large `token_rejected` share would be a smoking gun for the dry firehose.

## Phase 2 Audit Findings (2026-08-13)

Live funnel on the deployed build (~6 min warm, cold-start window):
- `announces_received` 61, `token_rejected` 27 (**44%**), `deduped_redis` 20, `emitted` 14.
- Absolute volume is the binding constraint: **~10 announces/min received**, vs ~6,000 sampled hashes/hr. Token rejection amplifies the dry firehose but is not the root cause.
- Node identity (task 2.3): `node_id.json` persisted per instance (`/data/state/instance-N/node_id.json`), stable across restarts, distinct per instance. BEP 42 regeneration is not churning the ID.
- Reachability (task 2.4): DHT ports 6881-6884 bound in the tunnel namespace; inbound UDP queue non-empty (traffic arrives).
- Uptime correlation (task 2.5): firehose remains ~10/min after ~2h uptime; **no evidence the "grows with reputation" prediction is materializing** at this table size (~2,000 nodes/instance).

**Conclusion for Phase 3**: the lever is node prominence (table size / time-in-network / not being sampled into oblivion), not the intake code or tokens. Token `prev_secret` already tolerates one rotation; the 44% rejection is consistent with clients holding tokens across a restart or using stale secrets, and is secondary. Any Phase-3 intervention should target routing-table growth and inbound-query exposure, not the announce handler.
