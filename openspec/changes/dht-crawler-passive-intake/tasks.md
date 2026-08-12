# Tasks

## 1. Phase 0 — absorb irontide as gaia-* crates (D39)

- [x] 1.1 Copy irontide-bencode/core/wire/dht into `vendor/gaia-*`
- [x] 1.2 Rename packages + lib names; rewrite internal `use irontide_*` → `gaia_*`
- [x] 1.3 Convert inter-crate deps to `path` deps; add to workspace members
- [x] 1.4 Update crawler `Cargo.toml` + source imports to `gaia-*`
- [x] 1.5 Verify all 4 crates' test suites pass (250 dht, 223 core, etc.)

## 2. Phase 1 — inbound event stream (D40)

- [x] 2.1 Add `DhtEvent` enum + `broadcast` channel to `DhtHandle` and `DhtActor`
- [x] 2.2 Emit `Announced` in `handle_query` announce_peer arm; `LookedUp` in get_peers arm
- [x] 2.3 Export `DhtEvent`; update test call sites for new `DhtActor::new` arg
- [x] 2.4 Clippy-clean doc comments in gaia-dht

## 3. Phase 2 — announce-first fetch path (D41)

- [x] 3.1 Add `FetchRequest { hash, occurrences, peer_hint }` in discovery
- [x] 3.2 `run_passive_intake` subscribes per instance, dedups via Redis, emits hinted requests
- [x] 3.3 `HashQueue` prioritizes hinted requests (heap key `(hinted, occurrences, hash)`)
- [x] 3.4 `fetch_one` takes `peer_hint`, dials it first, falls back to get_peers
- [x] 3.5 Sampler sends `FetchRequest` (no hint); remove dead `SampledHash`
- [x] 3.6 Unit tests for queue prioritization

## 4. Phase 3 — stable node identity + table growth (D42, D43)

- [x] 4.1 `load_or_create_node_id` persists per-instance `node_id.json`
- [x] 4.2 Pass `DhtConfig::own_id: Some(id)`
- [x] 4.3 Compose: `--max-nodes 8192`, `--no-restrict-ips`

## 5. Phase 4 — get_peers PutHash reuse

- [ ] 5.1 DEFERRED: hash→peers reuse from our own lookups (follow-up change)

## 6. Integration, deploy, verify

- [x] 6.1 `cargo test` (34) + `cargo clippy -D warnings` clean + release build
- [ ] 6.2 Write openspec change package
- [ ] 6.3 Deploy to remote-dev; benchmark vs the ~500-700/day baseline
- [ ] 6.4 Measure announce-derived fetch success rate + torrents/day over 24h
