## Purpose

Observe a candidate liveness threshold against live traffic before enabling it, and detect window/threshold coupling issues.

## ADDED Requirements

### Requirement: Shadow threshold observation
The CLI SHALL expose `--min-seen-shadow N`; when set, the crawler SHALL log what would be emitted under `min-seen=N` while the live path continues at its current setting. Standalone debug log + counters; no database schema change.

#### Scenario: Shadow counter
- **WHEN** `--min-seen-shadow 3` is set and the live `--min-seen` is 1
- **THEN** the crawler logs `shadow_filtered` (hashes that expired below 3), `shadow_emitted` (hashes that reached 3), and a sample of filtered hashes

### Requirement: Entry lifetime under shadow
An entry's lifetime SHALL be governed by `max(--min-seen, --min-seen-shadow)` so live emission does not delete an entry shadow mode still needs to observe.

#### Scenario: Entry survives live emit
- **WHEN** a hash is emitted at the live threshold (1) while shadow tests 3
- **THEN** its entry stays and continues accumulating reports toward 3

#### Scenario: Shadow threshold reached
- **WHEN** the hash reaches the shadow threshold
- **THEN** it is counted as `shadow_emitted` and its entry is removed

### Requirement: Near-miss bucketing
The crawler SHALL bucket expired entries by max distinct sources reached (`shadow_near_miss_1`, `shadow_near_miss_2`), so window/threshold coupling (e.g. STALE_BACKOFF erasing two early sightings before a third lands) is detectable.

#### Scenario: Near-miss detection
- **WHEN** a hash reaches 2 distinct sources then expires before a 3rd
- **THEN** it increments `shadow_near_miss_2`, signalling a possible window-edge effect

## MODIFIED Requirements

### Requirement: Window tuning guidance
If near-miss counters cluster at the window edge, the fix SHALL be to tune `--liveness-window`, not to loosen `--min-seen`, so a windowing problem is not masked by a threshold change.
