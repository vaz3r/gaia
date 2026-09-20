# GAIA Infrastructure & Ecosystem Topology Reference

This reference document maps out the entire GAIA deployment topology across all 4 operational target environments.

---

## 1. Node Topology Matrix

| Target Name | Role | Host / IP | Auth Method | Hosted Containers | Primary Responsibilities |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`workspace-production`** | Homelab Core Cluster | `100.87.194.112` (Tailscale) / `localhost` | Native SSH / Docker Context `prod` | `gaia-postgres`, `gaia-dashboard`, `gaia-classifier`, `gaia-ml`, `gaia-logger`, `gaia-redis`, `gaia-pgbouncer`, `gaia-backup`, `gaia-wstunnel-client`, `gaia-wireguard-client` | Master DB (`craw`), ML classification & anomaly detection, telemetry logging, backup, admin UI |
| **`gaia-gateway`** | Offshore VPS Ingress Proxy | `192.168.10.111` | Password SSH (`sshpass` from `gaia-gateway/.env`) | `gaia-gateway-nginx`, `gaia-gateway-wstunnel`, `gaia-gateway-wireguard` | Public SSL termination, Torznab rate limiting, WebSocket tunnel termination (`wstunnel`), WireGuard gateway (`10.99.0.1`) |
| **`gaia-portal`** | Public Web Serving Node | `192.168.10.139` | Password SSH (`sshpass` from `gaia-portal/.env`) | `gaia-portal-web`, `gaia-portal-api`, `gaia-portal-meilisearch`, `gaia-portal-redis`, `gaia-portal-sync`, `gaia-portal-wstunnel-client`, `gaia-portal-wireguard-client` | React web portal, .NET 10 search API, Meilisearch full-text search, CDC background synchronizer |
| **`gaia-node`** | External DHT Crawler VPS | `100.66.211.64` (alias `gaia`) / `132.145.189.201` (`zerone`) | SSH Key (`~/.ssh/zerone`) | `gaia-crawler`, `gaia-logger` | High-throughput BitTorrent DHT crawling, BEP-5 routing, BEP-9 peer metadata handshakes |

---

## 2. Container Port & Network Specification

### `workspace-production` (Homelab Core)
- **`gaia-postgres`**: Port `5432` (PostgreSQL 16 master instance, database: `craw`, user: `crawler`).
- **`gaia-pgbouncer`**: Port `6432` (Connection pooler: transaction mode, pool size: 25-500).
- **`gaia-dashboard`**: Port `3000` (ASP.NET Core / Vite admin control center).
- **`gaia-classifier`**: Port `8080` (FastAPI / LightGBM inference worker; `scripts/entrypoint.py`).
- **`gaia-ml`**: Host network mode (Supervisor orchestrating `scoring-worker` and `anomalies-worker`).
- **`gaia-redis`**: Port `6379` (In-memory caching and lock coordinator).
- **`gaia-logger`**: Port `3001` (Analyzer API), Port `3100` (Log ingestion receiver).
- **`gaia-backup`**: Automated `pg_dump` cron (`0 2 * * *`) with Rclone Google Drive synchronization.
- **`gaia-wstunnel-client`**: Outbound TLS 1.3 WebSocket to Gateway (`wss://192.168.10.111:443/wstunnel`).
- **`gaia-wireguard-client`**: Point-to-point WireGuard interface `wg0` (`10.99.0.2` -> `10.99.0.1`).

### `gaia-gateway` (Offshore Proxy)
- **`gaia-gateway-nginx`**: Ports `80`, `443` (TLS 1.3 termination, rate-limiting, `/wstunnel` proxying).
- **`gaia-gateway-wstunnel`**: Port `8443` (Decapsulates WebSocket into UDP WireGuard packets).
- **`gaia-gateway-wireguard`**: UDP `51820` / Interface `wg0` (`10.99.0.1/24`).

### `gaia-portal` (Search Node)
- **`gaia-portal-web`**: Port `3005` (React frontend).
- **`gaia-portal-api`**: Port `5000` (.NET 10 Search & Torznab API).
- **`gaia-portal-meilisearch`**: Port `7700` (Full-text search engine index `torrents`).
- **`gaia-portal-redis`**: Port `6379` (Query cache).
- **`gaia-portal-sync`**: Continuous CDC worker streaming verified torrents from Postgres to Meilisearch.
- **`gaia-portal-wstunnel-client`**: Outbound WebSocket to Gateway.
- **`gaia-portal-wireguard-client`**: WireGuard interface `wg0` (`10.99.0.3` -> `10.99.0.1`).

### `gaia-node` (Crawler VPS)
- **`gaia-crawler`**: UDP `6881-6882` (Kademlia DHT traversal, BEP-9 metadata download).
- **`gaia-logger`**: Shipper profile tailing crawler logs and posting to central logger receiver.

---

## 3. Data Flow Architecture

```
[Global DHT Swarm] ──UDP 6881──> [gaia-crawler (gaia-node)]
                                         │
                               Batch SQL │ Insert (via WireGuard)
                                         ▼
                          [gaia-postgres (workspace-production)]
                             │             │              │
                             ▼             ▼              ▼
                     [gaia-classifier] [gaia-ml]   [gaia-portal-sync]
                      (Category ML)   (Anomalies)         │
                                                          ▼
                                              [gaia-portal-meilisearch]
                                                          ▲
                                                          │
    [Users / *arr Apps] ──HTTPS──> [gaia-gateway] ──> [gaia-portal-api]
```
