# GAIA Operational Troubleshooting & Optimization Runbooks

This guide provides targeted diagnostic workflows and exact recovery commands for addressing issues, bugs, and bottlenecks identified by the `gaia-monitor` skill.

---

## 1. High Host Load & CPU Bottlenecks

### Symptoms:
- Host Load Average > 12.0 or 16.0
- Classifier inference latency > 60s
- Slow PostgreSQL query execution

### Diagnostic Steps:
```bash
# Check top CPU-consuming containers
docker stats --no-stream --format "table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}" | sort -k 2 -h -r

# Check host process tree
ps aux --sort=-%cpu | head -n 15
```

### Remediation:
1. If `gaia-classifier` is consuming excessive CPU:
   Adjust worker poll interval and batch size in `deploy/targets/workspace-production/docker-compose.yml`:
   ```bash
   # Increase poll interval to let CPU cool between batches
   # WORKER_POLL_INTERVAL=30
   docker restart gaia-classifier
   ```
2. If `gaia-postgres` is CPU-bound:
   Inspect active query locks:
   ```bash
   docker exec gaia-postgres psql -U crawler -d craw -c "
   SELECT pid, now() - query_start AS duration, query, state 
   FROM pg_stat_activity 
   WHERE state != 'idle' 
   ORDER BY duration DESC LIMIT 5;"
   ```

---

## 2. Ingestion Drop / Crawler Stagnation

### Symptoms:
- Hourly verified torrents drop > 30% below 24h moving average
- Inbound DHT packet rate drops near 0

### Diagnostic Steps:
```bash
# 1. Check crawler container status on gaia-node
ssh gaia "docker ps --filter name=gaia-crawler"

# 2. Check recent crawler log errors
ssh gaia "docker logs gaia-crawler --tail 50" | grep -iE "error|panic|drop|timeout"

# 3. Check crawler network socket and UDP listener
ssh gaia "ss -uln | grep 688"
```

### Remediation:
1. If crawler container crashed or is hung:
   ```bash
   ssh gaia "docker restart gaia-crawler"
   ```
2. If outbound WireGuard link to PostgreSQL is severed:
   ```bash
   # Check wg0 handshake on gaia-node
   ssh gaia "wg show wg0"
   # Restart tunnel client if handshake > 180s
   ssh gaia "docker restart gaia-wireguard-client gaia-wstunnel-client"
   ```

---

## 3. Database Write Contention in Classifier (`db Xs` bottleneck)

### Symptoms:
- Classifier batch log shows `Timing: db 57.84s` while `infer` is only `3.25s`
- UNCLASSIFIED backlog increases

### Diagnostic Steps:
```bash
# Check PostgreSQL locks on torrents table
docker exec gaia-postgres psql -U crawler -d craw -c "
SELECT relation::regclass, mode, granted, pid 
FROM pg_locks 
WHERE relation = 'torrents'::regclass;"
```

### Remediation:
1. Decrease batch size from 2000 to 500 in `workspace-production/docker-compose.yml` to minimize row-level lock duration:
   `WORKER_BATCH_SIZE: 500`
2. Ensure classifier updates use batched `UPDATE ... FROM (VALUES ...)` without long-running transactions.

---

## 4. Search Replication Lag (`Meilisearch Sync Lag`)

### Symptoms:
- Indexed documents in Meilisearch lag PostgreSQL by > 10,000 records
- Last periodic rebuild is older than 150 minutes

### Diagnostic Steps:
```bash
# Check gaia-portal-sync logs on gaia-portal
(source deploy/targets/gaia-portal/.env && sshpass -p "$DEPLOY_PASSWORD" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no "$DEPLOY_USER@$DEPLOY_HOST" "docker logs gaia-portal-sync --tail 50")

# Check Meilisearch task queue
(source deploy/targets/gaia-portal/.env && sshpass -p "$DEPLOY_PASSWORD" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no "$DEPLOY_USER@$DEPLOY_HOST" "curl -s -H 'Authorization: Bearer $MEILI_MASTER_KEY' http://127.0.0.1:7700/tasks?limit=5")
```

### Remediation:
1. If sync worker crashed:
   ```bash
   (source deploy/targets/gaia-portal/.env && sshpass -p "$DEPLOY_PASSWORD" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no "$DEPLOY_USER@$DEPLOY_HOST" "docker restart gaia-portal-sync")
   ```
2. Trigger immediate on-demand rebuild if index is corrupted:
   ```bash
   (source deploy/targets/gaia-portal/.env && sshpass -p "$DEPLOY_PASSWORD" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password -o PubkeyAuthentication=no "$DEPLOY_USER@$DEPLOY_HOST" "docker restart gaia-portal-sync")
   ```

---

## 5. WireGuard / wstunnel Link Severed

### Symptoms:
- WireGuard handshake age > 300 seconds
- Cannot ping `10.99.0.1`

### Diagnostic Steps:
```bash
# Check wstunnel client logs
docker logs gaia-wstunnel-client --tail 30

# Check WireGuard interface on host
wg show wg0
```

### Remediation:
1. Restart tunnel client containers in order:
   ```bash
   docker restart gaia-wstunnel-client
   sleep 3
   docker restart gaia-wireguard-client
   ```
2. Test tunnel reachability:
   ```bash
   ping -c 3 10.99.0.1
   ```

---

## 6. Disk Exhaustion (> 85% Used)

### Symptoms:
- Root filesystem `/` or `/home/core/gaia-data` > 85% full

### Remediation:
```bash
# 1. Clean dangling Docker images and build caches
docker system prune -f

# 2. Prune old crawler JSONL logs older than 7 days
find /mnt/gaia/logs/crawler/ -name "*.jsonl" -mtime +7 -delete

# 3. Check largest directories in data storage
du -sh /home/core/gaia-data/* | sort -h -r | head -n 10
```
