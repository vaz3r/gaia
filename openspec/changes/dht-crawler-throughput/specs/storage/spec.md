## Purpose

Retry failed hashes fast enough to catch swarms that appear quickly, especially the long tail of hashes with no peers at a given moment.

## ADDED Requirements

### Requirement: Faster exponential backoff base
The backoff sequence SHALL start at 60 seconds (instead of 5 minutes), double per attempt, and cap at 6 hours.

#### Scenario: First retry is quick
- **WHEN** a hash fails for the first time
- **THEN** its next attempt is scheduled 60 seconds later

#### Scenario: Backoff caps
- **WHEN** many attempts accumulate
- **THEN** the retry interval does not exceed 6 hours

### Requirement: Empty-peer hashes retry sooner
The crawler SHALL schedule a retry of a hash that failed with no peers (`empty_peers`) after a fixed short window (60 seconds), independent of the exponential backoff used for other failure reasons.

#### Scenario: Empty-peers hash retries in a minute
- **WHEN** a fetch fails because `get_peers` returned no peers
- **THEN** the hash is retried after 60 seconds even if the exponential schedule would be longer

#### Scenario: Other failures keep exponential backoff
- **WHEN** a fetch fails for a non-empty-peers reason
- **THEN** the standard exponential backoff schedule applies
