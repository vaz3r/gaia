## Purpose

Fetches full torrent metadata for sampled infohashes over TCP using the BitTorrent extension protocol (BEP 10) and `ut_metadata` (BEP 9), verifies the assembled metadata by SHA-1, and extracts the torrent name, file list, and total size. This is the only source of human-readable names — BEP 51 responses carry no names.

## ADDED Requirements

### Requirement: Fetch metadata via BEP 9
The enrichment layer SHALL, for a queued infohash, discover peers via DHT `get_peers`, connect to a peer over TCP, perform the BitTorrent handshake and extension handshake (BEP 10), and request the metadata pieces via `ut_metadata` (BEP 9) until the full metadata is assembled.

#### Scenario: Successful metadata download
- **WHEN** a sampled infohash is processed and a reachable peer advertises `ut_metadata`
- **THEN** the crawler downloads all metadata pieces and assembles the complete bencoded info dictionary

#### Scenario: Peer does not support ut_metadata
- **WHEN** a peer's extension handshake omits `ut_metadata` or reports `metadata_size` when the peer is unreachable
- **THEN** the fetcher tries the next known peer for the infohash, or marks the hash failed only after all peers are exhausted

### Requirement: Metadata integrity verification
The enrichment layer SHALL compute the SHA-1 hash of the assembled bencoded `info` dictionary and accept it only if it matches the sampled infohash.

#### Scenario: Matching hash is persisted
- **WHEN** the computed SHA-1 of the assembled info dictionary equals the sampled infohash
- **THEN** the metadata is accepted and passed to the extractor

#### Scenario: Mismatched hash is rejected
- **WHEN** the computed SHA-1 differs from the sampled infohash
- **THEN** the infohash is rejected and no partial data is persisted for it

### Requirement: Metadata extraction
The enrichment layer SHALL parse the verified bencoded metadata and extract the torrent `name`, the (optional) file list with per-file byte sizes, and the total size.

#### Scenario: Single-file torrent
- **WHEN** the verified metadata contains a single file under `name`
- **THEN** the extractor records that name and a total size equal to the file length

#### Scenario: Multi-file torrent
- **WHEN** the verified metadata contains a `files` list
- **THEN** the extractor records the top-level name, the per-file lengths, and a total size summed across all files

### Requirement: Bounded fetch concurrency
The enrichment layer SHALL enforce a configurable maximum number of concurrent in-flight metadata fetches so that failed or slow peer connections cannot exhaust file descriptors or memory.

#### Scenario: Concurrency cap is respected
- **WHEN** the number of active metadata fetches would exceed the configured maximum
- **THEN** additional infohashes wait in a bounded queue until an in-flight fetch completes

#### Scenario: Queue overflow is handled
- **WHEN** the bounded queue fills faster than fetches complete
- **THEN** the enrichment layer logs the overflow and drops the overflow indicator without crashing the pipeline

### Requirement: Skip previously-seen infohashes
The enrichment layer SHALL skip metadata fetches for infohashes already persisted in the database or already known to have failed recently, to avoid wasted fetches.

#### Scenario: Persisted hashes are not refetched
- **WHEN** an infohash already present in the database is handed to the enrichment layer
- **THEN** the fetch is skipped and no network activity is performed for it

#### Scenario: Recent failures back off
- **WHEN** an infohash failed to fetch metadata within the configured retry window
- **THEN** the enrichment layer does not retry it before the window elapses