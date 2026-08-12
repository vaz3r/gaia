## Purpose

Cut dead-peer TCP churn ~4x while keeping live-hash verification unchanged.

## ADDED Requirements

### Requirement: Tightened dial budgets
The fetch layer SHALL use `PARALLEL_DIALS=4`, `MAX_PEERS_PER_HASH=16`, `FETCH_TIMEOUT=3s`, and `EARLY_ABORT_DIALS=24` so failed fetches consume ~4x less connection churn.

#### Scenario: Fewer parallel dials
- **WHEN** a fetch dials peers for a hash
- **THEN** at most 4 peers are dialed concurrently (was 16)

#### Scenario: Lower peer cap
- **WHEN** a fetch iterates get_peers results
- **THEN** it tries at most 16 peers per hash (was 50)

#### Scenario: Faster dead-peer release
- **WHEN** a dialed peer does not respond
- **THEN** it times out after 3s (was 5s), freeing the slot sooner

#### Scenario: Earlier dead-hash abort
- **WHEN** 24 consecutive connect failures occur with no handshake
- **THEN** the hash is aborted as dead (was 64)

## MODIFIED Requirements

### Requirement: Verification unchanged
Live hashes SHALL still verify: the first live peer among the first dials wins, so reduced parallelism does not reduce the verified rate.

#### Scenario: Live hash still verified
- **WHEN** a hash has at least one live peer
- **THEN** the fetch succeeds despite the tighter budgets
