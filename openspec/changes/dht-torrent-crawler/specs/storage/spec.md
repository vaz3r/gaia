## Purpose

Persists accepted torrent records to a local SQLite database with upsert semantics, indexes them for name search, and records observation history (first/last seen) so the crawler never duplicates work and the dataset grows monotonically across restarts.

## ADDED Requirements

### Requirement: Torrent record persistence
The storage layer SHALL persist each accepted torrent as a row keyed by the 20-byte `info_hash`, storing the name, category (`movie`/`tv`), title, year, season, episode, total size in bytes, file count, first-seen timestamp, and last-seen timestamp.

#### Scenario: New torrent is inserted
- **WHEN** the pipeline delivers a previously unseen accepted torrent
- **THEN** a row is inserted with `first_seen` and `last_seen` both set to the current time

#### Scenario: Invalid category is rejected
- **WHEN** a record arrives whose category is neither `movie` nor `tv`
- **THEN** the storage layer rejects it and no row is written

### Requirement: Upsert semantics
The storage layer SHALL treat the `info_hash` as the primary key and, on a duplicate, update the mutable observation fields while preserving the original `first_seen`.

#### Scenario: Re-observed torrent bumps last_seen
- **WHEN** a torrent already present in the database is delivered again
- **THEN** `last_seen` is updated to the new observation time and `first_seen` is left unchanged

#### Scenario: First-seen is immutable
- **WHEN** a torrent is observed a second time
- **THEN** the stored `first_seen` value does not change

### Requirement: Durable writes with WAL
The storage layer SHALL open the SQLite database in WAL mode so that the crawler daemon can read and write concurrently without blocking, and commits SHALL be batched to bound write frequency.

#### Scenario: Writes batch without data loss
- **WHEN** many accepted torrents arrive within a short window
- **THEN** they are committed in batched transactions, and a hard exit does not lose already-committed batches

### Requirement: Name search
The storage layer SHALL support case-insensitive substring search over the normalized name field.

#### Scenario: Substring query returns matches
- **WHEN** a search for `matrix` is issued against a database containing `The Matrix 1999`
- **THEN** the matching row is returned by the query interface

#### Scenario: Non-matching query returns empty
- **WHEN** a search for a term with no matches is issued
- **THEN** an empty result set is returned without error

### Requirement: Seen-hash dedup for crawler
The storage layer SHALL expose a membership check so the crawler can test whether an infohash is already persisted, enabling the pipeline to skip re-crawling and re-fetching known torrents.

#### Scenario: Known hash reported as seen
- **WHEN** the crawler checks an infohash that exists as a table row
- **THEN** the membership check returns true and the pipeline skips it

#### Scenario: Unknown hash reported as unseen
- **WHEN** the crawler checks an infohash that is not present
- **THEN** the membership check returns false and the pipeline proceeds with the fetch