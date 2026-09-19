# GAIA — Master System Architecture & Ecosystem Reference

> **The Definitive Source of Truth for the GAIA Ecosystem**  
> *Last Updated: September 2026*

---

## Table of Contents
1. [Executive Summary & System Vision](#1-executive-summary--system-vision)
2. [Global Architecture & Network Topology](#2-global-architecture--network-topology)
3. [Applications Catalog (All 10 Apps)](#3-applications-catalog-all-10-apps)
   - [3.1 `apps/crawler` — Rust DHT Ingestion Engine](#31-appscrawler--rust-dht-ingestion-engine)
   - [3.2 `apps/api` — .NET 10 High-Velocity Torznab & Search API](#32-appsapi--net-10-high-velocity-torznab--search-api)
   - [3.3 `apps/portal` — Modern Public Search Web Portal](#33-appsportal--modern-public-search-web-portal)
   - [3.4 `apps/dashboard` — Administrative Control Center & Telemetry](#34-appsdashboard--administrative-control-center--telemetry)
   - [3.5 `apps/classifier` — ML Category Inference Worker](#35-appsclassifier--ml-category-inference-worker)
   - [3.6 `apps/ml` — Anomaly Detection & Multi-Model Intelligence](#36-appsml--anomaly-detection--multi-model-intelligence)
   - [3.7 `apps/sync` — Meilisearch & Redis Cache Synchronizer](#37-appssync--meilisearch--redis-cache-synchronizer)
   - [3.8 `apps/tunnel` — WebSocket WireGuard Encapsulator (`wstunnel`)](#38-appstunnel--websocket-wireguard-encapsulator-wstunnel)
   - [3.9 `apps/backup` — Automated Disaster Recovery Worker](#39-appsbackup--automated-disaster-recovery-worker)
   - [3.10 `apps/logger` — Distributed Log Shipper & Janitor](#310-appslogger--distributed-log-shipper--janitor)
4. [Deployment Targets & Infrastructure Roles](#4-deployment-targets--infrastructure-roles)
   - [4.1 `workspace-production` (Homelab Core)](#41-workspace-production-homelab-core)
   - [4.2 `gaia-gateway` (Offshore VPS Proxy)](#42-gaia-gateway-offshore-vps-proxy)
   - [4.3 `gaia-portal` (Public Web Serving Node)](#43-gaia-portal-public-web-serving-node)
   - [4.4 `gaia-node` (External VPS Crawler Node)](#44-gaia-node-external-vps-crawler-node)
5. [Air-Tight Homelab OpSec & ISP-Blindness Model](#5-air-tight-homelab-opsec--isp-blindness-model)
6. [Data Pipeline & Storage Architecture](#6-data-pipeline--storage-architecture)
7. [Operations & Server Migration Runbook](#7-operations--server-migration-runbook)

---

## 1. Executive Summary & System Vision

**GAIA** is an enterprise-grade, decentralized BitTorrent indexing, machine-learning classification, search, and telemetry ecosystem.

### Core Architectural Tenets
1. **Decoupled Ingestion & Serving**: High-volume, noisy BitTorrent DHT crawling (millions of UDP packets/sec) is isolated on external offshore VPS nodes (`gaia-node`). The homelab server never runs DHT crawl workers.
2. **Air-Tight Homelab OpSec**: The homelab storage and processing cluster has zero open inbound ports on the home router, zero public DNS records pointing to the home IP, and encrypts all outbound traffic through TLS 1.3 WebSockets (`wstunnel`) on port 443.
3. **Sub-Millisecond Search**: Torznab XML/JSON feeds and public web searches are served via Meilisearch and high-performance in-memory caching, capable of handling automated RSS poll traffic from `*arr` applications (Sonarr, Radarr, Prowlarr).
4. **Automated ML Classification**: Ingested payloads are classified into standard categories (Movies, Television, Anime, Software, Games, Adult) using trained LightGBM models, while unsupervised autoencoders and Isolation Forests detect malicious swarms, fake torrents, and zero-day anomalies.

---

## 2. Global Architecture & Network Topology

```mermaid
flowchart TD
    subgraph Internet_Public [Public Internet / Users / BitTorrent Swarms]
        Users["Users & Media Automation<br/>(Sonarr / Radarr / Web Browsers)"]
        DHT["Global BitTorrent Swarm<br/>(Mainline DHT / BEP-5 / BEP-9)"]
    end

    subgraph Offshore_VPS_1 [gaia-node (Offshore VPS - Crawler Node)]
        CRAW["apps/crawler<br/>(Rust DHT Ingestion)"]
        SHIPPER["apps/logger<br/>(Log Shipper)"]
        DHT <-->|UDP 6881 / DHT Packets| CRAW
    end

    subgraph Offshore_VPS_2 [gaia-gateway (Offshore Reverse Proxy - Public IP)]
        NGINX["Nginx (SSL Termination & Rate Limiting)<br/>TLS 1.3 / Port 443"]
        GW_TUNNEL["wstunnel-server<br/>(Port 8443)"]
        GW_WG["wireguard<br/>(10.99.0.1/24)"]
        
        Users -->|HTTPS 443| NGINX
        NGINX -->|/wstunnel WebSocket| GW_TUNNEL
        GW_TUNNEL <--> GW_WG
    end

    subgraph Homelab_Cluster [Homelab Infrastructure (Zero Inbound Ports / Hidden IP)]
        direction TB

        subgraph Portal_Node [gaia-portal (192.168.10.139)]
            P_TUNNEL["wstunnel-client<br/>(Outbound wss:// to Gateway)"]
            P_WG["wireguard-client<br/>(10.99.0.3/24)"]
            PORTAL_WEB["apps/portal (React UI)<br/>Port 3005"]
            PORTAL_API["apps/api (.NET 10 Minimal API)<br/>Port 5000"]
            MEILI["Meilisearch (Search Engine)<br/>Port 7700"]
            REDIS["Redis 7 (Cache)<br/>Port 6379"]
            SYNC["apps/sync (CDC Worker)"]

            P_TUNNEL <-->|TLS 1.3 WebSocket| GW_TUNNEL
            P_TUNNEL <--> P_WG
            NGINX -->|Forward over Tunnel 10.99.0.3:3005| PORTAL_WEB
            PORTAL_WEB --> PORTAL_API
            PORTAL_API --> MEILI
            PORTAL_API --> REDIS
            SYNC --> MEILI
            SYNC --> REDIS
        end

        subgraph Core_Node [workspace-production (100.87.194.112 / 192.168.10.10)]
            PG[("PostgreSQL 16 (Master DB)<br/>Port 5432 / 6432")]
            DASH["apps/dashboard (Admin Web)<br/>Port 3000"]
            ML_SVC["apps/classifier & apps/ml<br/>(Inference Workers)"]
            BACKUP["apps/backup<br/>(Rclone to Cloud)"]
            LOGGER["apps/logger<br/>(Log Receiver / Janitor)"]

            CRAW -->|Batch Ingest via WireGuard/Tailscale| PG
            SYNC -->|CDC Poll| PG
            PORTAL_API -->|Read-through Fallback| PG
            DASH --> PG
            ML_SVC --> PG
            BACKUP --> PG
            SHIPPER -->|HTTP Log Stream| LOGGER
        end

        subgraph OpSec_Shield [Homelab Kernel Defense]
            DOT["systemd-resolved DoT<br/>(Cloudflare/Quad9 Port 853)"]
            KS["Kernel Egress Killswitch<br/>(iptables drop 80/443 except Gateway)"]
        end
    end
```

---

## 3. Applications Catalog (All 10 Apps)

### 3.1 `apps/crawler` — Rust DHT Ingestion Engine
* **Language & Runtime**: Rust (2021 edition), Tokio asynchronous runtime.
* **Responsibilities**:
  - Traverses the global BitTorrent Mainline DHT via Kademlia routing tables (BEP-5).
  - Listens to `get_peers` and `announce_peer` queries to detect newly announced infohashes.
  - Connects to candidate seeders over TCP to retrieve raw torrent metadata dictionaries via the Extension Protocol (BEP-10) and Metadata Transfer Protocol (BEP-9 / `ut_metadata`).
  - Utilizes memory-efficient Bloom filters and LRU caches to prevent redundant DHT queries.
  - Batches discovered torrents and active peer IP:ports directly into PostgreSQL.
* **Key Configuration**:
  - `CRAW_EXTERNAL_IP`: Public IP of the crawler node.
  - `CRAW_PROFILE`: Execution profile (`production` or `development`).
  - `DATABASE_URL`: Connection string to PostgreSQL.

---

### 3.2 `apps/api` — .NET 10 High-Velocity Torznab & Search API
* **Language & Runtime**: C# (.NET 10 Minimal API), Kestrel web server.
* **Responsibilities**:
  - **Torznab Provider**: Serves Torznab-compliant XML and JSON feeds (`/api/torznab`) with zero-allocation streaming XML serialization for media automation apps (Sonarr, Radarr, Prowlarr).
  - **Live .Torrent File Builder**: Generates verified, valid `.torrent` files on the fly via `WireMetadataFetcher.BuildTorrentBuffer`. Extracts raw BEP-9 bencoded info dictionaries, dynamically computes string lengths, ensures canonical dictionary key ordering (BEP-3), and builds multi-tier tracker lists (BEP-12).
  - **Real-Time Telemetry SSE**: Exposes Server-Sent Events endpoint (`/api/live/stream`) streaming real-time swarm throughput, ingestion rates, and system gauges to the dashboard.
  - **Batch Lookups**: High-speed batch infohash resolution with Redis caching.

---

### 3.3 `apps/portal` — Modern Public Search Web Portal
* **Language & Framework**: React 18, Vite, Tailwind CSS, Lucide Icons.
* **Responsibilities**:
  - Sleek, dark-mode public search engine for discovering torrents.
  - Instant sub-second search results powered by Meilisearch.
  - Rich torrent detail modal: file list breakdown, exact byte sizes, health scores, category badges, one-click magnet copy, and direct `.torrent` download.
  - Reverse-proxied through Nginx on port 3005.

---

### 3.4 `apps/dashboard` — Administrative Control Center & Telemetry
* **Language & Framework**: React 18, Vite, Chart.js / Recharts, Tailwind CSS.
* **Responsibilities**:
  - Mission-control dashboard for system administrators.
  - Live charts displaying crawler ingestion rates (torrents/sec, peers/sec, wire fetch success rate).
  - Health gauges for all containers, database connection pools, memory usage, and ML workers.
  - Audit logging of top search queries and user agent statistics.

---

### 3.5 `apps/classifier` — ML Category Inference Worker
* **Language & Runtime**: Python 3.11, LightGBM, Scikit-learn, FastAPI.
* **Responsibilities**:
  - Continuously processes unclassified torrents from PostgreSQL (`WHERE category IS NULL`).
  - Cleans and tokenizes release names using regular expressions (extracting resolution, release group, codec, audio tags).
  - Executes inference using a trained LightGBM multiclass model to assign categories (`Movies`, `Television`, `Anime`, `Games`, `Software`, `Adult`, `Music`, `Ebooks`).
  - Computes a classification confidence score (0.00–1.00) and updates PostgreSQL.

---

### 3.6 `apps/ml` — Anomaly Detection & Multi-Model Intelligence
* **Language & Runtime**: Python 3.11, PyTorch / Scikit-learn / Joblib.
* **Responsibilities**:
  - Evaluates swarm behavior, file count anomalies, and fake torrent signatures.
  - Employs **Isolation Forests** and **Autoencoders** to flag zero-day spam releases, password-protected fake archives, and malicious executables masquerading as media files.
  - Updates the `risk_tier` (`SAFE`, `SUSPICIOUS`, `MALICIOUS`) and `policy_action` on the torrent record.

---

### 3.7 `apps/sync` — Meilisearch & Redis Cache Synchronizer
* **Language & Runtime**: Node.js 20, TypeScript / Docker.
* **Responsibilities**:
  - Implements a Change-Data-Capture (CDC) worker bridging PostgreSQL to Meilisearch.
  - Continuously polls for newly verified or updated torrents (`sync_state` bookmark table) and batches documents into Meilisearch indexes in chunks of 5,000 documents.
  - Automatically handles initial full rebuilds on startup or scheduled full re-syncs every 120 minutes.
  - Dispatches health heartbeats to Healthchecks.io and push notifications via Ntfy.sh if replication lags.

---

### 3.8 `apps/tunnel` — WebSocket WireGuard Encapsulator (`wstunnel`)
* **Language & Components**: Alpine Linux, `wstunnel` (Rust binary), `wireguard-tools`, `iptables`.
* **Responsibilities**:
  - Solves the problem of ISP Deep Packet Inspection (DPI) blocking or throttling raw WireGuard UDP traffic.
  - **Server Mode (`wstunnel-server`)**: Runs on `gaia-gateway`. Listens on localhost port 8443, decapsulates incoming WebSocket frames into UDP packets, and forwards them to local WireGuard.
  - **Client Mode (`wstunnel-client`)**: Runs on `gaia-portal`. Connects outbound to `wss://gateway:443/wstunnel` over TLS 1.3 and encapsulates local WireGuard traffic into binary WebSocket packets.

---

### 3.9 `apps/backup` — Automated Disaster Recovery Worker
* **Language & Components**: Alpine Linux, PostgreSQL client (`pg_dump`), Rclone.
* **Responsibilities**:
  - Runs on a cron schedule (`0 2 * * *` — 2:00 AM daily) to take complete PostgreSQL database dumps.
  - Compresses dumps using gzip and uploads encrypted backups to Google Drive or S3-compatible remote cloud storage via Rclone.
  - Enforces retention policies, pruning old backups to maintain the last N snapshots (`BACKUP_KEEP_COUNT=2`).

---

### 3.10 `apps/logger` — Distributed Log Shipper & Janitor
* **Language & Components**: Node.js / Express, Docker.
* **Responsibilities**:
  - **Shipper Profile**: Runs on remote nodes (`gaia-node`), scans local log directories, batches entries, and ships them securely over HTTP to the centralized receiver.
  - **Receiver / Server Profile**: Runs on `workspace-production`, ingests log streams, indexes log entries for dashboard queries, and runs automated janitor routines pruning logs older than `LOG_RETENTION_DAYS=7`.

---

## 4. Deployment Targets & Infrastructure Roles

GAIA organizes its deployment into 4 distinct target environments in `deploy/targets/`:

| Target | Location | Network Access | Containers Hosted |
| :--- | :--- | :--- | :--- |
| **`workspace-production`** | Homelab Server | Private LAN (No public IP) | `gaia-postgres`, `gaia-dashboard`, `gaia-classifier`, `gaia-backup`, `gaia-logger` |
| **`gaia-gateway`** | Offshore VPS | Public IP / Port 80 & 443 | `gaia-gateway-nginx`, `gaia-gateway-wstunnel`, `gaia-gateway-wireguard` |
| **`gaia-portal`** | Homelab DMZ | Private LAN (Tunnel egress) | `gaia-portal-web`, `gaia-portal-api`, `gaia-portal-meilisearch`, `gaia-portal-redis`, `gaia-portal-sync`, `gaia-portal-wstunnel-client`, `gaia-portal-wireguard-client` |
| **`gaia-node`** | External VPS | Public IP / UDP Port 6881 | `gaia-crawler`, `gaia-logger` |

---

## 5. Air-Tight Homelab OpSec & ISP-Blindness Model

The homelab runs in a hardened state where your ISP cannot detect the nature of your traffic or associate your home connection with BitTorrent activity:

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                 THE 6 OPSEC PILLARS                                    │
├───────────────────────────────┬────────────────────────────────────────────────────────┤
│ 1. Zero Public DNS            │ No domain names or public A/AAAA records point to your │
│                               │ home IP. All public traffic hits `gaia-gateway`.       │
├───────────────────────────────┼────────────────────────────────────────────────────────┤
│ 2. Zero Inbound Ports         │ The home router has zero open or forwarded ports. All  │
│                               │ tunnel traffic is initiated outbound from homelab.     │
├───────────────────────────────┼────────────────────────────────────────────────────────┤
│ 3. TLS 1.3 WebSocket Tunnel   │ WireGuard is packetized inside `wstunnel` TLS 1.3 frames│
│                               │ on port 443. To ISP DPI, it looks like banking HTTPS. │
├───────────────────────────────┼────────────────────────────────────────────────────────┤
│ 4. Encrypted DNS-over-TLS     │ All homelab DNS queries are encrypted over TCP port 853│
│    (DoT) via systemd-resolved │ to Cloudflare/Quad9. Outbound plaintext UDP 53 blocked.│
├───────────────────────────────┼────────────────────────────────────────────────────────┤
│ 5. Kernel Egress Killswitch   │ Linux `iptables` blocks all outbound HTTP (80) & HTTPS │
│                               │ (443) to the public internet except to Gateway IP.     │
├───────────────────────────────┼────────────────────────────────────────────────────────┤
│ 6. Bandwidth Traffic Shaping  │ Gateway Nginx caps per-connection throughput at        │
│                               │ `limit_rate 1500k;`, mimicking standard Netflix 4K.   │
└───────────────────────────────┴────────────────────────────────────────────────────────┘
```

### The Kernel Egress Killswitch (`/etc/iptables/gaia-killswitch.sh`)
```bash
# 1. Allow local loopback and LAN management
iptables -A OUTPUT -o lo -j ACCEPT
iptables -A OUTPUT -d 192.168.0.0/16 -j ACCEPT
iptables -A OUTPUT -d 10.0.0.0/8 -j ACCEPT

# 2. Allow Tailscale and WireGuard tunnel interfaces
iptables -A OUTPUT -o tailscale0 -j ACCEPT
iptables -A OUTPUT -o wg0 -j ACCEPT

# 3. Allow Encrypted DNS-over-TLS (port 853)
iptables -A OUTPUT -p tcp --dport 853 -j ACCEPT

# 4. Allow Outbound Port 443 ONLY to the whitelisted Gateway IP
iptables -A OUTPUT -p tcp -d "$GATEWAY_IP" --dport 443 -j ACCEPT

# 5. KERNEL EGRESS KILLSWITCH: Drop all unencrypted DNS (53) & public web (80/443)
iptables -A OUTPUT -p udp --dport 53 -j REJECT --reject-with icmp-port-unreachable
iptables -A OUTPUT -p tcp --dport 53 -j REJECT --reject-with icmp-port-unreachable
iptables -A OUTPUT -p tcp --dport 80 -j REJECT --reject-with icmp-net-unreachable
iptables -A OUTPUT -p tcp --dport 443 -j REJECT --reject-with icmp-net-unreachable
```

---

## 6. Data Pipeline & Storage Architecture

### Primary PostgreSQL Tables (`craw` Database)
* **`torrents`**: Master payload record (`infohash` BYTEA PK, `name`, `total_size`, `piece_length`, `file_count`, `category`, `category_confidence`, `health_score`, `popularity_score`, `verified_at`).
* **`torrent_files`**: Multi-file torrent hierarchy (`infohash`, `file_index`, `path`, `size_bytes`).
* **`peer_torrents`**: Discovered live peer endpoints (`infohash`, `peer_ip`, `peer_port`, `verified_at`).
* **`fetch_peer_outcomes`**: Telemetry log of BEP-9 metadata handshakes (`infohash`, `peer`, `result`, `created_at`).
* **`sync_state`**: Checkpoint cursor tracking Meilisearch synchronizer progress.

### Meilisearch Index (`torrents`)
* **Primary Key**: `infohash` (hex-encoded 40 characters).
* **Searchable Attributes**: `name`, `category`, `tags`.
* **Filterable Attributes**: `category`, `total_size`, `verified_at`, `health_score`.
* **Sortable Attributes**: `verified_at`, `total_size`, `popularity_score`, `health_score`.

---

## 7. Operations & Server Migration Runbook

### Deploying Services with `deploy.sh`
```bash
# Deploy a single target
./deploy/scripts/deploy.sh workspace-production HEAD "dashboard" --force-recreate

# Deploy the entire stack in dependency order
./deploy/scripts/deploy.sh --stack "gaia-gateway,gaia-portal,workspace-production" HEAD --force-recreate --verify
```

### Switching Gateway VPS in 30 Seconds (Plug-and-Play)
When migrating to a new VPS:
```bash
# 1. Update the new IP in deploy/targets/gaia-gateway/.env
DEPLOY_HOST="new.vps.ip"

# 2. Deploy Gateway and Portal
./deploy/scripts/deploy.sh gaia-gateway HEAD --force-recreate
./deploy/scripts/deploy.sh gaia-portal HEAD --force-recreate

# 3. Update the homelab OpSec (automatically whitelists the new Gateway IP)
./deploy/scripts/setup-homelab-opsec.sh workspace-production
./deploy/scripts/setup-homelab-opsec.sh gaia-portal

# 4. Verify complete leak immunity (Must show 8 PASSED, 0 FAILED)
./deploy/scripts/verify-opsec.sh workspace-production
./deploy/scripts/verify-opsec.sh gaia-portal
```
