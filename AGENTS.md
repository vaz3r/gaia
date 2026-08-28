# AGENTS.md

## Purpose

This file defines the mandatory operating rules for AI coding agents working on **Gaia**, a Rust-based BitTorrent DHT research crawler.

Treat these instructions as repository policy. Apply them to every task unless a more specific `AGENTS.md` exists deeper in the directory tree. A nested file may add stricter rules for its subtree, but it must not weaken the safety, security, privacy, or production protections in this file.

The words **MUST**, **MUST NOT**, **SHOULD**, and **SHOULD NOT** are normative.

---

## 1. Project Context

Gaia is an asynchronous, high-throughput network measurement system. Its main responsibilities may include:

- Participating in the BitTorrent DHT using KRPC over UDP.
- Processing DHT messages such as `ping`, `find_node`, `get_peers`, and supported extensions.
- Maintaining routing state.
- Discovering infohashes and peer endpoints.
- Optionally verifying protocol metadata through separately controlled subsystems.
- Batching observations and metrics to PostgreSQL.
- Exposing operational health information.
- Running in Docker on a small Linux VPS.

Current production topology:

```text
OVH VPS "gaia"
├── Gaia crawler container
├── Docker Engine and Compose
├── Tailscale
└── SSH administration over Tailscale
        │
        └── Tailscale-encrypted connection
                │
                ▼
workspace-production
├── PostgreSQL
└── Dashboard and supporting services
```

Production constraints:

- Architecture: `linux/amd64` on OVH.
- Compute: 2 shared vCores.
- Memory: 4 GB RAM.
- Storage: 40 GB NVMe.
- Public network: 500 Mbps.
- DHT listener: UDP port `6881`, unless configuration explicitly says otherwise.
- PostgreSQL remains remote and is reached through Tailscale.
- The VPS is disposable; the database is not.
- No change may assume unlimited CPU, memory, connections, IOPS, packet rate, or disk capacity.

---

## 2. Instruction Precedence

When instructions conflict, follow this order:

1. Human instructions in the current task.
2. This `AGENTS.md` and any stricter nested `AGENTS.md`.
3. Repository documentation and architecture decisions.
4. Existing tests and established code behavior.
5. Tool defaults and agent assumptions.

If a requested change would violate a security, privacy, legal, provider-policy, or destructive-operation rule in this file, stop and clearly explain the conflict instead of implementing it.

Never interpret silence as permission for a destructive or production-impacting action.

---

## 3. Core Agent Behavior

### 3.1 Inspect before editing

Before changing code, the agent MUST:

1. Read this file completely.
2. Search for nested `AGENTS.md` files.
3. Read the relevant source files, tests, configuration, and deployment files.
4. Identify the current behavior and invariants.
5. Check the working tree with `git status --short`.
6. Preserve unrelated user changes.
7. Form a concise implementation plan.

Do not modify a file based only on its filename, a previous conversation, or an assumed architecture.

### 3.2 Make the smallest correct change

- Prefer focused changes over broad rewrites.
- Do not refactor unrelated code while fixing a bug.
- Do not rename public APIs, configuration keys, metrics, database fields, containers, services, or scripts unless the task requires it.
- Avoid speculative abstractions.
- Preserve backward compatibility unless a breaking change is explicitly requested.
- If a migration is required, provide the migration and rollback path in the same change.

### 3.3 Do not hide uncertainty

The agent MUST distinguish among:

- Verified facts from inspected code.
- Reasonable conclusions.
- Assumptions that still require validation.

Do not claim a command, test, benchmark, deployment, or migration succeeded unless it was actually executed and its result checked.

### 3.4 No unattended scope expansion

Do not independently add:

- New network protocols.
- New public ports.
- New external services.
- New telemetry destinations.
- New data collection fields.
- New background loops.
- New database retention.
- New protocol identities.
- New peer interaction behavior.
- New production dependencies.

Such changes require explicit human direction and a documented impact assessment.

---

