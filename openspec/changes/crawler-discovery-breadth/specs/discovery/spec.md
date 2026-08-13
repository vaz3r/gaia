## Purpose

Raise distinct-node sampling toward the ~73/sec ceiling (from ~12/sec) by fixing the inverted backoff and widening node spread.

## ADDED Requirements

### Requirement: Backoff inversion
The sampler SHALL re-query a healthy node that returned 0 new hashes after a short backoff (60s), and a node that did not respond after a longer backoff (30s).

#### Scenario: Healthy exhausted node re-queried soon
- **WHEN** a node responds with 0 new hashes
- **THEN** it is re-queryable after 60s (not 300s)

#### Scenario: Non-responsive node backed off harder
- **WHEN** a node times out or errors
- **THEN** it is retried after 30s (not 10s)

### Requirement: Stale graduation
The long backoff (300s) SHALL apply only after 3 consecutive 0-new responses; a response with new hashes resets the counter.

#### Scenario: First 0-new does not shelve
- **WHEN** a productive node returns 0 new once
- **THEN** it keeps its short backoff status

#### Scenario: Persistent emptiness graduates
- **WHEN** a node returns 0 new 3 consecutive times
- **THEN** it is deprioritized for 300s

### Requirement: Rotating node spread
The sampler SHALL maintain a per-loop cursor and rotate the ready-node list by it each pick, so consecutive selections cycle through the full routing table.

#### Scenario: Loops cover the table
- **WHEN** the routing table has many ready nodes
- **THEN** successive picks advance through the table rather than re-selecting the same few

## MODIFIED Requirements

### Requirement: Unique discovery rate
The distinct-node sampling rate SHALL rise toward the ~73/sec ceiling, lifting unique hashes/hr.

#### Scenario: Higher utilization
- **WHEN** the backoff inversion and cursor are deployed
- **THEN** hashes_sampled-derived node sampling increases from ~12/sec toward ~73/sec
