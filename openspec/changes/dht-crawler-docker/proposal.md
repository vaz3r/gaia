## Why

The crawler is NAT-bound: even after discovery scaled ~3x (4 instances, routing grower) and fetches became more thorough, the verify rate is ~0.5% because most DHT peers cannot reach us / we cannot reach them from a NAT'd egress. Sustained throughput plateaued at ~80-100 torrents/hr.

The user operates a WireGuard server on an Oracle Cloud instance (public IP `132.145.189.201`, endpoint port **443**, tunnel DNS `10.8.1.3`, client `10.8.0.9/24`). Routing the crawler's traffic through that tunnel gives it a **public egress IP**, so peers can reach it — the same reason bitmagnet performs well on a real server. This change moves the crawler into Docker and routes all its traffic through a Gluetun WireGuard client container.

## What Changes

- **Docker containerization**: a multi-stage `Dockerfile` builds the release binary; the crawler runs in a slim runtime image.
- **Gluetun WireGuard client**: a `docker-compose.yml` stack runs `qmcgaw/gluetun` as the WireGuard client, and the crawler container uses `network_mode: "service:gluetun"` so all its UDP (DHT) and TCP (metadata) traffic egresses via the Oracle public IP.
- **Inbound DHT ports opened**: `FIREWALL_VPN_INPUT_PORTS=6881,6882,6883,6884` so Gluetun's tunnel firewall admits inbound DHT queries.
- **Secrets management**: WireGuard keys live in a gitignored `dht-crawler/.env` referenced by compose.
- **Data migration**: existing `crawler.sqlite` + `state/` are moved into a mounted `data/` dir so history and warm routing tables carry over.
- **pm2 retired for the crawler**: the Docker stack replaces the pm2 deployment. `ecosystem.config.cjs`/`run.sh` were removed entirely in the 2026-08-14 cleanup — Docker is the only deployment path.

## Capabilities

### New Capabilities

- `docker-deploy`: containerized crawler behind a Gluetun WireGuard client, with the DHT node reachable on a public egress IP.

### Modified Capabilities

- `architecture` (previous change): deployment moves from pm2/host process to docker compose with a shared network namespace.
- `discovery` (previous change): operates unchanged, but now from a public IP so routing tables and peer reachability improve.

## Impact

- **Code**: new `dht-crawler/Dockerfile`, `dht-crawler/docker-compose.yml`, `dht-crawler/.env` (gitignored), `.gitignore` entries for `data/`.
- **Dependencies**: Docker + Docker Compose on the host (present); Gluetun image pulled; no Rust dependency changes.
- **Operations**: crawler egresses from `132.145.189.201`; 8 UDP ports bound inside the Gluetun network namespace; pm2 removed.
- **Performance (expected)**: verify rate should rise several-fold from NAT-bound (~0.5%), pushing torrents/hr well above the ~100 ceiling.
