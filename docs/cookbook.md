# GAIA System Cookbook

A practical, no-jargon guide to running the GAIA BitTorrent DHT crawler. Covers day-to-day operations: monitoring, maintenance, configuration, deployment, and troubleshooting.

---

## 1. System Overview

GAIA crawls the BitTorrent DHT network to discover torrents and fetch their metadata (name, size, file list).

### Architecture (current)

| Component | Where it runs | What it does |
|---|---|---|
| **Crawler** | `gaia` VPS (OVH, x86-64) | Talks to the DHT network, harvests infohashes, fetches torrent metadata, writes to PostgreSQL |
| **Dashboard** | `zerone` VPS (Oracle, ARM64) | Web UI for viewing stats and metrics (read-only from DB) |
| **PostgreSQL** | `workspace-production` (`100.87.194.112`) | Stores all data: torrents, jobs, metrics |

### Hosts & How to Reach Them

Use these SSH aliases (configured in `~/.ssh/config`). `gaia` is reached over Tailscale; `zerone` and `workspace-production` are directly reachable.

```bash
ssh gaia       # crawler host (OVH VPS)
ssh zerone     # dashboard host (Oracle, old crawler host)
```

`workspace-production` has no SSH alias; it's accessed from `gaia` or via PostgreSQL directly.

### Database Access

The database lives at `100.87.194.112:5432`, database `craw`, user `crawler`. The password is in `deploy/config.env` (`DB_PASSWORD`).

To run SQL (from your local machine):

```bash
source deploy/config.env
DB="postgres://${DB_USER}:${DB_PASSWORD}@${DB_HOST}:${DB_PORT}/${DB_NAME}?sslmode=disable"
docker run --rm --network host postgres:16 psql "$DB" -c "SELECT count(*) FROM torrents;"
```

---

## 2. Monitoring (The Single Most Important Tool)

Everything you need for day-to-day monitoring is a single script: **`deploy/scripts/health.sh`**. It runs read-only; it never writes to the database.

### Quick Health Check

```bash
cd deploy/scripts
./health.sh                    # last 15 minutes
./health.sh --window 60        # last 1 hour
./health.sh --window 60 --no-logs   # skip log scanning (faster)
```

### Other Useful Views

```bash
./health.sh --json        # machine-readable JSON output
./health.sh --all         # minute-by-minute metric history
./health.sh --host zerone # check a specific host (default: gaia)
```

### What the Sections Mean

| Section | What it tells you |
|---|---|
| **CHANNELS** | Pipeline buffers. `fresh_channel_depth` near its max (65536) = input overflows the pipeline. `fresh_channel_dropped` > 0 = infohashes being thrown away. |
| **THROUGHPUT** | The money numbers. `verify_success` = torrents confirmed per hour. `fetch_attempts` = connection attempts. `tcp/utp_metadata_ok` = metadata fetched over each transport. |
| **SOURCE** | DHT lookup health. `source_queries` = DHT lookups made. `source_timeout` high = many lookups timing out. |
| **SCHEDULER** | Retry job treadmill. `scheduler_claims` = jobs re-processed per hour. |
| **CONNECT** | Connection success. `tcp_connect_ok` / `tcp_attempts` ratio = how often connections succeed. |
| **HARVEST/DHT** | Inbound network traffic. `inbound_get_peers` = discovery queries. `inbound_find_node_dropped` = throttled responses (expected to be high). |
| **JOBS / DB** | Database state. `pending` = queued jobs, `verifying` = in-flight, `failed`/`dead` = finished. `torrents: 1h` = new verified in last hour. |
| **FLAGS** | Problems detected. Any line under `-- FLAGS --` needs attention. |

### Reading the Numbers

- **`rate=`** numbers are normalized to **per-hour** rates (e.g., `verify_success rate=1.2k` = ~1,200 torrents/hour).
- **`dlt=`** is the raw count change in the window.
- `gauge=` is an instantaneous buffer depth.

### Expected Ranges (Healthy Operation)