## 4. Absolute Safety Boundaries

The agent MUST NOT implement, optimize, enable, or advise on functionality intended to:

- Disrupt, overload, degrade, or evade protections on third-party systems.
- Conduct denial-of-service activity or packet flooding.
- Perform unauthorized scanning, exploitation, intrusion, or vulnerability probing.
- Manipulate remote routing tables through deceptive identity multiplication.
- Present one host as a large swarm of independent nodes to gain disproportionate influence.
- Evade abuse attribution, provider enforcement, lawful process, or monitoring.
- Conceal activity using false account, billing, identity, or infrastructure information.
- Download, upload, seed, store, or distribute copyrighted payloads without authorization.
- Defeat provider limits, anti-abuse systems, rate limits, or network controls.

If the repository contains existing experimental multi-identity or aggressive network behavior, treat it as **high risk**:

- Do not enable it by default.
- Do not increase its scale, rate, reach, persistence, or stealth.
- Do not weaken its kill switch, limits, or observability.
- Do not deploy it to production.
- Safe work is limited to disabling it, documenting it, testing it in an isolated controlled environment, or reducing its risk.

Supported research functionality must remain standards-oriented, bounded, observable, and non-disruptive.

---

## 5. Repository Discovery

At the beginning of a task, inspect the repository using focused commands such as:

```bash
git status --short
find .. -name AGENTS.md -print
find . -maxdepth 3 -type f | sort | sed -n '1,240p'
rg -n "TODO|FIXME|HACK|unsafe|unwrap\(|expect\(" apps deploy .
```

Then inspect only the files relevant to the task.

Likely important areas include:

```text
apps/crawler/
apps/crawler/src/
apps/crawler/src/dht/
apps/crawler/src/verify/
apps/crawler/config/
deploy/compose/
deploy/scripts/
migrations/
Cargo.toml
Cargo.lock
.env.example
```

Do not assume all paths exist. Verify them first.

---

## 6. Rust Engineering Standards

### 6.1 General quality

All Rust changes MUST:

- Compile on stable Rust unless the repository explicitly pins another toolchain.
- Preserve `linux/amd64` compatibility.
- Pass formatting and relevant lint checks.
- Avoid unnecessary allocation in packet-processing paths.
- Avoid blocking operations on Tokio worker threads.
- Avoid unbounded queues, collections, tasks, retries, logs, or concurrency.
- Avoid panics on untrusted network input.
- Preserve cancellation and graceful shutdown behavior.

### 6.2 Error handling

- Do not add `unwrap()` or `expect()` on data influenced by the network, database, filesystem, configuration, or environment.
- Prefer typed errors and contextual messages.
- Classify transient and permanent failures when retry behavior differs.
- Never log secrets in error context.
- A malformed remote packet must be rejected without crashing a worker or process.
- A database outage must not create a tight retry loop.

### 6.3 Unsafe code

- Do not add new `unsafe` code unless explicitly required.
- Any new `unsafe` block requires a nearby `// SAFETY:` explanation and focused tests.
- Prefer safe standard-library or well-maintained crate functionality.

### 6.4 Dependencies

Before adding a crate:

1. Confirm the standard library or an existing dependency cannot solve the problem.
2. Explain why the dependency is necessary.
3. Prefer actively maintained, narrowly scoped crates.
4. Avoid default features that are not needed.
5. Check license compatibility where tooling permits.
6. Run applicable dependency and security checks if available.

Do not update unrelated dependencies or regenerate `Cargo.lock` unnecessarily.

### 6.5 Formatting and linting

Use repository-supported commands. Typical checks are:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

If the workspace is too large or a command is unavailable, run the narrowest valid equivalent and state what was not run.

---

## 7. Async and Concurrency Invariants

Gaia is latency-sensitive and highly concurrent. The following rules are mandatory:

