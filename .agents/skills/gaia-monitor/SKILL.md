---
name: gaia-monitor
description: >-
  Monitors and analyzes the overall health, performance, throughput, and operations of the GAIA ecosystem.
  Use when checking system health, analyzing crawler ingestion rates (torrents/hr), auditing container fleets,
  evaluating classification and ML scoring pipelines, detecting anomalies or throughput drops, inspecting
  database performance and search replication lag, or diagnosing bugs, issues, and bottlenecks.
---

# GAIA Ecosystem Health & Performance Monitoring Skill

This skill equips the agent to act as a dedicated **Site Reliability & Performance Engineering specialist** for the GAIA ecosystem. It monitors the complete ingestion pipeline, audits all 24+ containers across 4 deployment targets (`workspace-production`, `gaia-gateway`, `gaia-portal`, `gaia-node`), diagnoses system bottlenecks, checks network tunnels and OpSec killswitches, inspects database throughput, scans error logs, and delivers an executive Health & Performance Report with actionable remediation steps.

---

## Architecture & Subsystem Matrix

The GAIA infrastructure consists of 4 distinct deployment targets connected via an encrypted WireGuard/wstunnel mesh:
- **`workspace-production`** (Homelab Core — `100.87.194.112`): Hosts master PostgreSQL (`craw`), PgBouncer, Redis, Classifier, ML Supervisor, Logger, Backup, and Dashboard.
- **`gaia-gateway`** (Offshore VPS Proxy — `192.168.10.111`): Ingress SSL termination, Nginx rate-limiting, and wstunnel server (`8443`).
- **`gaia-portal`** (Public Web Node — `192.168.10.139`): React Portal, .NET 10 API, Meilisearch full-text search, and CDC Synchronizer.
- **`gaia-node`** (Crawler Node — `100.66.211.64` / `gaia`): Rust Mainline DHT crawler and log shipper.

For full architectural diagrams and container mappings, consult [topology_map.md](./references/topology_map.md).

---

## When to Use This Skill

Activate this skill when:
- The user runs `/gaia-monitor` or asks about overall GAIA health, status, or performance.
- Investigating ingestion throughput, hourly torrent verification velocity (`torrents/hr`), or sudden drops in crawler volume.
- Inspecting classification batch latency, model inference timing (`fetch` vs `infer` vs `db commit`), or unclassified backlog growth.
- Checking container crashes, restart loops, or memory/CPU resource consumption.
- Auditing database buffer cache hit ratio, active locks, slow queries (> 5s), or PgBouncer connection pool saturation.
- Verifying search engine replication sync lag between PostgreSQL and Meilisearch.
- Auditing network tunnels (WireGuard handshake recency, wstunnel latency) or checking OpSec killswitch compliance.
- Identifying operational bugs, resource constraints, or system bottlenecks.

---

## Monitoring Execution Protocol

When conducting a health and performance inspection, follow this protocol:

### Step 1: Determine Inspection Scope
1. **Full Ecosystem Audit (Default)**: Inspects all 4 nodes, all containers, crawler trends, classifier timing, database metrics, and search replication.
2. **Quick Health Probe**: Rapid check of local containers, database connectivity, and gateway tunnel ping (< 5 seconds).
3. **Subsystem Deep Dive**: Targeted inspection of a specific subsystem (`crawler`, `classifier`, `database`, `search`, or `tunnels`).

### Step 2: Execute Telemetry Probes
Run the appropriate helper probe from the `scripts/` directory:

```bash
# 1. Full automated audit with bottleneck diagnosis (Recommended)
./.agents/skills/gaia-monitor/scripts/monitor_gaia.sh

# 2. Fast (< 5 sec) status check
./.agents/skills/gaia-monitor/scripts/monitor_gaia.sh --quick

# 3. Machine-readable JSON telemetry
./.agents/skills/gaia-monitor/scripts/monitor_gaia.sh --json

# 4. Target-specific container inspection
./.agents/skills/gaia-monitor/scripts/probe_containers_fleet.sh --target gaia-node
```

