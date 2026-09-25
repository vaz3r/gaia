# Crawler Performance, Memory Capping & Storage Janitor Tuning

## Overview
This document serves as an operational reference and rollback guide for the crawler performance tuning applied on **September 25, 2026**. 

The goal of this tuning is to ensure continuous high-throughput ingestion (**> 150k new torrents/day** and **> 8k torrents/hr**) on resource-constrained crawler nodes (such as `gaia-node`, equipped with 3.8 GB RAM).

---

## 1. Problem Statement & Root Cause Diagnosis

### Issue A: Crawler OOM-Kill Reboot Cycle
Prior to this tuning, `gaia-crawler` on `gaia-node` was experiencing fatal Out-Of-Memory (OOM) kills by the Linux kernel every 40–60 minutes:
- Memory ceiling reached ~3.1 GB anonymous RSS on a 3.8 GB RAM VM.
- Docker automatically restarted the container (RestartCount reached 14).
- Every restart completely destroyed the in-memory DHT routing table, resetting the crawler back to 7 bootstrap nodes and plunging hourly throughput to near zero for 10–15 minutes while the DHT routing table re-populated.

### Issue B: PostgreSQL Storage Janitor I/O Stalls
Every 5 minutes (`300s`), the crawler executed a massive batch deletion (`janitor_batch_size = 100000`) on `verification_jobs` and `infohash_sightings`. This flooded PostgreSQL with dead tuples, triggering intensive autovacuum runs (`autovacuum: VACUUM public.verification_jobs`) that locked NVMe/SATA disks at 98% utilization and stalled new torrent inserts for up to 54 seconds per query.

---

## 2. Parameter Tuning Matrix

Configuration file: [`apps/crawler/config/production.toml`](../apps/crawler/config/production.toml)

| Section | Parameter | Baseline (Old) | Tuned (New) | Architectural Rationale |
| :--- | :--- | :--- | :--- | :--- |
| `[fetch]` | `conn_limiter_max_entries` | `500000` | `150000` | Drastically reduces memory consumption of IP connection tracking table while maintaining 60s cooldown. |
| `[fetch]` | `ip_cooldown_max_entries` | `200000` | `75000` | Caps blacklist/cooldown cache allocations in Rust memory. |
| `[fetch]` | `pipeline_limit` | `6000` | `4000` | Limits inflight fetch pipeline depth, preventing socket buffer queue accumulation during bursts. |
| `[fetch]` | `global_fetch_limit` | `2500` | `2000` | Prevents file descriptor and socket buffer exhaustion. |
| `[cache]` | `peer_cache_max_entries` | `250000` | `100000` | Lowers peer cache footprint. Swarm peers cycle rapidly, so 100k entries is more than sufficient for 120s TTL. |
| `[cache]` | `announce_cache_max_entries`| `100000` | `50000` | Caps announce cache entries to reduce resident memory. |
| `[storage]`| `janitor_interval_secs` | `300` | `60` | Runs cleanup more frequently in small bites rather than massive 5-minute spikes. |
| `[storage]`| `janitor_batch_size` | `100000` | `10000` | Eliminates disk I/O lock contention on PostgreSQL and prevents autovacuum freezes. |
| `[storage]`| `janitor_batch_sleep_ms` | `10` | `25` | Smooths database query pacing between deletion chunks. |

---

## 3. Rollback Procedure

If throughput drops significantly or unexpected errors arise, revert to the baseline configuration using these steps:

### Step 1: Revert `production.toml`
Update [`apps/crawler/config/production.toml`](../apps/crawler/config/production.toml):
```toml
[fetch]
pipeline_limit = 6000
global_fetch_limit = 2500
ip_cooldown_max_entries = 200000
conn_limiter_max_entries = 500000

[cache]
peer_cache_max_entries = 250000
announce_cache_max_entries = 100000

[storage]
janitor_interval_secs = 300
janitor_batch_size = 100000
janitor_batch_sleep_ms = 10
```

### Step 2: Deploy and Restart
```bash
# Push config to crawler node
scp apps/crawler/config/production.toml gaia:/home/ubuntu/gaia/apps/crawler/config/production.toml

# Restart crawler container
ssh gaia "docker restart gaia-crawler"
```

---

## 4. Verification and Telemetry Probes

Use these commands to verify steady-state operation:

```bash
# 1. Check crawler memory usage (RSS should stay < 1.8 GB)
ssh gaia "ps -p \$(pgrep -x crawler) -o pid,rss,pmem,vsz,etime"

# 2. Check kernel OOM logs (should show no new OOM events)
ssh gaia "sudo dmesg -T | grep -iE 'oom|killed process' | tail -n 10"

# 3. Check live ingestion velocity (new torrents in last 5 minutes)
ssh 192.168.10.10 "docker exec -i gaia-postgres psql -U crawler -d craw -c \"
SELECT 
    count(*) FILTER (WHERE verified_at > now() - interval '5 minutes') as verified_5m,
    count(*) FILTER (WHERE first_seen > now() - interval '5 minutes') as new_5m,
    round(count(*) FILTER (WHERE verified_at > now() - interval '5 minutes') * 12.0, 0) as verified_rate_hr
FROM torrents;\""
```
