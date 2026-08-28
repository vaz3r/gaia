# Gaia Agent Guidelines

Gaia is a high-throughput BitTorrent DHT crawler written in Rust. Follow these rules when modifying the project.

## General Workflow

1. Read the relevant code and tests before editing.
2. Check `git status --short` and preserve unrelated changes.
3. Make the smallest change that solves the task.
4. Do not rename public APIs, config keys, metrics, or database fields unless required.
5. Add or update tests for changed behavior.
6. Run formatting, checks, lints, and tests before finishing.

## Rust Standards

- Use stable, idiomatic Rust.
- Prefer clear ownership and borrowing over unnecessary cloning.
- Avoid `unwrap()` and `expect()` on network, database, config, or filesystem input.
- Return useful errors with context.
- Do not add `unsafe` unless absolutely necessary and explicitly requested.
- Do not add dependencies when the standard library or an existing crate is sufficient.
- Keep functions focused and names descriptive.
- Preserve compatibility with `linux/amd64` and `linux/arm64` when possible.

## Tokio and Concurrency

- Never block a Tokio worker thread.
- Keep database and filesystem I/O out of the UDP hot path.
- Use bounded channels and define what happens when they are full.
- Do not spawn an unbounded task per packet, peer, or infohash.
- Do not hold a mutex guard across `.await`.
- Give long-running tasks a shutdown path.
- Use timeouts for network and database operations.
- Use bounded exponential backoff for retry loops.
- Avoid busy loops and immediate retries after failures.

## DHT Packet Handling

Treat every packet as untrusted.

- Validate packet size before parsing.
- Bound bencode nesting, list sizes, string sizes, and allocations.
- Validate transaction IDs, node IDs, infohashes, tokens, and compact node formats.
- Malformed packets must return an error or be dropped without panicking.
- Keep `handle_datagram()` and equivalent handlers synchronous and non-blocking.
- Preserve `try_send()` and non-blocking socket behavior in the hot path.
- Avoid per-packet info-level logging.
- Do not create responses significantly larger than requests without a protocol reason.

## DHT Protocol Correctness

- Follow BEP-5 semantics for `ping`, `find_node`, `get_peers`, and `announce_peer`.
- Preserve transaction ID matching.
- Preserve XOR-distance ordering.
- Return only valid compact node and peer data.
- Keep token generation and verification tied to the requester IP.
- Preserve current-secret and previous-secret token validation windows.
- Do not change external IP, port, Node ID, or announcement behavior silently.
- Keep DHT discovery, metadata fetching, and peer transfer as separate subsystems.

## Routing Table

- Keep bucket sizes bounded.
- Preserve good, questionable, and bad node handling.
- Deduplicate nodes consistently.
- Do not allow routing state to grow without a configured limit.
- Avoid sorting the entire table when only the closest few nodes are required.
- Prefer top-k selection or bounded candidate sets for hot lookups.
- Do not duplicate every node into multiple tables unless the task explicitly requires it and memory cost is measured.

Routing-table changes should test:

- Bucket selection
- XOR ordering
- Capacity limits
- Deduplication
- Node replacement
- Liveness transitions
- Closest-node results

## Performance

Gaia processes many small UDP packets. Optimize for packet cost, not only Mbps.

- Avoid allocations and copies in the hot path.
- Reuse buffers where practical.
- Avoid repeated full sorts.
- Keep lock contention low.
- Batch database writes.
- Keep logging buffered and bounded.
- Measure before claiming a performance improvement.

For hot-path changes, compare at least one of:

- Packets processed per second
- CPU usage
- Processing latency
- Allocations
- Queue drops
- UDP errors
- Routing lookup time

## PostgreSQL

- Never write to PostgreSQL directly from the UDP handler.
- Keep writes asynchronous and batched.
- Use a shared, bounded connection pool.
- Do not hardcode credentials or database addresses.
- Track failed batches and dropped records.
- Avoid tight retry loops when PostgreSQL is unavailable.
- Use explicit migrations for schema changes.
- Do not run destructive production queries automatically.

## Configuration

- Document every new config key.
- Keep defaults conservative and bounded.
- Validate invalid or dangerous values at startup.
- Do not hardcode production IPs, paths, credentials, or hostnames.
- Keep secrets out of Git and logs.
- Update `.env.example` when adding environment variables.

## Docker

- Do not use `privileged: true`.
- Do not mount the Docker socket into the crawler.
- Publish the DHT listener explicitly as UDP.
- Do not expose PostgreSQL or internal metrics publicly.
- Keep container logs size-limited.
- Preserve graceful shutdown behavior.
- Avoid `latest` image tags for production deployments.

Example DHT port declaration:

```yaml
ports:
  - "6881:6881/udp"
```

## Testing

Public tests must not contact the live DHT.

Use:

- Loopback UDP sockets
- Mock peers
- Synthetic bencoded packets
- Disposable PostgreSQL containers
- Deterministic Node IDs and tokens

Run the narrowest relevant tests during development, then run the full checks before completion:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

If a command cannot be run, state why.

## Git Rules

- Do not overwrite unrelated user changes.
- Do not run `git reset --hard`, `git clean`, force-push, or history rewrites unless explicitly requested.
- Review the final diff with `git diff --check` and `git diff`.
- Keep commits focused.

## Completion Report

When finished, report:

1. What changed.
2. Which files changed.
3. Tests and checks run.
4. Any remaining risks or limitations.
5. Whether configuration, migration, or restart is required.
