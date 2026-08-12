## Purpose

Describe the process/CLI changes: a `--scale` concurrency knob wired through the crawler, compose, and pipeline buffers.

## ADDED Requirements

### Requirement: Scale flag
The CLI SHALL expose `--scale N` (default 10) that multiplies sampler QPS, sampler loops, fetch concurrency, lookup concurrency, and channel buffer sizes.

#### Scenario: Default scale
- **WHEN** no `--scale` is provided
- **THEN** budgets use scale=10 (bitmagnet's baseline)

#### Scenario: Aggressive day-one scale
- **WHEN** `--scale 50` is set
- **THEN** all concurrency and buffers multiply by 50 for maximum aggregation

### Requirement: Compose wired
The compose file SHALL pass `--scale` (default 10) so the deployed stack uses the new budgets.

#### Scenario: Deployed scale matches CLI default
- **WHEN** compose starts the crawler
- **THEN** it passes `--scale 10` (or an explicit override)