- The UDP receive path MUST remain non-blocking.
- Do not perform database, filesystem, DNS, HTTP, or other blocking I/O in packet handlers.
- Use bounded channels with explicit overflow behavior.
- Every task loop must have cancellation or shutdown behavior.
- Every retry loop must have bounded exponential backoff and jitter where appropriate.
- Do not hold mutex guards across `.await` unless the lock type and design explicitly require it.
- Do not spawn an unbounded task per packet, peer, infohash, or database row.
- Avoid global locks in packet-processing paths.
- Preserve fairness so metadata verification cannot starve DHT processing.
- Preserve the bounded receive-drain behavior unless benchmarks and tests justify a change.

When adding concurrency, document:

- Maximum task count.
- Queue capacity.
- Overflow policy.
- Timeout.
- Cancellation behavior.
- Memory upper bound.

---

## 8. Network and Protocol Rules

### 8.1 Untrusted input

Every network packet is untrusted.

- Validate packet size before parsing.
- Bound bencode nesting, string lengths, list lengths, and allocation.
- Reject malformed transaction IDs and invalid field lengths.
- Validate compact node and peer encodings.
- Avoid amplification behavior.
- Responses should not be disproportionately larger than requests.
- Never reflect traffic to an address not validated by the protocol flow.

### 8.2 Rate controls

Network behavior MUST remain bounded by configuration.

- Keep a global rate limit.
- Keep per-destination limits where supported.
- Back off from unresponsive destinations.
- Avoid repeatedly querying the same endpoint in a short interval.
- Expose dropped, throttled, timed-out, and rejected operations as metrics.
- Do not silently raise default rates.

Any change affecting packet rate, query concurrency, response percentage, identity count, or lookup frequency requires:

1. Tests.
2. A resource-impact note.
3. Updated configuration documentation.
4. A safe default.
5. A rollback or kill switch.

### 8.3 Public binding

- The DHT service may bind only to the configured UDP port.
- Do not expose PostgreSQL, Docker API, metrics, debugging endpoints, or dashboards publicly.
- Docker port declarations MUST include `/udp` for DHT.
- Do not add public TCP listeners without explicit approval.
- Do not change the expected external IP behavior silently.

### 8.4 DHT identity

- Preserve stable identity behavior unless explicitly changed.
- Do not introduce identity multiplication to manipulate third-party routing state.
- Do not send protocol claims that misrepresent the crawler’s behavior or capabilities.
- Changes involving `announce_peer`, implied ports, tokens, node IDs, or advertised endpoints require protocol-focused tests and explicit human review.

### 8.5 Metadata and peer interaction

Treat DHT discovery, peer discovery, metadata retrieval, and payload transfer as separate subsystems.

- Do not merge their controls or metrics.
- Payload piece transfer must remain disabled unless the repository owner explicitly authorizes a lawful test environment.
- Do not add seeding or payload storage behavior.
- Track peer connections, metadata transfers, and failures separately from DHT traffic.
- Never describe the system as “DHT-only” if peer metadata retrieval is enabled.

---

## 9. Cryptography and Tokens

- Do not create custom cryptographic primitives.
- Preserve constant-time comparison for authentication tokens where applicable.
- Do not log tokens, secrets, keys, authentication headers, or raw credentials.
- Secret rotation must preserve a bounded overlap window where protocol compatibility requires it.
- Changes to HMAC/token generation or verification require known-answer tests, expiry tests, previous-secret overlap tests, and malformed-input tests.
- Measure hot-path cost before and after cryptographic changes.

---

## 10. Routing Table Rules

Routing state is performance-critical.

- Preserve Kademlia distance semantics.
- Deduplicate correctly by the intended identity and endpoint rules.
- Do not turn bounded buckets into unbounded collections.
- Avoid full collection sorts when only top-k results are needed, unless benchmarks show the simple implementation is adequate.
- Do not duplicate every discovered node across many internal routing tables.
- Preserve liveness, questionable-node, bad-node, and replacement behavior.
- Any routing-table redesign requires property tests or equivalent invariant tests.

At minimum, test:

