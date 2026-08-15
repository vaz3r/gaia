# Search Specification

## Purpose

Instant fuzzy search over verified torrent names, with filters, sorting, and pagination, backed by PostgreSQL trigram indexes.

## Requirements

### Requirement: Fuzzy name search
The search API SHALL return torrents whose name fuzzy-matches the query using trigram similarity, ranked by relevance (best match first by default).

#### Scenario: Typo-tolerant match
- **WHEN** a query slightly misspells a stored name (e.g. "madix" for "Matrix")
- **THEN** the torrent is returned among the results via trigram similarity

#### Scenario: Default relevance ordering
- **WHEN** a search query returns multiple results without an explicit sort
- **THEN** results are ordered by descending similarity to the query

### Requirement: Instant response
The search API SHALL return results quickly for interactive use: a single-keyword search over the current dataset SHALL complete well under interactive latency using the trigram GIN index (no full-table scan for common prefixes).

#### Scenario: Index-backed query
- **WHEN** a search executes
- **THEN** it uses the trigram index path for similarity ranking rather than a sequential scan

### Requirement: Filters and sorting
The search API SHALL support filtering by minimum size, maximum size, minimum file count, and minimum first-seen date, and SHALL support sorting by relevance, newest, largest, and name, each ascending or descending.

#### Scenario: Size and age filters applied
- **WHEN** a query includes a size range and a minimum first-seen date
- **THEN** only torrents within the size range and seen after the date are returned

#### Scenario: Sort applied
- **WHEN** a sort of "newest" is requested
- **THEN** results are ordered by first_seen descending

### Requirement: Pagination
The search API SHALL paginate results with a stable cursor so pages do not shift when new torrents arrive between requests.

#### Scenario: Keyset pagination
- **WHEN** the client requests the next page using the returned cursor
- **THEN** it receives results strictly after the previous page's boundary

### Requirement: Validated query parameters
The search API SHALL reject malformed query parameters (non-numeric filters, unsupported sort keys) with a 400 error and a clear message.

#### Scenario: Bad sort key rejected
- **WHEN** a request specifies an unsupported sort key
- **THEN** the API responds 400 and names the offending parameter
