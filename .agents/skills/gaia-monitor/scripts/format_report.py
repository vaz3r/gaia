#!/usr/bin/env python3
"""
format_report.py — Generates an Executive GAIA Health & Performance Report in GitHub-flavored Markdown.
Consumes diagnostic JSON from diagnose_bottlenecks.py.
"""
import sys
import json
import datetime

def format_markdown(data):
    overall_status = data.get("overall_status", "UNKNOWN")
    raw = data.get("raw_telemetry", {})
    crawler = raw.get("crawler", {})
    classifier = raw.get("classifier", {})
    db = raw.get("database", {})
    sync = raw.get("search_sync", {})
    containers = raw.get("containers", {}).get("containers", [])
    hosts = raw.get("hosts", {}).get("hosts", [])
    tunnels = raw.get("hosts", {}).get("network_tunnels", {})
    opsec = raw.get("hosts", {}).get("opsec_compliance", {})
    findings = data.get("findings", [])

    # Status Badge
    badge_map = {
        "HEALTHY": "🟢 ALL SYSTEMS OPERATIONAL",
        "DEGRADED": "🟡 DEGRADED PERFORMANCE / BOTTLENECK DETECTED",
        "CRITICAL": "🔴 CRITICAL ALERT / SYSTEM OUTAGE"
    }
    status_badge = badge_map.get(overall_status, "⚪ UNKNOWN STATUS")

    now_utc = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")

    md = []
    md.append(f"# 🛡️ GAIA Ecosystem — Health & Performance Report")
    md.append(f"> **Generated at**: `{now_utc}`  ")
    md.append(f"> **Fleet Status**: **{status_badge}**  ")
    md.append("")

    # Executive Summary Badges
    v1h = crawler.get("verified_last_1h", 0)
    v24h = crawler.get("verified_last_24h", 0)
    unclass = classifier.get("unclassified_backlog", 0)
    total_torrents = crawler.get("total_torrents", 0)
    running_conts = sum(1 for c in containers if c.get("state") == "running")
    total_conts = len(containers)
    cache_hit = db.get("postgresql", {}).get("buffer_cache_hit_ratio_pct", 0.0)
    sync_lag = sync.get("meilisearch", {}).get("replication_lag_documents", 0)

    md.append("### Executive KPI Snapshot")
    md.append(f"| Metric | Current Value | SLA / Target | Status |")
    md.append(f"| :--- | :--- | :--- | :--- |")
    md.append(f"| **Hourly Ingestion Rate** | **{v1h:,} torrents/hr** ({crawler.get('rate_delta_pct', 0.0):+.1f}% vs 24h avg) | >= 10,000/hr | {'🟢 Normal' if v1h >= 10000 else '🟡 Low' if v1h > 0 else '🔴 Stalled'} |")
    md.append(f"| **Active Containers** | **{running_conts} / {total_conts} containers online** | 100% online | {'🟢 Optimal' if running_conts == total_conts else '🟡 Degraded'} |")
    md.append(f"| **Unclassified Backlog** | **{unclass:,} records** | <= 25,000 | {'🟢 Drained' if unclass <= 25000 else '🟡 Backlog'} |")
    md.append(f"| **PostgreSQL Cache Hit** | **{cache_hit:.2f}%** | >= 99.00% | {'🟢 Optimal' if cache_hit >= 99.0 else '🔴 Disk-Bound'} |")
    md.append(f"| **Search Replication Lag** | **{sync_lag:,} documents** | <= 10,000 | {'🟢 Synchronized' if sync_lag <= 10000 else '🟡 Replicating'} |")
    md.append(f"| **WireGuard Tunnel Latency**| **{tunnels.get('core_tunnel_ping_latency', 'N/A')}** ({tunnels.get('core_to_gateway_wg_age_sec', 0)}s handshake) | < 50ms / <= 180s | {'🟢 Stable' if tunnels.get('core_to_gateway_wg_age_sec', 0) <= 180 else '🔴 Severed'} |")
    md.append("")
    md.append("---")
    md.append("")

    # 1. Ingestion & Crawler Dynamics
    md.append("## 1. Ingestion & Crawler Dynamics")
    md.append(f"- **Total Database Torrents**: `{total_torrents:,}`")
    md.append(f"- **24h Rolling Volume**: `{v24h:,}` torrents (Hourly Moving Avg: `{crawler.get('hourly_moving_avg', 0):,}` torrents/hr)")
    md.append(f"- **DHT Swarm Routing Table**: `{crawler.get('routing_table_len', 0):,}` active nodes")
    md.append(f"- **BEP-9 Wire Metadata Success**: `{crawler.get('bep9_wire_success_rate', 0.0)}%` ({crawler.get('unique_infohashes', 0):,} hashes / {crawler.get('fetch_attempts', 0):,} attempts)")
    md.append(f"- **DHT Inbound Queries**: `get_peers={crawler.get('inbound_get_peers', 0):,}`, `announce={crawler.get('inbound_announce_peer', 0):,}`")
    md.append("")
    md.append("#### Hourly Ingestion History (Last 12 Hours)")
    md.append("| Hour Slot (UTC) | Verified Torrents | Throughput (Avg/Min) | Trend vs Baseline |")
    md.append("| :--- | :--- | :--- | :--- |")
    history = crawler.get("hourly_history", [])
    ma = crawler.get("hourly_moving_avg", 1) or 1
    for row in history[:12]:
        cnt = row.get("verified", 0)
        diff = ((cnt - ma) / ma) * 100.0
        icon = "🟢" if diff >= 0 else "🟡" if diff >= -30 else "🔴"
        md.append(f"| `{row.get('hour')}` | **{cnt:,}** | {row.get('avg_per_min', 0.0):.1f} / min | {icon} {diff:+.1f}% |")
    md.append("")
    md.append("---")
    md.append("")

    # 2. Classification & Machine Learning
    batch = classifier.get("classifier_batch", {})
    ml = classifier.get("ml_intelligence", {})
    md.append("## 2. Classification & Machine Learning Intelligence")
    md.append(f"- **Classified Torrents**: `{classifier.get('classified_total', 0):,}` | **Quality Scored**: `{classifier.get('scored_total', 0):,}`")
    md.append(f"- **Latest Batch**: `{batch.get('size', 0)}` records | **Processing Velocity**: `{batch.get('rate_rec_sec', 0)} rec/s`")
    md.append(f"- **Batch Verdict Breakdown**: Accepted `{batch.get('accepted_pct', 0.0)}%` | Flagged `{batch.get('flagged_pct', 0.0)}%`")
    md.append("")
    md.append("#### Batch Execution Timing Breakdown")
    md.append("| Pipeline Phase | Elapsed Time | Proportion of Batch | Bottleneck Status |")
    md.append("| :--- | :--- | :--- | :--- |")
    md.append(f"| **PostgreSQL Record Fetch** | `{batch.get('timing_fetch_sec', 0.0):.2f}s` | — | {'🟢 Fast' if batch.get('timing_fetch_sec', 0.0) < 10 else '🟡 Slow'} |")
    md.append(f"| **LightGBM Model Inference** | `{batch.get('timing_infer_sec', 0.0):.2f}s` | `{batch.get('infer_time_ratio_pct', 0.0):.1f}%` | {'🟢 Normal' if batch.get('infer_time_ratio_pct', 0.0) < 50 else '🟡 Elevated'} |")
    md.append(f"| **PostgreSQL Batch Commit** | `{batch.get('timing_db_sec', 0.0):.2f}s` | `{batch.get('db_time_ratio_pct', 0.0):.1f}%` | {'🟢 Normal' if batch.get('db_time_ratio_pct', 0.0) < 40 else '🔴 Lock Contention'} |")
    md.append("")
    md.append(f"- **ML Unsupervised Anomaly Engine**: Latest Score `{ml.get('anomaly_score', 0.0):.3f}` (Severity: `{ml.get('anomaly_severity', 'NORMAL')}`) on window `{ml.get('anomaly_window', 'N/A')}`")
    md.append(f"- **Quality Scoring Velocity**: Scored `{ml.get('recent_scored_batch_total', 0):,}` | Refreshed `{ml.get('recent_refreshed_batch_total', 0):,}`")
    md.append("")
    md.append("---")
    md.append("")

    # 3. Storage, Database & Search Replication
    pg = db.get("postgresql", {})
    redis_data = db.get("redis", {})
    backup_data = db.get("backup", {})
    sync_worker = sync.get("sync_worker", {})
    meili = sync.get("meilisearch", {})

    md.append("## 3. Storage, Database & Search Replication")
    md.append(f"- **PostgreSQL Master Size**: `{pg.get('database_size', 'N/A')}` | **Buffer Cache Hit Ratio**: `{pg.get('buffer_cache_hit_ratio_pct', 0.0):.2f}%`")
    md.append(f"- **Connections**: `{pg.get('active_connections', 0)}` Active | `{pg.get('idle_connections', 0)}` Idle | `{pg.get('idle_in_transaction', 0)}` In-Transaction | `{pg.get('total_connections', 0)} / {pg.get('max_connections', 100)}` Pool")
    md.append(f"- **Contention Indicators**: `{pg.get('waiting_locks', 0)}` Waiting Locks | `{pg.get('slow_queries_active', 0)}` Slow Queries (>5s)")
    md.append(f"- **Redis In-Memory Cache**: Used `{redis_data.get('used_memory', 'N/A')} / {redis_data.get('max_memory', 'N/A')}` | Hit Rate: `{redis_data.get('hit_rate_pct', 0.0)}%` | Evictions: `{redis_data.get('evicted_keys', 0)}`")
    md.append(f"- **Meilisearch Search Engine**: `{meili.get('indexed_documents', 0):,}` indexed documents (`{meili.get('status', 'N/A')}`)")
    md.append(f"- **CDC Replication Status**: `{sync_worker.get('status', 'N/A')}` | Last Rebuild: `{sync_worker.get('last_rebuild_age_minutes', 0)}` mins ago | Streaming: `{sync_worker.get('streaming_rate_docs_per_sec', 0)} docs/s`")
    md.append(f"- **Disaster Recovery Backup**: `{backup_data.get('status', 'N/A')}` (Last dump: `{backup_data.get('last_run_timestamp', 'N/A')}`)")
    md.append("")
    md.append("#### Top 5 Largest PostgreSQL Relations")
    md.append("| Table Name | Disk Size | Approximate Rows |")
    md.append("| :--- | :--- | :--- |")
    for t in pg.get("top_tables", []):
        md.append(f"| `{t.get('table')}` | **{t.get('size')}** | {t.get('rows', 0):,} |")
    md.append("")
    md.append("---")
    md.append("")

    # 4. Container Fleet Status
    md.append("## 4. Container Fleet Status Matrix")
    md.append("| Target Node | Container Name | State | Health Status | Restarts | CPU % | Memory Footprint |")
    md.append("| :--- | :--- | :--- | :--- | :--- | :--- | :--- |")
    for c in containers:
        h_icon = "🟢" if c.get("health") in ("healthy", "none") else "🔴"
        s_icon = "🟢" if c.get("state") == "running" else "🔴"
        r_str = f"**{c.get('restarts')}**" if c.get('restarts', 0) > 0 else "0"
        md.append(f"| `{c.get('target')}` | `{c.get('name')}` | {s_icon} `{c.get('state')}` | {h_icon} `{c.get('health')}` | {r_str} | `{c.get('cpu_pct')}` | `{c.get('mem_usage')}` |")
    md.append("")
    md.append("---")
    md.append("")

    # 5. Host Infrastructure & OpSec
    md.append("## 5. Host Infrastructure & OpSec Compliance")
    md.append("| Target | Hostname | Uptime | Load Avg (15m) | RAM Available | Root Disk Space |")
    md.append("| :--- | :--- | :--- | :--- | :--- | :--- |")
    for h in hosts:
        ram = h.get("ram_mb", {})
        load = h.get("load_avg", {})
        l15 = load.get("15m", 0.0)
        l_icon = "🟢" if l15 <= 12.0 else "🟡" if l15 <= 16.0 else "🔴"
        md.append(f"| `{h.get('target')}` | `{h.get('hostname')}` | {h.get('uptime')} | {l_icon} `{l15:.2f}` | `{ram.get('available', 0)}MB / {ram.get('total', 0)}MB` | `{h.get('disk_root')}` |")
    md.append("")
    md.append("#### Encrypted Tunnels & OpSec Invariants")
    md.append(f"- **Homelab -> Gateway WireGuard Handshake**: `{tunnels.get('core_to_gateway_wg_age_sec', 0)}s` ago (Target <= 180s)")
    md.append(f"- **Homelab -> Gateway Tunnel Latency**: `{tunnels.get('core_tunnel_ping_latency', 'N/A')}` ({tunnels.get('core_tunnel_packet_loss', 'N/A')})")
    md.append(f"- **Portal -> Gateway Tunnel Latency**: `{tunnels.get('portal_tunnel_ping_latency', 'N/A')}`")
    md.append(f"- **Encrypted DNS-over-TLS (DoT)**: `{opsec.get('dns_over_tls', 'N/A')}`")
    md.append(f"- **Kernel Egress Killswitch**: `{opsec.get('egress_killswitch', 'N/A')}`")
    md.append("")
    md.append("---")
    md.append("")

    # 6. Bottlenecks & Diagnostics
    md.append("## 6. Bottleneck & Anomaly Diagnostic Matrix")
    if not findings:
        md.append("✅ **No active bottlenecks or anomalies detected. All subsystems operating within SLA thresholds.**")
    else:
        for idx, f in enumerate(findings, 1):
            sev_icon = "🔴" if f.get("severity") == "CRITICAL" else "🟡"
            md.append(f"### {idx}. {sev_icon} [{f.get('severity')}] {f.get('title')}")
            md.append(f"- **Subsystem / Component**: `{f.get('subsystem')}` / `{f.get('component')}`")
            md.append(f"- **Evidence**: {f.get('evidence')}")
            md.append(f"- **Root Cause**: {f.get('root_cause')}")
            md.append(f"- **Remediation Action**: `{f.get('remediation')}`")
            md.append("")

    md.append("---")
    md.append("")

    # 7. Actionable Runbook Commands
    md.append("## 7. Actionable Remediation Runbook")
    md.append("Run the following verified commands to remediate the detected bottlenecks:")
    md.append("```bash")
    md.append("# 1. If PostgreSQL Buffer Cache is low (< 95%):")
    md.append("# Check running locks and long queries")
    md.append("docker exec gaia-postgres psql -U crawler -d craw -c \"SELECT pid, now() - query_start AS duration, query FROM pg_stat_activity WHERE state != 'idle' ORDER BY duration DESC;\"")
    md.append("")
    md.append("# 2. If Classifier Database Write contention is high (> 50%):")
    md.append("# Reduce batch size to 500 to shorten row-lock duration")
    md.append("# Edit WORKER_BATCH_SIZE: 500 in deploy/targets/workspace-production/docker-compose.yml")
    md.append("docker restart gaia-classifier")
    md.append("")
    md.append("# 3. If OpSec Killswitch or DNS-over-TLS is leaking:")
    md.append("./deploy/scripts/setup-homelab-opsec.sh workspace-production")
    md.append("./deploy/scripts/verify-opsec.sh workspace-production")
    md.append("")
    md.append("# 4. If Search Replication is overdue (> 150m):")
    md.append("(source deploy/targets/gaia-portal/.env && sshpass -p \"$DEPLOY_PASSWORD\" ssh $DEPLOY_USER@$DEPLOY_HOST \"docker restart gaia-portal-sync\")")
    md.append("```")
    md.append("")

    return "\n".join(md)

if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--file":
        with open(sys.argv[2]) as f:
            data = json.load(f)
    else:
        data = json.load(sys.stdin)
    print(format_markdown(data))