- Correct bucket selection.
- Correct XOR ordering.
- Capacity limits.
- Deduplication.
- Liveness transitions.
- Closest-node output size.
- Stability under duplicate and malformed nodes.
- Memory growth under adversarial input.

---

## 11. PostgreSQL and Persistence

### 11.1 Production topology

PostgreSQL is remote and accessed over Tailscale.

- Do not expose port `5432` publicly.
- Do not move PostgreSQL onto the Gaia VPS without explicit instruction.
- Do not embed credentials in scripts, source code, logs, Compose files committed to Git, or command examples.
- Use environment variables, protected env files, a secrets manager, Docker secrets, or `.pgpass` as appropriate.

### 11.2 Connection pools

- Do not set the crawler pool near PostgreSQL’s complete `max_connections` value.
- Start with a conservative shared pool, currently expected around 12 connections unless measured requirements justify otherwise.
- Confirm whether settings apply per process, per subsystem, or per pool.
- Expose pool usage, wait time, errors, and acquisition latency.

### 11.3 Batching and failure behavior

The UDP hot path must never wait synchronously for PostgreSQL.

- Preserve asynchronous batching.
- Use bounded buffers.
- Avoid tight retries during outages.
- Track failed flushes and dropped records.
- If a disk spool is introduced, it must be bounded by size and age and have an explicit overflow policy.
- A spool must not be introduced silently because it changes durability, privacy, and disk-use behavior.

### 11.4 Schema changes

For every database migration:

- Make it explicit and versioned.
- Provide forward and rollback reasoning.
- Avoid long blocking table rewrites where possible.
- Consider indexes for new query patterns.
- Avoid destructive data changes without a backup and explicit approval.
- Never run production migrations automatically from an unreviewed agent session.

### 11.5 Health queries

Do not use unbounded exact `COUNT(*)` scans as routine health checks on large tables.

Prefer lightweight checks such as:

```sql
SELECT 1;
```

For freshness, use an indexed timestamp or a purpose-built health table/metric.

---

## 12. Privacy and Data Minimization

Gaia may process IP addresses, ports, timestamps, node IDs, infohashes, and metadata. Treat these records as sensitive operational data.

The agent MUST:

- Collect only fields required by the stated purpose.
- Avoid adding new identifiers without explicit approval.
- Avoid public exposure of raw peer-level data.
- Avoid labels that claim an observed IP belongs to a downloader, infringer, or specific person.
- Preserve configurable retention and deletion behavior.
- Protect raw logs and exports.
- Avoid full-packet logging by default.
- Never include real peer data in tests, fixtures, examples, screenshots, or documentation.
- Use synthetic documentation examples.

Changes affecting collection scope, retention, correlation, publication, export, or cross-border storage require explicit human review.

---

## 13. Configuration Rules

### 13.1 Source of truth

- Defaults belong in typed configuration code or committed example configuration.
- Production secrets belong outside Git.
- `.env.example` must contain placeholders only.
- Every new environment variable must be documented.
- Configuration parsing must fail clearly on invalid or unsafe values.

### 13.2 Safe defaults

Defaults must favor safety and bounded resource use.

- New experimental functionality defaults to disabled.
- New public listeners default to disabled.
- New data retention defaults to the minimum practical duration.
- New concurrency and queue limits must be finite.
- Dangerous combinations must fail validation rather than run silently.

### 13.3 Naming

Preserve existing naming conventions such as `CRAW_...` unless a migration is explicitly requested.

For a renamed variable:

- Support the old name temporarily when feasible.
- Emit a deprecation warning without exposing values.
- Update `.env.example`, config docs, deployment scripts, and tests.

### 13.4 Production-specific values

Do not hardcode:

- Public IP addresses.
- Tailscale IP addresses.
- Database passwords.
- Hostnames that vary by environment.
- Absolute home paths when an environment variable or Compose variable is appropriate.

---

## 14. Docker and Compose Rules

### 14.1 Image compatibility

