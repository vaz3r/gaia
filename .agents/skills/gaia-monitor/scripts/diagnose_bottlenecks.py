#!/usr/bin/env python3
"""
diagnose_bottlenecks.py — Automated Root-Cause Diagnostic & Anomaly Detection Engine for GAIA.
Cross-correlates telemetry from all subsystem probes and generates actionable diagnostic findings.
"""
import sys
import json
import subprocess
import os
from concurrent.futures import ThreadPoolExecutor

def run_probe(cmd):
    try:
        res = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=120)
        out = res.stdout.strip()
        if not out:
            return {}
        # Parse JSON from output
        return json.loads(out)
    except Exception as e:
        sys.stderr.write(f"Warning: probe '{cmd}' encountered error: {e}\n")
        return {}

def diagnose():
    script_dir = os.path.dirname(os.path.realpath(__file__))
    
    # Run all probes concurrently
    probes = {
        "crawler": f"{script_dir}/probe_crawler_ingestion.sh --json",
        "classifier": f"{script_dir}/probe_classifier_ml.sh --json",
        "database": f"{script_dir}/probe_database_storage.sh --json",
        "search_sync": f"{script_dir}/probe_search_sync.sh --json",
        "containers": f"{script_dir}/probe_containers_fleet.sh --json",
        "hosts": f"{script_dir}/probe_hosts_network.sh --json"
    }

    results = {}
    with ThreadPoolExecutor(max_workers=6) as executor:
        future_to_probe = {executor.submit(run_probe, cmd): name for name, cmd in probes.items()}
        for future in future_to_probe:
            name = future_to_probe[future]
            try:
                results[name] = future.result()
            except Exception as e:
                sys.stderr.write(f"Error reading result for {name}: {e}\n")
                results[name] = {}

    crawler_data = results.get("crawler", {})
    classifier_data = results.get("classifier", {})
    db_data = results.get("database", {})
    sync_data = results.get("search_sync", {})
    containers_data = results.get("containers", {})
    hosts_data = results.get("hosts", {})

    findings = []
    overall_status = "HEALTHY" # HEALTHY (🟢), DEGRADED (🟡), CRITICAL (🔴)

    def set_status(severity):
        nonlocal overall_status
        if severity == "CRITICAL":
            overall_status = "CRITICAL"
        elif severity == "WARNING" and overall_status != "CRITICAL":
            overall_status = "DEGRADED"

    # ── 1. Database Buffer Cache & Contention ──
    pg = db_data.get("postgresql", {})
    cache_hit = pg.get("buffer_cache_hit_ratio_pct", 100.0)
    if cache_hit > 0.0 and cache_hit < 95.0:
        sev = "CRITICAL" if cache_hit < 80.0 else "WARNING"
        set_status(sev)
        findings.append({
            "subsystem": "Storage / Database",
            "component": "PostgreSQL (craw)",
            "severity": sev,
            "title": f"Sub-Optimal Buffer Cache Hit Ratio ({cache_hit}%)",
            "evidence": f"PostgreSQL buffer cache hit ratio is {cache_hit}% (SLA >= 99.0%). Over 40% of disk pages are read from NVMe disk instead of memory cache.",
            "root_cause": "Database size (74 GB) exceeds configured PostgreSQL shared_buffers memory allocation, causing frequent NVMe page eviction during heavy index scans.",
            "remediation": "Increase `shared_buffers` in `deploy/postgres/postgresql.workspace-production.conf` and optimize top relation queries on `fetch_peer_outcomes` (28 GB) and `infohash_sightings` (13 GB)."
        })

    slow_queries = pg.get("slow_queries_active", 0)
    if slow_queries > 0:
        set_status("WARNING")
        findings.append({
            "subsystem": "Storage / Database",
            "component": "PostgreSQL (craw)",
            "severity": "WARNING",
            "title": f"Active Slow Queries Detected ({slow_queries})",
            "evidence": f"{slow_queries} query(s) currently executing longer than 5 seconds.",
            "root_cause": "Long-running aggregations or unindexed lookups holding locks on torrent or metrics tables.",
            "remediation": "Inspect executing queries with: `docker exec gaia-postgres psql -U crawler -d craw -c \"SELECT pid, now() - query_start AS duration, query FROM pg_stat_activity WHERE state != 'idle' ORDER BY duration DESC;\"`"
        })

    # ── 2. Classifier Timing & DB Lock Contention ──
    batch = classifier_data.get("classifier_batch", {})
    db_ratio = batch.get("db_time_ratio_pct", 0.0)
    infer_ratio = batch.get("infer_time_ratio_pct", 0.0)
    db_sec = batch.get("timing_db_sec", 0.0)
    infer_sec = batch.get("timing_infer_sec", 0.0)

    if db_ratio > 40.0 and db_sec > 5.0:
        set_status("WARNING")
        findings.append({
            "subsystem": "Classification Pipeline",
            "component": "gaia-classifier",
            "severity": "WARNING",
            "title": f"Database Commit Contention ({db_ratio:.1f}% of batch time)",
            "evidence": f"Batch write step took {db_sec}s while model inference took {infer_sec}s. Total rate: {batch.get('rate_rec_sec')} rec/s.",
            "root_cause": "Row-level lock contention on `torrents` table during concurrent crawler ingestion and classifier batch updates.",
            "remediation": "Tune `WORKER_BATCH_SIZE: 500` (down from 2000) in `deploy/targets/workspace-production/docker-compose.yml` to commit updates in smaller, non-blocking transactions."
        })
    elif infer_ratio > 70.0 and infer_sec > 60.0:
        set_status("WARNING")
        findings.append({
            "subsystem": "Classification Pipeline",
            "component": "gaia-classifier",
            "severity": "WARNING",
            "title": "CPU-Bound Model Inference Latency",
            "evidence": f"Inference took {infer_sec}s per batch ({infer_ratio:.1f}% of duration).",
            "root_cause": "High CPU utilization on host slowing LightGBM CPU multiclass inference.",
            "remediation": "Increase `WORKER_POLL_INTERVAL: 30` to throttle batches or allocate dedicated CPU affinity."
        })

    # ── 3. Host Resources & Container Fleet ──
    containers = containers_data.get("containers", [])
    for c in containers:
        name = c.get("name", "")
        restarts = c.get("restarts", 0)
        state = c.get("state", "")
        
        if state != "running":
            sev = "WARNING" if "portainer" in name else "CRITICAL"
            set_status(sev)
            findings.append({
                "subsystem": "Container Fleet",
                "component": f"{c.get('target')}/{name}",
                "severity": sev,
                "title": f"Container Is Not Running (State: {state})",
                "evidence": f"Container `{name}` on target `{c.get('target')}` is in state `{state}`.",
                "root_cause": "Container crashed, completed exit, or was stopped.",
                "remediation": f"Inspect logs: `docker logs {name} --tail 50` and restart if needed."
            })
        
        if restarts >= 3:
            set_status("WARNING")
            findings.append({
                "subsystem": "Container Fleet",
                "component": f"{c.get('target')}/{name}",
                "severity": "WARNING",
                "title": f"High Container Restart Count ({restarts} restarts)",
                "evidence": f"Container `{name}` on target `{c.get('target')}` has restarted {restarts} times.",
                "root_cause": "Process crash, OOM kill, or network handshake failure triggering Docker restart policy.",
                "remediation": f"Review crash logs: `docker logs {name} --tail 100`"
            })

    # Host Resources
    hosts = hosts_data.get("hosts", [])
    for h in hosts:
        target = h.get("target", "")
        ram = h.get("ram_mb", {})
        load = h.get("load_avg", {})
        avail_mb = ram.get("available", 9999)
        load15 = load.get("15m", 0.0)

        if avail_mb < 500:
            set_status("WARNING")
            findings.append({
                "subsystem": "Host Infrastructure",
                "component": f"{target} ({h.get('hostname')})",
                "severity": "WARNING",
                "title": f"Low Available Memory ({avail_mb} MB available)",
                "evidence": f"Host `{target}` has only {avail_mb} MB available memory remaining.",
                "root_cause": "Memory pressure from high-concurrency worker threads or crawler buffer caches.",
                "remediation": "Clean up cached buffers or tune memory limits in docker-compose.yml."
            })

        if load15 > 12.0:
            set_status("WARNING")
            findings.append({
                "subsystem": "Host Infrastructure",
                "component": f"{target} ({h.get('hostname')})",
                "severity": "WARNING",
                "title": f"Elevated CPU Load Average ({load15:.2f})",
                "evidence": f"Host `{target}` 15-minute load average is {load15:.2f}.",
                "root_cause": "High concurrent database queries and background ML inference routines.",
                "remediation": "Run `top -b -n 1` or inspect container CPU percentages."
            })

        # Check root disk space utilization
        disk_str = h.get("disk_root", "")
        if "%" in disk_str:
            try:
                pct = int([x for x in disk_str.split() if "%" in x][0].replace("%", ""))
                if pct >= 90:
                    set_status("CRITICAL")
                    findings.append({
                        "subsystem": "Host Storage",
                        "component": f"{target} ({h.get('hostname')})",
                        "severity": "CRITICAL",
                        "title": f"Root Disk Space Critical ({pct}% used)",
                        "evidence": f"Root filesystem on `{target}` is {pct}% full: {disk_str}.",
                        "root_cause": "Unchecked accumulation of log files, temporary dumps, or bloat.",
                        "remediation": "Clean stale logs and check disk usage: `sudo du -sh /home/core/gaia-data/*`."
                    })
                elif pct >= 80:
                    set_status("WARNING")
                    findings.append({
                        "subsystem": "Host Storage",
                        "component": f"{target} ({h.get('hostname')})",
                        "severity": "WARNING",
                        "title": f"Root Disk Space Elevated ({pct}% used)",
                        "evidence": f"Root filesystem on `{target}` is {pct}% full: {disk_str}.",
                        "root_cause": "High disk utilization approaching capacity threshold.",
                        "remediation": "Audit and prune old logs and container layers."
                    })
            except Exception:
                pass

    # ── Dashboard HTTP Reachability & Service Health ──
    try:
        dash_res = subprocess.run(
            "curl -s -w '%{http_code}:%{time_total}' -m 5 -o /dev/null http://workspace-production:3000/",
            shell=True, capture_output=True, text=True, timeout=8
        )
        dash_out = dash_res.stdout.strip()
        if dash_out:
            code, latency = dash_out.split(":")
            latency_f = float(latency)
            if code != "200":
                set_status("CRITICAL")
                findings.append({
                    "subsystem": "Dashboard Service",
                    "component": "gaia-dashboard",
                    "severity": "CRITICAL",
                    "title": f"Dashboard HTTP Endpoint Error (HTTP {code})",
                    "evidence": f"HTTP GET http://workspace-production:3000/ returned status {code}.",
                    "root_cause": "Dashboard application error or Kestrel backend exception.",
                    "remediation": "Inspect container logs: `docker logs gaia-dashboard --tail 50`."
                })
            elif latency_f > 3.0:
                set_status("WARNING")
                findings.append({
                    "subsystem": "Dashboard Service",
                    "component": "gaia-dashboard",
                    "severity": "WARNING",
                    "title": f"Slow Dashboard Response ({latency_f:.2f}s latency)",
                    "evidence": f"HTTP GET http://workspace-production:3000/ took {latency_f:.2f}s to respond.",
                    "root_cause": "Host I/O wait, CPU throttling, or slow database queries.",
                    "remediation": "Check database load and host CPU/disk utilization."
                })
    except Exception as e:
        set_status("CRITICAL")
        findings.append({
            "subsystem": "Dashboard Service",
            "component": "gaia-dashboard",
            "severity": "CRITICAL",
            "title": "Dashboard HTTP Endpoint Unreachable",
            "evidence": f"Failed to connect to http://workspace-production:3000/ within 5s: {e}",
            "root_cause": "Container unmapped, network/Tailscale routing failure, or process hang.",
            "remediation": "Check container state (`docker ps | grep dashboard`) and Tailscale status (`tailscale status`)."
        })

    # ── 4. Ingestion & Crawler Dynamics ──
    ingest_anomaly = crawler_data.get("ingestion_anomaly", "NONE")
    rate_delta = crawler_data.get("rate_delta_pct", 0.0)
    v1h = crawler_data.get("verified_last_1h", 0)
    ma = crawler_data.get("hourly_moving_avg", 0)
    if ingest_anomaly in ("SEVERE_DROP", "CRITICAL_ZERO_INGESTION"):
        sev = "CRITICAL" if ingest_anomaly == "CRITICAL_ZERO_INGESTION" else "WARNING"
        set_status(sev)
        findings.append({
            "subsystem": "Crawler & Ingestion",
            "component": "gaia-crawler",
            "severity": sev,
            "title": f"Ingestion Rate Drop Detected ({rate_delta:.1f}% vs 24h baseline)",
            "evidence": f"Current hour rate is {v1h} torrents/hr compared to the 24h moving average of {ma} torrents/hr.",
            "root_cause": "External BitTorrent DHT network throttling, peer unreachability, or UDP socket saturation.",
            "remediation": "Verify crawler container logs: `ssh gaia \"docker logs gaia-crawler --tail 50\"`"
        })

    # ── 5. Search Sync Replication ──
    sync = sync_data.get("sync_worker", {})
    rebuild_age = sync.get("last_rebuild_age_minutes", 0)
    lag = sync_data.get("meilisearch", {}).get("replication_lag_documents", 0)
    if rebuild_age > 150:
        set_status("WARNING")
        findings.append({
            "subsystem": "Search & Portal",
            "component": "gaia-portal-sync",
            "severity": "WARNING",
            "title": f"Search Replication Rebuild Overdue ({rebuild_age} mins ago)",
            "evidence": f"Last periodic rebuild was {rebuild_age} minutes ago (SLA <= 120m). Replication lag is {lag:,} documents.",
            "root_cause": "Sync worker did not trigger or is stuck indexing.",
            "remediation": "Trigger resync on portal: `(source deploy/targets/gaia-portal/.env && sshpass -p \"$DEPLOY_PASSWORD\" ssh $DEPLOY_USER@$DEPLOY_HOST \"docker restart gaia-portal-sync\")`"
        })

    # ── 6. OpSec Compliance ──
    opsec = hosts_data.get("opsec_compliance", {})
    if "LEAK_DETECTED" in opsec.get("egress_killswitch", ""):
        findings.append({
            "subsystem": "OpSec & Security",
            "component": "Kernel Killswitch",
            "severity": "WARNING",
            "title": "Outbound Port 80 Egress Killswitch Inactive",
            "evidence": "Direct HTTP connection to public IP succeeded without routing through the Gateway proxy.",
            "root_cause": "iptables egress killswitch rules not applied on current host.",
            "remediation": "Apply homelab killswitch: `./deploy/scripts/setup-homelab-opsec.sh workspace-production`"
        })

    tunnels = hosts_data.get("network_tunnels", {})
    wg_age = tunnels.get("core_to_gateway_wg_age_sec", 0)
    if wg_age > 180:
        sev = "CRITICAL" if wg_age > 300 else "WARNING"
        set_status(sev)
        findings.append({
            "subsystem": "Network & Tunnels",
            "component": "WireGuard Tunnel",
            "severity": sev,
            "title": f"Stale WireGuard Handshake ({wg_age}s ago)",
            "evidence": f"Latest handshake to gateway is {wg_age}s old (SLA <= 180s).",
            "root_cause": "wstunnel connection dropped or Gateway Nginx restarted.",
            "remediation": "Restart tunnel client: `docker restart gaia-wstunnel-client gaia-wireguard-client`"
        })

    result = {
        "overall_status": overall_status,
        "total_findings": len(findings),
        "findings": findings,
        "raw_telemetry": {
            "crawler": crawler_data,
            "classifier": classifier_data,
            "database": db_data,
            "search_sync": sync_data,
            "containers": containers_data,
            "hosts": hosts_data
        }
    }
    print(json.dumps(result, indent=2))

if __name__ == "__main__":
    diagnose()
