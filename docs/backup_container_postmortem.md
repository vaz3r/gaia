# Backup Container Incident Postmortem & Architecture Fix (Sept 2026)

## Incident Summary
Between September 6, 2026 (~17:07 UTC) and September 9, 2026, automated daily backups of the PostgreSQL database (`gaia-backup` on `workspace-production`) failed to execute. 

Daily cron attempts at 02:00 UTC aborted with:
```
pg_dump: error: could not translate host name "postgres" to address: Name does not resolve
```

## Root Cause Analysis

### 1. Docker Network Isolation Across Custom Bridge
On September 6, `gaia-postgres` was re-created to configure 2 GB shared memory for parallel query workers (commit `c271a81`). 

When `gaia-postgres` was created, Docker attached it solely to the default Docker bridge network (`172.17.0.0/16`). In Docker:
- DNS resolution of container/service names (`postgres`, `gaia-postgres`) **only works within user-defined bridge networks** (such as `gaia-production_default` at `172.18.0.0/16`).
- Containers on the default Docker bridge do not participate in Docker's internal DNS server (`127.0.0.11`).
- Consequently, when `gaia-backup` tried to connect to `DB_HOST=postgres`, Docker's embedded DNS server returned `NXDOMAIN`.

### 2. Lack of Explicit Network Aliases in `docker-compose.yml`
In `deploy/targets/workspace-production/docker-compose.yml`, the `postgres` service definition did not explicitly declare network aliases on the default network, relying on implicit Docker Compose naming which broke when services were individually modified.

### 3. Path Fallback Discrepancy
The volume mount path in `docker-compose.yml` defaulted to `/home/${DEPLOY_USER:-ubuntu}/gaia-data/...`, whereas the deployment target user on `workspace-production` is `core`.

### 4. Brittle Shell Error Handling
In `apps/backup/backup.sh`, `pg_dump ... > "${LOCAL_FILE}"` used shell output redirection. When `pg_dump` failed to connect, the shell still created a 0-byte file in `/tmp`. Because the script had no cleanup trap on error, empty `.dump` files accumulated in `/tmp`.

## Fixes Implemented

1. **Docker Network Aliases:**
   - Updated `deploy/targets/workspace-production/docker-compose.yml` to explicitly assign `postgres` and `gaia-postgres` network aliases to the `postgres` service on the compose network:
     ```yaml
     networks:
       default:
         aliases:
           - postgres
           - gaia-postgres
     ```
   - Standardized volume mount paths to `/home/${DEPLOY_USER:-core}/gaia-data/...`.

2. **Network Reattachment on Live Host:**
   - Connected `gaia-postgres` to `gaia-production_default` with both `postgres` and `gaia-postgres` aliases without restarting PostgreSQL, eliminating downtime.

3. **Script Robustness & Trap Cleanup:**
   - Updated `apps/backup/backup.sh`:
     - Added an `EXIT/INT/TERM` trap to guarantee removal of incomplete or temporary `/tmp/*.dump` files upon script failure.
     - Added a `pg_isready` pre-flight check loop (15 attempts with 2s delay) to verify connectivity before launching `pg_dump`.
     - Switched `pg_dump` to `-f "${LOCAL_FILE}"` to avoid empty file redirection artifacts.

4. **Live Execution & Verification:**
   - Cleared stale 0-byte files from `/tmp`.
   - Executed on-demand backup dump (`5.0 GB` compressed zstd dump) and initiated upload to Google Drive via `rclone`.
