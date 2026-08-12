## Purpose

Describe the CLI/process changes: runtime-tunable liveness flags and the shared counter wiring.

## ADDED Requirements

### Requirement: Runtime liveness flags
The CLI SHALL expose `--liveness-window` (120s), `--liveness-cap` (8 distinct sources), `--liveness-max-entries` (100k), and `--min-seen-shadow` (0 = off) as runtime flags, so tuning during the shadow phase does not require a rebuild.

#### Scenario: Tuning without rebuild
- **WHEN** the shadow run shows near-miss clustering at the window edge
- **THEN** `--liveness-window` is adjusted via compose without recompiling

### Requirement: Shared counter wiring
The process SHALL create the liveness counter once and clone it into every sampler (the `SharedBloom` pattern at `crawler.rs:101`).

#### Scenario: One counter per process
- **WHEN** the crawler starts with N instances
- **THEN** all N instances' sampler loops share one liveness counter, so distinct sources are counted across instances in-process

### Requirement: Sweep task
The process SHALL run a periodic sweep task enforcing the global backstop (`--liveness-max-entries`).

#### Scenario: Backstop enforced
- **WHEN** the counter grows past the backstop
- **THEN** oldest entries are evicted even if never re-read
