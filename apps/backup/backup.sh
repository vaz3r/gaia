#!/bin/bash
set -eo pipefail

echo "[$(date -Iseconds)] Starting database backup..."

# Use provided remote path or default to gdrive:/ (assuming rclone is configured)
REMOTE="${RCLONE_REMOTE_PATH:-gdrive:/}"
KEEP_COUNT=${BACKUP_KEEP_COUNT:-2}
TIMESTAMP=$(date +"%Y-%m-%dT%H-%M-%S")
FILENAME="craw-backup-${TIMESTAMP}.dump"

echo "[$(date -Iseconds)] Streaming pg_dump to rclone destination: ${REMOTE}/${FILENAME}"

# pg_dump to rclone rcat
PGPASSWORD="${PG_PASSWORD}" pg_dump -h "${DB_HOST}" -p "${DB_PORT}" -U "${POSTGRES_USER}" -Fc "${POSTGRES_DB}" | rclone rcat "${REMOTE}/${FILENAME}"

echo "[$(date -Iseconds)] Upload complete. Pruning old backups (keeping ${KEEP_COUNT})..."

# List files sorted by time (newest first), output JSON, parse with jq, skip KEEP_COUNT, delete the rest
rclone lsjson "${REMOTE}" | jq -r "sort_by(.ModTime) | reverse | .[${KEEP_COUNT}:] | .[]?.Path" | while read -r FILE; do
    if [ -n "$FILE" ]; then
        echo "[$(date -Iseconds)] Deleting old backup: ${FILE}"
        rclone deletefile "${REMOTE}/${FILE}"
    fi
done

echo "[$(date -Iseconds)] Backup process finished successfully!"