#### Individual Subsystem Probes
- **Crawler & Ingestion**: [probe_crawler_ingestion.sh](./scripts/probe_crawler_ingestion.sh)
- **Classifier & ML**: [probe_classifier_ml.sh](./scripts/probe_classifier_ml.sh)
- **Database & Storage**: [probe_database_storage.sh](./scripts/probe_database_storage.sh)
- **Search & CDC Replication**: [probe_search_sync.sh](./scripts/probe_search_sync.sh)
- **Container Fleet**: [probe_containers_fleet.sh](./scripts/probe_containers_fleet.sh)
- **Host Resources & OpSec**: [probe_hosts_network.sh](./scripts/probe_hosts_network.sh)

---

## Step 3: Diagnostic Analysis & Threshold Benchmarking

Evaluate the collected metrics against the calibrated SLA thresholds defined in [alert_thresholds.md](./references/alert_thresholds.md):

| Subsystem | Metric | Normal (🟢) | Warning (🟡) | Critical (🔴) |
| :--- | :--- | :--- | :--- | :--- |
| **Crawler** | Hourly Rate vs 24h Avg | `>= 80%` of avg | `50% - 79%` of avg | `< 50%` or 0 intake |
| **Classifier** | DB Write Time Ratio | `<= 30%` of batch | `31% - 50%` of batch | `> 50%` of batch |
| **Database** | Buffer Cache Hit Ratio | `>= 99.0%` | `95.0% - 98.9%` | `< 95.0%` |
| **Search Sync** | Replication Rebuild Age | `<= 135 mins` | `136 - 150 mins` | `> 150 mins` |
| **Host System**| 15m CPU Load Average | `<= 12.0` | `> 12.0` | `> 16.0` |
| **Host System**| RAM Available | `>= 1.5 GB` | `< 1.0 GB` | `< 500 MB` |
| **Networking** | WireGuard Handshake Age| `<= 180s` | `181s - 300s` | `> 300s` (Severed) |

---

## Step 4: Executive Report Synthesis

Synthesize the probe findings into the standard Executive Report format (see [sample_health_report.md](./examples/sample_health_report.md)):

1. **Header & Global Status Banner**:
   - `🟢 ALL SYSTEMS OPERATIONAL` | `🟡 DEGRADED PERFORMANCE` | `🔴 CRITICAL ALERT`
2. **Executive KPI Snapshot**: Table summarizing Ingestion Rate, Active Containers, Unclassified Backlog, Buffer Cache Hit Ratio, Search Sync Lag, and WireGuard Tunnel Latency.
3. **Ingestion & Crawler Dynamics**: 12-hour hourly verification history table, DHT routing table size, BEP-9 wire metadata success rate.
4. **Classification & ML Intelligence**: Batch throughput (rec/s), timing breakdown (`fetch` vs `infer` vs `db commit`), unclassified backlog depth, anomaly detector score.
5. **Storage, Database & Search Replication**: Database size, cache hit %, active/idle connections, top tables, Redis memory, Meilisearch indexed document count, CDC sync lag.
6. **Container Fleet Status Matrix**: Table of all containers with Status, Health, Restarts, CPU %, Memory.
7. **Host Infrastructure & OpSec Compliance**: Uptime, Load Average, RAM, Disk, WireGuard handshake recency, DNS-over-TLS status, Egress killswitch audit.
8. **Bottleneck & Anomaly Diagnostic Matrix**: Distinct cards identifying root causes for any detected bottleneck.
9. **Actionable Remediation Runbook**: Copy-paste safe shell commands to resolve identified issues (referencing [troubleshooting_runbooks.md](./references/troubleshooting_runbooks.md)).

---

## Diagnostic Runbook Reference Links

When an anomaly or bottleneck is discovered, reference the dedicated remediation runbooks:
- [High Host Load & CPU Throttling](./references/troubleshooting_runbooks.md#1-high-host-load--cpu-bottlenecks)
- [Ingestion Drop / Crawler Stagnation](./references/troubleshooting_runbooks.md#2-ingestion-drop--crawler-stagnation)
- [Database Commit Contention in Classifier](./references/troubleshooting_runbooks.md#3-database-write-contention-in-classifier-db-xs-bottleneck)
- [Search Replication Lag / Meilisearch Sync Delay](./references/troubleshooting_runbooks.md#4-search-replication-lag-meilisearch-sync-delay)
- [WireGuard / wstunnel Link Severed](./references/troubleshooting_runbooks.md#5-wireguard--wstunnel-link-severed)
- [Disk Exhaustion & Log Pruning](./references/troubleshooting_runbooks.md#6-disk-exhaustion--85-used)