| Metric | Healthy | Concerning |
|---|---|---|
| `verify_success` | Steady, growing with DHT presence | Drops to near 0 for hours |
| `fresh_channel_dropped` | 0 | >0 for extended periods after warmup |
| `source_timeout` | < 50% of `source_queries` | > 70% |
| `inbound_find_node_dropped` | ~95% of `inbound_find_node` (by design) | Dropping close to 0% (throttle misconfigured) |
| Log anomalies | 0 errors, 0 panics | Any panic, repeated errors |
| Restarts | 0 | > a few |

---

## 3. Day-to-Day Maintenance

### Backup the Database

```bash
# One-time manual backup (saves to repo root)
cd deploy/scripts
ssh workspace-production-or-gaia 'docker exec craw-db pg_dump -U crawler craw' > backup-$(date +%Y%m%d).sql
```

Full backup + PostgreSQL config deployment is automated in `./db-init.sh` (see §6).

### Check Disk Space

```bash
ssh gaia 'df -h /'
# Logs live in /home/ubuntu/gaia-data/logs (rotated automatically to ~500MB max)
```

If `gaia-data/logs` grows unexpectedly, check rotation:
```bash
ssh gaia 'ls -lah /home/ubuntu/gaia-data/logs/ | head'
```

### Clean Up Old Data (Janitor)

The crawler has a built-in janitor that automatically deletes:
- `dead` verification jobs older than 24h
- `verified` jobs older than 1h

This runs every 30 minutes on its own. You shouldn't need to do anything.

If you need to manually clean an oversized table:

```sql
-- Count dead jobs
SELECT count(*) FROM verification_jobs WHERE status='dead';

-- Manually delete dead jobs older than 24h (only if janitor lags)
DELETE FROM verification_jobs WHERE status='dead' AND updated_at < now() - interval '1 day';
```

### View Crawler Logs

```bash
# Live tail from the container
ssh gaia 'docker logs -f gaia-crawler --tail 50'

# Latest JSONL log file (structured, in /home/ubuntu/gaia-data/logs/)
ssh gaia 'ls -t /home/ubuntu/gaia-data/logs/crawler-*.jsonl | head -1 | xargs tail -5'

# Search for errors in the last log file
ssh gaia 'grep -l error /home/ubuntu/gaia-data/logs/*.jsonl | tail -1 | xargs grep error | tail -20'
```

---

## 4. Configuration

Configuration is layered. Higher layers override lower ones:

```
built-in defaults  <  default.toml  <  {CRAW_PROFILE}.toml  <  .env (CRAW_* vars)
```

The active profile is `production` (set in the Dockerfile).

### The Two Config Files

| File | Purpose | Example of keys |
|---|---|---|
| **`apps/crawler/config/default.toml`** | Baseline every deploy inherits | All tunables |
| **`apps/crawler/config/production.toml`** | Production overrides (checked into git) | `sybil_count`, `rate_limit_per_sec`, logging paths |
| **`.env`** (repo root) | Runtime overrides fed to Docker Compose as `CRAW_*` env vars | `CRAW_EXTERNAL_IP`, `CRAW_WORKERS`, DB creds |

**Rule of thumb:** Put host-specific values (IPs, per-machine tuning like `CRAW_WORKERS`) in `.env`. Put functional defaults that all deployments should share in `production.toml`.

### Important Config Values (Current Production)