Production runs on `linux/amd64`.

- Verify images support `linux/amd64`.
- Preserve multi-platform builds if both ARM64 and AMD64 are supported.
- Do not rely on emulation in production.
- Pin base images to a controlled tag or digest according to repository convention.

### 14.2 Container security

Where compatible with the application:

- Run as a non-root user.
- Drop unnecessary Linux capabilities.
- Do not use `privileged: true`.
- Do not mount `/var/run/docker.sock`.
- Use a read-only root filesystem where practical.
- Mount only required writable paths.
- Add memory, PID, and reasonable CPU protections where appropriate.
- Configure bounded Docker log rotation.
- Preserve graceful shutdown with a sufficient stop period.

### 14.3 Ports

The Compose file should expose only required ports. Example:

```yaml
ports:
  - "6881:6881/udp"
```

Do not expose:

- `5432` PostgreSQL.
- Docker API ports `2375` or `2376`.
- Debuggers.
- Profilers.
- Internal metrics.
- Administrative dashboards.

### 14.4 Deployment behavior

- Avoid `latest` tags in production.
- Record the deployed image digest.
- Build once and deploy the same artifact.
- Do not rebuild production from an uncommitted working tree.
- Do not use destructive Compose flags without explicit approval.

---

## 15. Deployment Safety

### 15.1 Never deploy automatically

The agent MUST NOT deploy to production unless explicitly instructed in the current task.

Without explicit deployment authorization, the agent may:

- Prepare code.
- Prepare commands.
- Validate configuration locally.
- Build images.
- Run tests.
- Produce a deployment checklist.

It must not:

- SSH into production.
- Restart production services.
- Apply production migrations.
- Change OVH firewall rules.
- Change Tailscale ACLs.
- Rotate production credentials.
- Delete production data.

### 15.2 Pre-deployment gate

Before an authorized deployment, verify:

```bash
git status --short
git rev-parse HEAD
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
docker compose config
```

Also verify:

- The intended branch and commit.
- Required environment variables without printing secrets.
- Database connectivity through Tailscale.
- Disk and memory headroom.
- Published ports.
- Rollback procedure.
- Old and new container/image identifiers.

### 15.3 Cutover

- Avoid prolonged simultaneous operation of old and new crawler instances.
- Do not run the same protocol identity from multiple public endpoints unless explicitly designed and reviewed.
- Stop the old instance gracefully before full cutover.
- Record UTC cutover time.
- Keep rollback deployment files available.

### 15.4 Rollback

Every production change must have a practical rollback.

Rollback instructions must identify:

- Previous commit or image digest.
- Configuration compatibility.
- Database migration implications.
- Graceful stop command.
- Restart command.
- Verification steps.

Do not claim “no data loss.” State precisely which committed data is preserved and which in-memory data may be lost.

---

## 16. Destructive Command Policy

The agent MUST NOT run destructive commands without explicit, current human authorization and a clear explanation of impact.

Examples include:

```text
rm -rf
find ... -delete
git reset --hard
git clean -fd or -fdx
git checkout -- <path>
git restore <path> when user changes may exist
docker system prune
docker volume prune
docker compose down -v
DROP / TRUNCATE / DELETE without a bounded predicate
filesystem formatting or partitioning
credential or key deletion
firewall deny-all changes on a remote host
```

Never overwrite or discard uncommitted user work.

If cleanup is required, list the exact targets first and use the narrowest possible command.

---

## 17. Secrets and Sensitive Information

Never print, commit, store in documentation, or echo:

- Database passwords.
- OVH bearer tokens.
- API keys.
- Tailscale auth keys.
- Private SSH keys.
- WireGuard private keys.
- Session cookies.
- Full DSNs containing credentials.
- Real payment or identity data.

If a secret is observed in chat, logs, source, or command history:

1. Do not repeat it.
2. Redact it in all output.
3. Warn that it should be rotated or revoked.
4. Remove it from tracked files without exposing it again.
5. Do not claim deletion from Git history unless history was explicitly and safely rewritten.

