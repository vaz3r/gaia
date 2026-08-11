## Purpose

Deploys the crawler in Docker behind a Gluetun WireGuard client so all traffic egresses from a public IP (the Oracle Cloud instance), making the DHT node reachable and raising the metadata verify rate.

## ADDED Requirements

### Requirement: Containerized crawler
The crawler SHALL run in a Docker container built from a multi-stage Dockerfile that compiles the release binary and runs it against a mounted data volume.

#### Scenario: Image builds and runs
- **WHEN** the Docker image is built
- **THEN** the binary runs with `--db /data/crawler.sqlite --state-dir /data/state --port 6881 --instances 4`

#### Scenario: Data persists across restarts
- **WHEN** the container restarts
- **THEN** the SQLite database and routing state under `/data` are preserved

### Requirement: Traffic through Gluetun WireGuard
The crawler container SHALL share the Gluetun container's network namespace so all UDP and TCP traffic egresses through the WireGuard tunnel to the Oracle public IP.

#### Scenario: Egress is the public IP
- **WHEN** the stack is up
- **THEN** a public-IP lookup from the crawler container reports `132.145.189.201`

#### Scenario: Crawler depends on a healthy tunnel
- **WHEN** Gluetun is not yet connected
- **THEN** the crawler does not start until the tunnel is healthy

### Requirement: DHT ports open inbound on the tunnel
Gluetun's firewall SHALL allow inbound UDP on the crawler's DHT ports so the node is reachable through the tunnel.

#### Scenario: Inbound DHT accepted
- **WHEN** another DHT node sends a query to our public IP on a configured DHT port
- **THEN** Gluetun forwards it to the crawler (ports 6881-6884 configured via `FIREWALL_VPN_INPUT_PORTS`)

### Requirement: Secrets out of the repository
WireGuard credentials SHALL be provided via a gitignored `.env` file referenced by Compose, and SHALL NOT be committed.

#### Scenario: Keys are not committed
- **WHEN** the repository is inspected
- **THEN** `.env` and any WireGuard key material are absent from version control