| Key | Current | What it controls |
|---|---|---|
| `CRAW_EXTERNAL_IP` | `135.125.131.176` | gaia's public IP; used to generate BEP42 node IDs and advertise itself |
| `CRAW_WORKERS` | `4` | Number of UDP listener sockets (should roughly match CPU cores) |
| `CRAW_FIND_NODE_RESPONSE_PERCENT` | `5` | **CPU throttle.** Respond to only 5% of inbound `find_node` queries. Lower = less CPU, less DHT presence. Higher = more discovery, more CPU. |
| `CRAW_PIPELINE_LIMIT` | `4000` | Max concurrent infohash verifications |
| `CRAW_FETCH_LIMIT` | `1200` | Max concurrent network connections |
| `CRAW_RACE_PEERS` | `8` | Peer attempts per infohash |
| `CRAW_PORT_BASE` | `6882` | UDP port the crawler listens on |
| `CRAW_FETCH_TIMEOUT_MS` | `25000` | Max time to download a torrent's metadata |
| `CRAW_SOURCE_DEADLINE_MS` | `15000` | Max time for a DHT peer lookup |
| `CRAW_SYBILS` | `16` | Number of virtual DHT node identities |
| `CRAW_RATE_LIMIT` | `8.0` | Outbound queries/sec per remote IP |

> **Full reference:** every tunable is documented in `apps/crawler/config/default.toml` with comments. Read that file before changing anything.

### Tuning Cheat Sheet

| Goal | Change this |
|---|---|
| Reduce CPU usage | Lower `CRAW_FIND_NODE_RESPONSE_PERCENT` (e.g., 5 → 3) |
| Increase discovery | Raise `CRAW_FIND_NODE_RESPONSE_PERCENT` (e.g., 5 → 10) |
| Match crawler to CPU count | Set `CRAW_WORKERS` = number of cores |
| More concurrent fetches | Raise `CRAW_FETCH_LIMIT` (watch bandwidth) |
| Buffer more inbound harvests | Raise `fresh_channel_capacity` in `production.toml` |
| Recover dead peers faster | Lower `CRAW_CONNECT_DEADLINE_MS` |
| Reduce slow-metadata blocking | Lower `CRAW_FETCH_TIMEOUT_MS` (risks dropping slow peers) |

### How to Change a Config Value

1. Edit `.env` (for `CRAW_*` env vars) or `production.toml` (for TOML-only keys).
2. Commit + push to GitHub.
3. Re-apply on the host with `./config-restart.sh` (see §6) or `./deploy.sh`.
4. Verify the running value:

```bash
ssh gaia 'grep -h "effective config" /home/ubuntu/gaia-data/logs/*.jsonl | tail -1 | python3 -m json.tool'
```

This prints the config the running crawler actually loaded.

---

## 5. Deployment

### Prerequisites on a Fresh Host

Done once when a new crawler host is provisioned:

1. Ubuntu user (scripts assume `ubuntu`), passwordless sudo.
2. Docker + Docker Compose installed.
3. The SSH key from this repo's machine added to `ubuntu@host` (scripts use `~/.ssh/zerone`).
4. Hostname set if desired (`sudo hostnamectl set-hostname <name>`).
5. `ubuntu` added to docker group: `sudo usermod -aG docker ubuntu`.
6. Tailscale installed and authenticated (to reach PostgreSQL at `100.87.194.112`).
7. GitHub credentials so the host can `git clone` the private repo:
   ```bash
   echo "https://vaz3r:YOUR_PAT@github.com" | sudo tee /home/ubuntu/.git-credentials
   sudo chown ubuntu /home/ubuntu/.git-credentials && sudo chmod 600 /home/ubuntu/.git-credentials
   sudo -u ubuntu git config --global credential.helper store
   ```
8. Clone the repo: `sudo git clone https://github.com/vaz3r/gaia.git /home/ubuntu/gaia` (then `chown -R ubuntu`).
9. Create data dirs (owned by container UID 10001):
   ```bash
   sudo mkdir -p /home/ubuntu/gaia-data/crawler /home/ubuntu/gaia-data/logs
   sudo chown -R 10001:10001 /home/ubuntu/gaia-data
   ```
10. **Open the UDP port** (e.g. 6882) in the cloud provider's firewall (OVH Control Panel) — the OS firewall does NOT do this. Without it the crawler is invisible to the DHT.
11. Add an SSH alias in `~/.ssh/config` if you want a named host.

### The Deploy Script

**`./deploy/scripts/deploy.sh`** is the one-command deployment. It:

