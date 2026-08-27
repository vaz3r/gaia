# Migration Plan: Crawler to OVH VPS-1

## Overview

Migrate the Gaia DHT crawler from zerone (Oracle Cloud ARM64) to OVH VPS-1 (x86-64). PostgreSQL and dashboard remain on current infrastructure.

## New VPS Specs

| Resource | Value |
|---|---|
| Plan | OVH VPS-1 |
| CPU | 2 vCore |
| RAM | 4 GB |
| Disk | 40 GB NVMe |
| Network | Unlimited, 500 Mbps |
| Monthly Cost | ~$5.35 (ex. tax) |

## Architecture After Migration

```
OVH VPS-1 "gaia" (2 vCores, 4GB RAM, 500Mbps)
├── Crawler (Docker)
├── Tailscale (user-installed)
└── SSH access
        │
        │ Tailscale tunnel (100.x.x.x)
        ▼
workspace-production (DB)
├── PostgreSQL (unchanged)
└── Tailscale
```

## Pre-Migration Checklist

- [x] Create OVH VPS-1 instance
- [x] Create user account (recommend `ubuntu` for script compatibility)
- [x] Install Docker + Docker Compose
- [x] Install Tailscale, authenticate to workspace-production network
- [x] Install Git
- [x] Add SSH key from local machine
- [x] Set hostname to `gaia`
- [x] Add `ubuntu` to docker group
- [x] Set up GitHub credentials on gaia (`.git-credentials`)
- [x] Clone repo to `/home/ubuntu/gaia`
- [x] Create data dirs (UID 10001): `/home/ubuntu/gaia-data/{crawler,logs}`
- [x] Open UDP 6882 in OVH Control Panel firewall
- [x] Verify Tailscale connectivity to workspace-production
- [x] Verify PostgreSQL is reachable

## File Changes Required

### 1. `.env` (Production Environment)

| Variable | Current Value | New Value |
|---|---|---|
| `DEPLOY_USER` | (new) | `ubuntu` |
| `CRAW_EXTERNAL_IP` | `132.145.189.201` | `135.125.131.176` (gaia public IP) |
| `DB_HOST` | `100.87.194.112` | `100.87.194.112` (unchanged, workspace-production) |
| `CRAW_WORKERS` | `8` | `4` (match 2 vCores) |

### 1b. `deploy/config.env` (Deployment Infrastructure)

| Variable | New Value |
|---|---|
| `DEPLOY_HOST` | `gaia` |
| `DEPLOY_SERVICES` | `crawler` (dashboard stays on zerone) |
| `DEPLOY_SSH_KEY` | `${HOME}/.ssh/zerone` (same key reused) |

### 2. `deploy/scripts/deploy.sh`

| Line | Current | New |
|---|---|---|
| 12 | `HOST="${1:-zerone}"` | `HOST="${1:-gaia}"` |
| 14 | `REMOTE_GIT="/home/ubuntu/gaia"` | `REMOTE_GIT="/home/<user>/gaia"` |
| 16 | `SSH_KEY="${HOME}/.ssh/zerone"` | `SSH_KEY="${HOME}/.ssh/gaia"` |
| 51 | `/home/ubuntu/gaia-data/` | `/home/<user>/gaia-data/` |

### 3. `deploy/scripts/config-restart.sh`

| Line | Current | New |
|---|---|---|
| 10 | `HOST="${1:-zerone}"` | `HOST="${1:-gaia}"` |
| 11 | `REMOTE_GIT="/home/ubuntu/gaia"` | `REMOTE_GIT="/home/<user>/gaia"` |
| 13 | `SSH_KEY="${HOME}/.ssh/zerone"` | `SSH_KEY="${HOME}/.ssh/gaia"` |

### 4. `deploy/scripts/health.sh`

| Line | Current | New |
|---|---|---|
| 21 | `HOST="zerone"` | `HOST="gaia"` |
| 48 | `SSH_KEY="${HOME}/.ssh/zerone"` | `SSH_KEY="${HOME}/.ssh/gaia"` |
| 50 | `LOG_DIR="/home/ubuntu/gaia-data/logs"` | `LOG_DIR="/home/<user>/gaia-data/logs"` |
| 64 | Hardcoded `DB_CONN` with old IP/password | Update to new Tailscale IP |

### 5. `deploy/compose/docker-compose.yml`

| Line | Current | New |
|---|---|---|
| 36 | `/home/ubuntu/gaia-data/crawler` | `/home/<user>/gaia-data/crawler` |
| 37 | `/home/ubuntu/gaia-data/logs` | `/home/<user>/gaia-data/logs` |
| 38 | `/home/ubuntu/gaia/apps/crawler/config` | `/home/<user>/gaia/apps/crawler/config` |

## Config Tuning for 4GB RAM

Update `apps/crawler/config/production.toml`:

```toml
[dht]
walker_alpha = 3
walker_interval_ms = 250
source_deadline_ms = 15000
source_max_queries = 24
source_query_timeout_secs = 5
sybil_count = 16
rate_limit_per_sec = 8.0

[fetch]
global_fetch_limit = 1200
race_peers = 8
metadata_timeout_secs = 25
utp_enabled = true

[storage]
pg_pool_max_connections = 40  # Reduce from 128 to match PostgreSQL max_connections=50

[logging]
log_dir = "/data/logs"
log_json = true
log_file_max_bytes = 50000000
log_total_max_bytes = 500000000
log_flush_interval_ms = 500
log_buffer_capacity = 8192
```

