## Purpose

Persists torrents as a generic index: the `torrents` table stores torrent metadata only (infohash, name, size, file count, observation timestamps) with no media taxonomy, while the raw bencoded `info` dictionary remains in the `scanned` table so classification and other enrichment can be re-derived later.

## ADDED Requirements

### Requirement: Torrent-metadata-only schema
The `torrents` table SHALL store exactly: `info_hash BLOB PRIMARY KEY`, `name TEXT NOT NULL`, `size_bytes INTEGER`, `file_count INTEGER`, `first_seen INTEGER NOT NULL`, `last_seen INTEGER NOT NULL`. It SHALL NOT store media-specific columns (category, title, year, season, episode).

#### Scenario: New torrent is inserted
- **WHEN** the pipeline delivers a previously unseen verified torrent
- **THEN** a row is inserted with `first_seen` and `last_seen` set to the current time and its size/file count populated

#### Scenario: No media columns exist
- **WHEN** the schema is inspected
- **THEN** `torrents` has no `category`, `title`, `year`, `season`, or `episode` column

### Requirement: Upsert semantics
The storage layer SHALL treat `info_hash` as the primary key and, on a duplicate, update mutable fields (name, size, file count, last_seen) while preserving the original `first_seen`.

#### Scenario: Re-observed torrent bumps last_seen
- **WHEN** a torrent already present is delivered again
- **THEN** `last_seen` (and name/size/file_count) are updated and `first_seen` is unchanged

### Requirement: Raw info dictionary preserved
The storage layer SHALL keep the raw bencoded `info` dictionary for every accepted torrent in the `scanned` table's `info_bytes` column, so classification and any future enrichment can be re-run offline.

#### Scenario: Accepted torrent retains info_bytes
- **WHEN** a torrent is accepted and persisted
- **THEN** the corresponding `scanned` row stores its raw `info_bytes`

### Requirement: Schema migration
The storage layer SHALL migrate an existing pre-change database by rebuilding `torrents` without the media columns, copying `info_hash`, `name`, `size_bytes`, `file_count`, `first_seen`, and `last_seen`, and leaving the `scanned` table intact.

#### Scenario: Existing rows survive migration
- **WHEN** a database created under the old (media-column) schema is opened
- **THEN** all existing torrent rows remain with their torrent metadata intact and the media columns are gone

### Requirement: Name search
The storage layer SHALL support case-insensitive substring search over the name field.

#### Scenario: Substring query returns matches
- **WHEN** a search for `matrix` is issued against a database containing `The Matrix 1999`
- **THEN** the matching row is returned

#### Scenario: Non-matching query returns empty
- **WHEN** a search has no matches
- **THEN** an empty result set is returned without error
