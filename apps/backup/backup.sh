#!/bin/bash
set -eo pipefail

echo "[$(date -Iseconds)] Starting database backup..."

# Use provided remote path or default to gdrive:/ (assuming rclone is configured)
REMOTE="${RCLONE_REMOTE_PATH:-gdrive:/}"
KEEP_COUNT=${BACKUP_KEEP_COUNT:-2}
TIMESTAMP=$(date +"%Y-%m-%dT%H-%M-%S")
FILENAME="craw-backup-${TIMESTAMP}.dump"

# Dump to a local file first to allow rclone to retry on API rate limits
LOCAL_FILE="/tmp/${FILENAME}"

# Ensure temporary file is cleaned up if script errors or exits prematurely
cleanup() {
    local exit_code=$?
    if [ -f "${LOCAL_FILE}" ]; then
        echo "[$(date -Iseconds)] Cleaning up temporary file: ${LOCAL_FILE}"
        rm -f "${LOCAL_FILE}"
    fi
    exit ${exit_code}
}
trap cleanup EXIT INT TERM

echo "[$(date -Iseconds)] Checking database readiness at ${DB_HOST}:${DB_PORT}..."
MAX_ATTEMPTS=15
ATTEMPT=1
until pg_isready -h "${DB_HOST}" -p "${DB_PORT}" -U "${POSTGRES_USER}" -d "${POSTGRES_DB}"; do
    if [ ${ATTEMPT} -ge ${MAX_ATTEMPTS} ]; then
        echo "[$(date -Iseconds)] ERROR: Database at ${DB_HOST}:${DB_PORT} is not ready after ${MAX_ATTEMPTS} attempts. Aborting."
        exit 1
    fi
    echo "[$(date -Iseconds)] Waiting for database... (attempt ${ATTEMPT}/${MAX_ATTEMPTS})"
    sleep 2
    ATTEMPT=$((ATTEMPT + 1))
done

echo "[$(date -Iseconds)] Dumping database to local file: ${LOCAL_FILE}"
PGPASSWORD="${PG_PASSWORD}" pg_dump -h "${DB_HOST}" -p "${DB_PORT}" -U "${POSTGRES_USER}" -Fc -Z zstd -f "${LOCAL_FILE}" "${POSTGRES_DB}"

echo "[$(date -Iseconds)] Uploading to Google Drive..."
rclone copy "${LOCAL_FILE}" "${REMOTE}/" --drive-chunk-size 128M --tpslimit 2 --retries 5

echo "[$(date -Iseconds)] Cleaning up local file..."
rm "${LOCAL_FILE}"

echo "[$(date -Iseconds)] Upload complete. Pruning old backups (keeping ${KEEP_COUNT})..."

# List files sorted by time (newest first), output JSON, parse with jq, skip KEEP_COUNT, delete the rest
rclone lsjson "${REMOTE}" | jq -r "sort_by(.ModTime) | reverse | .[${KEEP_COUNT}:] | .[]?.Path" | while read -r FILE; do
    if [ -n "$FILE" ]; then
        echo "[$(date -Iseconds)] Deleting old backup: ${FILE}"
        rclone deletefile "${REMOTE}/${FILE}"
    fi
done

echo "[$(date -Iseconds)] Backup process finished successfully!"