## Deployment Steps

### Step 1: Update Configuration

1. Edit `.env` with new VPS IP and Tailscale DB host
2. Edit deploy scripts with new hostname, SSH key, and paths
3. Edit docker-compose.yml with new volume mount paths
4. Update `production.toml` if needed (pg_pool_max_connections)

### Step 2: Deploy

```bash
# From local machine
./deploy/scripts/deploy.sh gaia
```

### Step 3: Verify

```bash
# Check crawler is running
ssh gaia 'docker ps --filter name=gaia-crawler'

# Run health check
./deploy/scripts/health.sh --window 60 --no-logs

# Monitor for 24 hours before decommissioning zerone crawler
```

## Risk Assessment

### DB Connection Pool Mismatch

The crawler's `pg_pool_max_connections=128` exceeds PostgreSQL's `max_connections=50`. This may cause connection refused errors under load. Fix: set `pg_pool_max_connections = 40` in `production.toml`.

### CPU on Shared vCores

OVH VPS-1 uses shared/burstable vCores. At ~11,000 PPS with HMAC-SHA1 token verification, CPU may spike during DHT traffic bursts. Mitigation: `CRAW_FIND_NODE_RESPONSE_PERCENT=5` keeps CPU at ~33% on 4 cores.

### Network Latency to DB

Database writes go over Tailscale to workspace-production. Current batch write intervals (1s for torrents, 30s for peer outcomes, 60s for metrics) are sufficient to mask latency. No changes needed.

### No Disk Spool

BatchWriter uses in-memory buffers only. If DB is unreachable, data is lost after buffer fills. This is acceptable for a crawler (data is discoverable again).

## Rollback Plan

If the new VPS fails:

1. Stop crawler on new VPS: `ssh gaia 'docker stop gaia-crawler'`
2. Start crawler on zerone: `ssh zerone 'docker start gaia-crawler'`
3. No data loss (DB unchanged)

## Monitoring Commands

```bash
# Health check (1-hour window)
./deploy/scripts/health.sh --window 60

# Real-time Docker logs
ssh gaia 'docker logs -f gaia-crawler --tail 100'

# CPU/memory usage
ssh gaia 'docker stats gaia-crawler --no-stream'

# Database connectivity
ssh gaia 'docker run --rm postgres:16 psql "postgres://crawler:<password>@<DB_IP>:5432/craw?sslmode=disable" -c "SELECT count(*) FROM torrents"'
```

## Decision Thresholds

### Keep VPS-1 if:
- Average CPU remains below 75%
- No individual vCPU pinned near 100%
- UDP receive errors remain at zero
- Database batches remain current
- Verified torrents comparable to zerone

### Upgrade to VPS-2 if:
- Average CPU regularly exceeds 80%
- One core consistently saturated
- Packet loss increases at 25,000 PPS
- Crawler becomes unstable during bursts
- Remote database serialization consumes noticeable CPU

### Do NOT move PostgreSQL to VPS-1 if:
- Crawler already consumes 1.3-1.7 cores
- PostgreSQL introduces unpredictable CPU, memory, disk I/O spikes
- RAM might fit but CPU contention would be worse

## Migration Status (2026-08-27)

**COMPLETE.** Crawler migrated from zerone (Oracle ARM64) to gaia (OVH VPS-1, x86-64).

### What Changed
- **Dockerfile**: removed hardcoded `--platform=linux/arm64`, builds natively per host
- **`deploy/config.env`**: `DEPLOY_HOST=gaia`, `DEPLOY_SERVICES=crawler` (crawler-only host)
- **`deploy.sh`**: builds/recreates configurable `$DEPLOY_SERVICES`; skips dashboard index init when dashboard not deployed
- **`.env`**: `CRAW_EXTERNAL_IP=135.125.131.176`, `CRAW_WORKERS=4`
- **SSH config**: added `gaia` alias (Tailscale IP `100.66.211.64`, reuses zerone key)
- **gaia setup**: hostname set, docker group added, GitHub credentials, repo cloned, data dirs (UID 10001), port UDP 6882 opened in OVH panel

### Verified Working
- Crawler binds 4x UDP sockets on `0.0.0.0:6882` (matches `CRAW_WORKERS=4`)
- Public DHT reachability confirmed (probe from outside: ~5% find_node responses matching `find_node_response_percent=5`)
- Inbound traffic flowing: `inbound_find_node=545`, `inbound_get_peers=1144`
- Harvesting + verifying: `harvested=1184`, routing table 187 nodes, `verified_per_hour=480`
- `health.sh` targets gaia correctly via `deploy/config.env`

### Post-Migration Notes
- Only `gaia-crawler` runs on gaia; dashboard remains on zerone
- DB stays on workspace-production (via Tailscale)
- Run `docker compose up -d --force-recreate --no-build crawler` (config-restart.sh) for config-only changes
- Monitor CPU: 2 shared vCores, `CRAW_FIND_NODE_RESPONSE_PERCENT=5` keeps CPU manageable