Use placeholders:

```text
<DB_PASSWORD>
<OVH_PUBLIC_IP>
<DB_TAILSCALE_IP>
<TAILSCALE_AUTH_KEY>
```

---

## 18. Testing Requirements

### 18.1 Test the behavior changed

Every bug fix should include a regression test when feasible.

Every feature should cover:

- Happy path.
- Invalid input.
- Boundary conditions.
- Timeout or cancellation.
- Resource-bound behavior.
- Relevant concurrency behavior.

### 18.2 Network tests

Tests MUST NOT depend on the public DHT or contact arbitrary Internet hosts.

Use:

- Local UDP sockets.
- Loopback addresses.
- Deterministic fixtures.
- Mock peers.
- Controlled containers.
- Recorded synthetic messages with no real personal data.

Do not run high-rate or load tests against third-party infrastructure.

### 18.3 Database tests

- Use an isolated test database.
- Never point tests at production.
- Use unique schemas or disposable containers.
- Clean up only test-created data.
- Confirm environment safeguards before destructive test setup.

### 18.4 Performance changes

For hot-path changes, benchmark before and after where possible.

Measure at least one relevant metric:

- Datagrams processed per second.
- CPU time per datagram.
- Allocation count or bytes.
- Routing lookup latency.
- HMAC/token latency.
- Queue depth and drop rate.
- Database flush duration.

Do not claim an optimization without measurements.

---

## 19. Observability Requirements

Any operation that can fail silently should have a counter or structured diagnostic path.

Important metrics include:

- UDP datagrams received and sent.
- Parse failures.
- Invalid protocol messages.
- `try_send` failures.
- Socket send failures.
- Rate-limited operations.
- DHT queries and responses by type.
- Timeouts.
- Queue depth or high-water marks.
- Metadata connection attempts and outcomes.
- Database batch size, duration, failures, and dropped records.
- Last successful database flush.
- Graceful shutdown duration.

Metrics MUST NOT include secrets or unnecessary raw peer identifiers.

Structured logs should:

- Use UTC timestamps.
- Include a stable event name.
- Include instance/build identifiers where useful.
- Avoid per-packet info-level logging.
- Use sampling or aggregation for high-rate events.
- Respect configured file and total-size limits.

---

## 20. Resource Budgets

Design for the actual VPS, not a developer workstation.

Target constraints:

- 2 shared vCores.
- 4 GB RAM.
- 40 GB disk.
- Remote PostgreSQL.
- High rates of small UDP packets.

Required design properties:

- Bounded memory growth.
- Bounded disk growth.
- No unbounded task creation.
- No per-packet synchronous logging.
- No database write per packet.
- No full-table health scans.
- No assumption that advertised Mbps guarantees packet-per-second capacity.

When changing limits, estimate worst-case memory usage. If a queue can contain `N` items of approximate size `S`, document the approximate upper bound and include overhead.

---

## 21. Production Monitoring and Acceptance

Do not judge capacity using bandwidth alone.

Relevant system observations include:

```bash
mpstat -P ALL 1
vmstat 1
netstat -su
nstat -az
ip -s link
cat /proc/net/softnet_stat
docker stats gaia-crawler --no-stream
```

On a two-vCore host, Docker CPU values are approximately:

```text
100% = one fully utilized core
200% = both cores fully utilized
```

Production acceptance should consider:

- Whole-server and per-core CPU.
- `%soft` and `%steal`.
- UDP receive/send errors.
- Interface and softnet drops.
- Queue drops.
- Database freshness.
- Crawler yield.
- Provider notices or mitigation events.

A short spike alone is not a failure. Sustained saturation combined with drops, growing queues, or degraded output is actionable.

---

## 22. Documentation Standards

Update documentation when behavior changes.

Documentation MUST state:

- What changed.
- Why it changed.
- Default behavior.
- Configuration and limits.
- Security and privacy implications.
- Metrics or logs added.
- Deployment steps.
- Rollback steps.

