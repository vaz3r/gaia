# fetch spec — failure taxonomy (D63, D64)

## Scope

Classify the ~11% `other` fetch-failure bucket into named causes without changing fetch behavior.

## Requirements

- `FetchFailureKind` enum with variants: `timeout`, `connect_refused`, `connection_reset`, `connection_closed`, `handshake_failed`, `no_ut_metadata`, `metadata_rejected`, `parse_error`, `sha1_mismatch`, `early_abort`, `deadline`, `empty_peers`, `other`.
- One classifier `FetchFailureKind::from_error(&anyhow::Error) -> FetchFailureKind` used both for the DB `failure_reason` string and the atomic counters.
- Classification happens at the error source (`fetch_from_peer` / the dial loop), not via `msg.contains` on the formatted string later.
- SQLite `failure_reason` remains a string (the kind's `as_str()`), no schema change.
- Dominant-failure selection (highest per-hash count wins) unchanged.

## Acceptance

- `other` share drops from ~11% to <3% over a 24h live run; the remainder is distributed across named buckets.
- New buckets visible in the "peer failure breakdown" log line and `benchmark/liveness.sh`.
- `cargo test` covers `from_error` mapping for each distinct error message.