1. Pushes the repo to the target (git fetch/checkout a ref)
2. Ensures data dirs + permissions
3. Builds the Docker image **on the target host** (never cross-compiles)
4. Recreates containers
5. Runs dashboard DB index init (skipped on crawler-only hosts)
6. Prints status

```bash
# Deploy whatever is committed on origin/main
./deploy/scripts/deploy.sh gaia

# Deploy a specific commit (rollback)
./deploy/scripts/deploy.sh gaia <commit-sha>

# Deploy to a specific host (overrides config)
./deploy/scripts/deploy.sh <hostname>
```

**Important:** deploy.sh deploys committed code. Make sure your changes are committed and pushed to GitHub first, because the host pulls from GitHub:

```bash
git add -A && git commit -m "change description" && git push origin main
./deploy/scripts/deploy.sh gaia
```

### Restart Without Rebuilding

For config-only changes (`production.toml` is bind-mounted read-only from the host), you don't need a rebuild — just restart with the existing image:

```bash
./deploy/scripts/config-restart.sh gaia
```

This is faster and leaves the binary untouched.

### Which Services Deploy Where

Control this per host in `deploy/config.env`:

```
DEPLOY_SERVICES="crawler"          # gaia: crawler only
DEPLOY_SERVICES="crawler dashboard" # a host that also runs the dashboard
```

---

## 6. File & DB Maintenance Scripts

| Script | What it does | When to run |
|---|---|---|
| `deploy/scripts/deploy.sh` | Full deploy (build + restart) | Code/config changed, needs rebuild |
| `deploy/scripts/config-restart.sh` | Restart with existing image | `production.toml` changed (no rebuild needed) |
| `deploy/scripts/health.sh` | Health + metrics report | Any time, read-only |
| `deploy/scripts/db-init.sh` | Backs up DB + applies PostgreSQL config to workspace-production | Rarely; when PG needs tuning |

### db-init.sh Warnings

`db-init.sh`:
- Backs up the whole DB (pg_dump) before making changes
- Recreates the `craw-db` container
- Requires working SSH to `workspace-production` as user `core`

Only run it if you understand PostgreSQL tuning. During normal operation the DB needs no attention.

---

## 7. Troubleshooting / Diagnosis

### Situation → Action

| Symptom | Likely cause | What to do |
|---|---|---|
| **Dashboard down** | Container crashed / redeploy | `ssh zerone 'docker ps --filter name=gaia-dashboard'`, check `docker logs` |
| **Crawler down / restarting** | Crash loop (bad config, DB unreachable) | `ssh gaia 'docker ps --filter name=gaia-crawler'`; `docker logs gaia-crawler --tail 50`; check the DB is reachable: `ssh gaia 'tailscale ping workspace-production'` |
| **`fresh_channel_dropped` climbing** | Pipeline can't keep up with DHT harvest | Raise `fresh_channel_capacity` (TOML) or `CRAW_FETCH_LIMIT`; restart |
| **STARVATION flag (pending=0, verifying=0)** | Nothing queued — crawler is discovery-driven, **usually benign** | Wait; check `verify_success` trend. If it's ~0 for hours, discovery or DB write is broken |
| **`verify_success` near 0 for a long time** | No DHT presence yet, or DB write failing, or port blocked | Check UDP port reachable (see below); check log for DB errors; wait for routing table to build (days) |
| **Very high CPU** | `find_node_response_percent` too high for the CPU count | Lower `CRAW_FIND_NODE_RESPONSE_PERCENT`; lower `CRAW_WORKERS` to cores |
| **No inbound traffic** (`inbound_get_peers` ~ 0) | UDP port blocked by provider firewall | Test reachability (below); open port in provider panel |
| **All metadata_timeout** | Far/weak peers; fine ratio | Check connect% — if connect% also near 0, likely network/port issue |
| **DB errors in logs** | workspace-production down / Tailscale broken | `ssh gaia 'tailscale ping workspace-production'`; `docker exec craw-db pg_isready` |

### Check If the DHT Port Is Reachable

Run from your local machine (any network):