Use neutral, precise language.

Do not use terms such as:

- “weaponize”
- “stealth”
- “evade”
- “hide from provider”
- “force remote nodes”
- “blanket the network”

Describe legitimate technical behavior accurately without sensational or evasive wording.

Do not claim:

- “No data loss” when in-memory records may be lost.
- “DHT-only” when peer metadata connections are enabled.
- “Anonymous” when a hosting provider can identify an account.
- “Unlimited” when implementation or provider fair-use limits may apply.

---

## 23. Git and Change Hygiene

Before editing:

```bash
git status --short
```

After editing:

```bash
git diff --check
git diff --stat
git diff -- <relevant-paths>
```

Rules:

- Do not amend, rebase, force-push, or rewrite history unless explicitly instructed.
- Do not commit generated artifacts unless repository policy requires them.
- Do not include secrets or environment-specific values.
- Keep commits focused and explain intent.
- Preserve line endings and file formatting conventions.
- Do not modify unrelated files because a formatter or generator touched them unless required.

Suggested commit style:

```text
fix(crawler): prevent retry loop on database outage
feat(metrics): expose UDP receive buffer errors
chore(deploy): harden crawler container settings
```

---

## 24. Required Final Report

At the end of every task, report:

1. **Summary**: what changed and why.
2. **Files changed**: concise list.
3. **Validation**: exact commands run and outcomes.
4. **Risks or limitations**: remaining concerns.
5. **Deployment impact**: whether configuration, migration, restart, firewall, or downtime is required.
6. **Rollback**: how to reverse the change when relevant.

Never state that production was changed unless it actually was.

Example:

```text
Summary
- Added bounded exponential backoff to database batch flushes.
- Added metrics for flush failures and dropped records.

Validation
- cargo fmt --all -- --check: passed
- cargo test -p crawler storage::tests: passed
- cargo clippy -p crawler --all-targets -- -D warnings: passed

Deployment impact
- Crawler restart required.
- No schema migration.
- No new ports or secrets.

Rollback
- Redeploy the previous image digest; configuration remains compatible.
```

---

## 25. Stop Conditions

The agent must stop and ask for human direction when:

- Requirements are materially ambiguous and different interpretations change network behavior or data handling.
- A change may expose a new public service.
- A schema migration could destroy or rewrite significant data.
- Production credentials are missing.
- Existing uncommitted changes conflict with the task.
- Tests reveal unrelated failures that block safe validation.
- The requested behavior may violate provider terms, law, third-party rights, or the safety boundaries above.
- A task requests identity concealment, abuse evasion, disruptive crawling, or routing manipulation.
- An authorized production action lacks a tested rollback path.

Do not bypass the stop condition with a guess.

---

## 26. Definition of Done

A task is complete only when:

- The requested behavior is implemented with minimal scope.
- Security and privacy boundaries remain intact.
- Resource use remains bounded.
- Relevant tests pass.
- Formatting and linting pass or failures are documented.
- Configuration and documentation are updated.
- No secrets are exposed.
- No unrelated user work is overwritten.
- Deployment and rollback requirements are stated.
- The final report accurately reflects what was and was not executed.

---

## 27. Quick Agent Checklist

```text
[ ] Read all applicable AGENTS.md files
[ ] Check git status and preserve user changes
[ ] Inspect relevant code, tests, config, and deployment files
[ ] Confirm task scope and invariants
[ ] Avoid destructive or production actions without explicit authorization
[ ] Keep queues, tasks, retries, logs, disk, and memory bounded
[ ] Keep packet handlers non-blocking
[ ] Preserve protocol correctness and conservative rates
[ ] Do not expose new ports or secrets
[ ] Do not manipulate third-party routing through synthetic identity swarms
[ ] Add or update tests
[ ] Run formatting, checks, lints, and tests
[ ] Review final diff
[ ] Document deployment impact and rollback
[ ] Report exact validation results
```

