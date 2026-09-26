# Crawler Performance, Memory Capping & Host Process Audit

## Overview
This document serves as an operational reference and rollback guide for the crawler performance tuning and host optimization applied on **September 25–26, 2026**. 

The goal of this tuning is to ensure continuous, uninterrupted high-throughput ingestion (**> 150k new torrents/day** and **> 8k torrents/hr**) on resource-constrained crawler nodes (such as `gaia-node`, equipped with 3.8 GB RAM).

---

## 1. Problem Statement & Root Cause Diagnosis

### Issue A: Crawler OOM-Kill Reboot Cycle & Rogue Benchmark Process
Prior to this tuning, `gaia-crawler` on `gaia-node` was experiencing fatal Out-Of-Memory (OOM) kills by the Linux kernel:
- Memory ceiling reached ~3.1 GB anonymous RSS on a 3.8 GB RAM VM.
- Furthermore, an **orphaned background benchmark process (`PID 218263` / `218267: sudo nohup /tmp/ab_full.sh`) running since August 29** was executing in a loop, periodically restarting the container and attaching `strace` to worker threads, directly provoking kernel OOM kills.
- Every restart completely destroyed the in-memory DHT routing table, resetting the crawler back to 7 bootstrap nodes and plunging hourly throughput to near zero for 10–15 minutes while the DHT routing table re-populated.
- **Remediation**: `ab_full.sh` was terminated with `SIGKILL`, and concurrency limits were scaled to keep working memory safely between 1.0 GB and 1.4 GB.

### Issue B: PostgreSQL Storage Janitor I/O Stalls
Every 5 minutes (`300s`), the crawler executed a massive batch deletion (`janitor_batch_size = 100000`) on `verification_jobs` and `infohash_sightings`. This flooded PostgreSQL with dead tuples, triggering intensive autovacuum runs (`autovacuum: VACUUM public.verification_jobs`) that locked NVMe/SATA disks at 98% utilization and stalled new torrent inserts for up to 54 seconds per query.
- **Remediation**: Re-paced the janitor to run every 60s with 10k batch size and 25ms sleep, eliminating autovacuum freezes.

---

## 2. Parameter Tuning Matrix

Configuration file: [`apps/crawler/config/production.toml`](../apps/crawler/config/production.toml)

| Section | Parameter | Baseline (Aug 2026) | Intermediate (Sep 25) | Right-Sized (Sep 26) | Architectural Rationale |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `[fetch]` | `global_fetch_limit` | `2500` | `2000` | **`800`** | Caps concurrent peer sockets at 6,400 (down from 16,000+), eliminating hundreds of megabytes in socket buffers. |
| `[fetch]` | `pipeline_limit` | `6000` | `4000` | **`1500`** | Limits inflight fetch pipeline depth, preventing socket queue accumulation during bursts. |
| `[fetch]` | `conn_limiter_max_entries` | `500000` | `150000` | **`30000`** | Drastically reduces heap-allocated `Arc<Semaphore>` entries in DashMap. |
| `[fetch]` | `ip_cooldown_max_entries` | `200000` | `75000` | **`20000`** | Caps blacklist/cooldown cache allocations in Rust memory. |
| `[cache]` | `peer_cache_max_entries` | `250000` | `100000` | **`30000`** | Lowers peer cache footprint; 30k entries is more than sufficient for 120s TTL. |
| `[cache]` | `announce_cache_max_entries`| `100000` | `50000` | **`20000`** | Caps announce cache entries to reduce resident memory. |
| `[storage]`| `janitor_interval_secs` | `300` | `60` | **`60`** | Runs cleanup frequently in small bites rather than massive 5-minute spikes. |
| `[storage]`| `janitor_batch_size` | `100000` | `10000` | **`10000`** | Eliminates disk I/O lock contention on PostgreSQL. |
| `[storage]`| `janitor_batch_sleep_ms` | `10` | `25` | **`25`** | Smooths database query pacing between deletion chunks. |

---

## 3. Host Process Optimization on `gaia-node`

During the process audit on September 26, 2026, four unneeded default OS daemons were identified and deactivated:

| Daemon | Purpose | Why Disabled |
| :--- | :--- | :--- |
| `ModemManager` | Cellular/GSM modem management | No cellular dongles exist on this server VM. |
| `fwupd` | Physical motherboard/firmware updater | Incompatible/unnecessary inside a hypervisor guest VM. |
| `udisks2` | Desktop USB flash/optical automounter | Headless server with no removable disks. |
| `multipathd` | SAN/Enterprise multipath controller | No redundant Fibre Channel or iSCSI SAN multipath devices. |

**Command to disable**:
```bash
sudo systemctl disable --now ModemManager fwupd udisks2 multipathd
```

---

## 4. Rollback Procedure

If higher concurrency is required (e.g. after scaling the VM to 8 GB or 16 GB RAM), revert using these steps:

### Step 1: Revert `production.toml`
Update [`apps/crawler/config/production.toml`](../apps/crawler/config/production.toml):
```toml
[fetch]
pipeline_limit = 4000
global_fetch_limit = 2000
race_peers = 8
ip_cooldown_max_entries = 75000
conn_limiter_max_entries = 150000

[cache]
peer_cache_max_entries = 100000
announce_cache_max_entries = 50000

[storage]
janitor_interval_secs = 60
janitor_batch_size = 10000
janitor_batch_sleep_ms = 25
```

### Step 2: Deploy and Restart
```bash
# Push config to crawler node
scp apps/crawler/config/production.toml gaia:/home/ubuntu/gaia/apps/crawler/config/production.toml

# Restart crawler container
ssh gaia "docker restart gaia-crawler"
```

---

## 5. Verification and Telemetry Probes

Use these commands to verify steady-state operation:

```bash
# 1. Check crawler memory usage (RSS should stay around ~1.2 GB - 1.4 GB)
ssh gaia "ps -p \$(pgrep -x crawler) -o pid,rss,pmem,vsz,etime; free -m"

# 2. Check kernel OOM logs (should show no new OOM events)
ssh gaia "sudo dmesg -T | grep -iE 'oom|killed process' | tail -n 10"

# 3. Check live ingestion velocity (last 5 minutes)
ssh 192.168.10.10 "docker exec -i gaia-postgres psql -U crawler -d craw -c \"
SELECT 
    count(*) FILTER (WHERE verified_at > now() - interval '5 minutes') as verified_5m,
    count(*) FILTER (WHERE first_seen > now() - interval '5 minutes') as new_5m,
    round(count(*) FILTER (WHERE verified_at > now() - interval '5 minutes') * 12.0, 0) as verified_rate_hr
FROM torrents 
WHERE verified_at > now() - interval '5 minutes';\""
```