```bash
python3 /tmp/opencode/dht_probe.py 135.125.131.176 6882
# Expect "1/20 responses" or more (crawler only answers ~5% of find_node by design)
# 0/20 = port blocked
```

A response rate of anywhere from 1–5 out of 20 means the port is open and the crawler is answering.

### Check the Running Config

```bash
ssh gaia 'grep -h "effective config" /home/ubuntu/gaia-data/logs/*.jsonl | tail -1 | python3 -m json.tool'
```

### Check DHT Health Over Time

```bash
# Routing table size growth (should climb over first hours/days)
ssh gaia 'ls -t /home/ubuntu/gaia-data/logs/crawler-*.jsonl | head -1 | xargs grep -o "\"routing_table\":[0-9]*" | tail -20'
```

If `routing_table` stalls at a small value (~tens), the crawler isn't building DHT presence — check port + external IP.

---

## 8. Common Operations Reference

### Stop / Start / Restart Crawler Manually

```bash
ssh gaia 'docker stop gaia-crawler'
ssh gaia 'docker start gaia-crawler'
ssh gaia 'docker restart gaia-crawler'
```

### See Live Resource Use

```bash
ssh gaia 'docker stats gaia-crawler --no-stream'
ssh gaia 'top -bn1 | head -8'
```

### How Much Torrent Data Do We Have?

```sql
SELECT count(*) FROM torrents;
SELECT count(*) FROM torrents WHERE verified_at > now() - interval '24 hours';
```

### Find Where Time Goes (Fetch Phase Analysis)

```sql
SELECT phase, transport, result, count(*),
       percentile_cont(0.5) WITHIN GROUP (ORDER BY elapsed_ms) AS p50_ms
FROM fetch_peer_outcomes
WHERE created_at > now() - interval '30 minutes' AND phase IS NOT NULL
GROUP BY 1,2,3 ORDER BY 5 DESC;
```

This shows metadata fetch timing per phase (connect vs metadata) and transport (tcp/utp).

### Migrating the Crawler to a New Host

The whole process is in `docs/migration-ovh-vps.md`. Short version:

1. Provision host, install Docker + Tailscale, add SSH key, clone repo, create data dirs.
2. Point `deploy/config.env` at the new host (`DEPLOY_HOST`, `DEPLOY_SSH_KEY`, `DEPLOY_SERVICES`).
3. Update `.env` — at minimum `CRAW_EXTERNAL_IP` (new public IP) and `CRAW_WORKERS` (cores).
4. Open the UDP port in the provider firewall.
5. `./deploy/scripts/deploy.sh <host>`.
6. Verify with `health.sh` and the port probe.

---

## 9. Safety Rules

1. **Never edit `.env` or `deploy/config.env` without committing**. They contain the DB password and are the source of truth for the running system.
2. **Verify the port is open** after any firewall/provider changes — the crawler silently degrades without it.
3. **health.sh is read-only — safe to run anytime.**
4. **db-init.sh recreates the DB container** — run only when needed; it backs up first.
5. **Don't judge throughput too early.** A freshly deployed crawler needs hours-to-days to build DHT routing-table presence. `verify_success` grows over time.
6. **Config precedence:** env vars win. If a value in `.env` differs from `production.toml`, `.env` wins.
7. **Deploy = commit, push, deploy.** The host builds from GitHub, so an uncommitted change will not be picked up.

---

## 10. Quick Command Card

```bash
# Health
./deploy/scripts/health.sh --window 60

# Deploy
./deploy/scripts/deploy.sh gaia

# Config restart (no rebuild)
./deploy/scripts/config-restart.sh gaia

# Logs
ssh gaia 'docker logs -f gaia-crawler --tail 50'

# Containers
ssh gaia 'docker ps'

# DB (from local, after: source deploy/config.env)
docker run --rm --network host postgres:16 psql "$DB" -c "SELECT count(*) FROM torrents;"

# Port probe
python3 /tmp/opencode/dht_probe.py 135.125.131.176 6882
```