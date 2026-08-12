## Purpose

Route announced hashes through the fetch pipeline with a live-peer dial hint so the highest-yield hashes skip discovery and go straight to a peer dial.

## ADDED Requirements

### Requirement: Announce-derived fetch requests
The crawler SHALL run a passive-intake loop per instance subscribing to `DhtEvent`; each `Announced` event SHALL become a `FetchRequest { hash, occurrences: 1, peer_hint: Some(peer_addr) }`, subject to Redis seen-set dedup, and SHALL be forwarded to the fetch pipeline.

#### Scenario: Announce becomes a hinted request
- **WHEN** the intake loop receives `Announced { info_hash, peer_addr }` not already seen
- **THEN** a `FetchRequest` with `peer_hint: Some(peer_addr)` is sent to the pipeline

#### Scenario: Already-seen announce dropped
- **WHEN** the hash is already in the shared seen-set
- **THEN** the intake loop skips it without emitting

### Requirement: Hint-first dialing
The fetch layer SHALL dial a request's `peer_hint` before any `get_peers` lookup (after blocklist and dead-peer checks). A SHA-1-verified metadata result from the hinted peer SHALL complete the fetch without any discovery traffic; otherwise the fetch SHALL fall back to the normal `get_peers` path.

#### Scenario: Hint verifies immediately
- **WHEN** the hinted peer answers with SHA-1-verified metadata
- **THEN** the fetch succeeds without a `get_peers` lookup

#### Scenario: Hint fails
- **WHEN** the hinted peer is unreachable or serves mismatched metadata
- **THEN** the fetch falls back to `get_peers` and continues as before

### Requirement: Hint priority in the queue
The fetch queue SHALL order hinted requests above sampled ones (heap key `(hinted, occurrences, hash)`), so live announced hashes are fetched before sampled hashes of similar popularity.

#### Scenario: Hinted request dequeued first
- **WHEN** a hinted request and a sampled request are both queued
- **THEN** the hinted request is dequeued first regardless of occurrence count
