## Purpose

Classifies extracted torrent release names as `movie` or `tv` (skipping everything else) and extracts title metadata (year, season, episode). The classification is deterministic, run on normalized names, and is what keeps the index lightweight — only movie/TV torrents are persisted.

## ADDED Requirements

### Requirement: Deterministic name normalization
The filter SHALL normalize a release name before classification by replacing punctuation with spaces, collapsing whitespace, and lowercasing, such that the same input always yields the same normalized form and result.

#### Scenario: Punctuation becomes spaces
- **WHEN** a name like `The.Matrix.1999.1080p.BluRay.x264-YIFY` is normalized
- **THEN** it becomes the lowercased, space-separated token stream `the matrix 1999 1080p bluray x264-yify`

#### Scenario: Deterministic output
- **WHEN** the same raw name is classified twice
- **THEN** both runs produce the identical category and extracted metadata

### Requirement: Movie classification
The filter SHALL classify a name as `movie` only when it contains a four-digit year in `19xx`–`20xx` range AND a quality/container tag (e.g. `1080p`, `720p`, `2160p`, `4k`, `bluray`, `brrip`, `web-dl`, `webrip`, `hdtv`, `dvdrip`).

#### Scenario: Classic movie release
- **WHEN** the name is `The Matrix 1999 1080p BluRay x264 YIFY`
- **THEN** the filter classifies it as `movie` and extracts year `1999`

#### Scenario: Movie without quality tag is rejected
- **WHEN** the name is `Some Cool Film 1998` (no quality/container tag)
- **THEN** the filter does NOT classify it as `movie` and it is treated as a skip

### Requirement: TV classification
The filter SHALL classify a name as `tv` only when it contains a clear season/episode marker such as `SxxExx`, `Season N Episode N`, or a recognized `SxNN` single-digit-season form.

#### Scenario: Standard episode marker
- **WHEN** the name is `Breaking Bad S05E09 1080p HDTV x264`
- **THEN** the filter classifies it as `tv` and extracts season `5` and episode `9`

#### Scenario: Multi-episode season marker
- **WHEN** the name is `Game of Thrones Season 1 Complete 720p`
- **THEN** the filter classifies it as `tv` and extracts season `1` with no episode

### Requirement: Other content is skipped
The filter SHALL classify names that match neither movie nor TV criteria — including music, software, games, and adult content — as `skip`, and SHALL NOT pass them to the storage layer.

#### Scenario: Software release is skipped
- **WHEN** the name is `Adobe Photoshop 2024 macOS` with no movie/TV markers
- **THEN** the filter classifies it as `skip` and it is not stored

### Requirement: Extracted metadata fields
For accepted names, the filter SHALL extract and emit the clean title, and the year, season, and episode values when present, leaving absent numeric fields as `NULL`.

#### Scenario: Title is cleaned of tags
- **WHEN** the movie `The Matrix 1999 1080p BluRay x264 YIFY` is accepted
- **THEN** the emitted title is the tag-stripped `the matrix` with year `1999`