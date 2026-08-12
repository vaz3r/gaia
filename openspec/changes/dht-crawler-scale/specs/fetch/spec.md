## Purpose

Raise fetch concurrency and selectivity to match bitmagnet: a `--scale` knob and get_peers-first dialing so we never burn dials on empty lookups.

## ADDED Requirements

### Requirement: Scale knob
The fetch layer SHALL scale its concurrency and lookup budgets by a `--scale` factor (default 10, matching bitmagnet's `scaling_factor`), so concurrency can be raised to match discovery throughput.

#### Scenario: Scale applied
- **WHEN** `--scale` is set
- **THEN** fetch concurrency, lookup concurrency, and pipeline buffers multiply accordingly

### Requirement: get_peers-first selectivity
The fetch layer SHALL only dial peers when `get_peers` returned confirmed live values; an empty lookup SHALL fail fast as `empty_peers` rather than burning dials.

#### Scenario: Empty lookup fails fast
- **WHEN** `get_peers` for a hash returns no live values
- **THEN** the fetch is recorded as `empty_peers` without dialing any peer

#### Scenario: Hint path exempt
- **WHEN** a hash arrived with a live announce hint
- **THEN** the hinted peer is still dialed directly before any get_peers lookup

## MODIFIED Requirements

### Requirement: Fetch pool saturation
The fetch pool SHALL stay saturated as discovery scales; with higher concurrency and selectivity, records_persisted/hr SHALL rise with the larger stream.
