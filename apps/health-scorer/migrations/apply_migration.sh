#!/usr/bin/env bash
# apply_migrations.sh — Apply health-scorer migrations in the correct order.
#
# Usage:
#   DATABASE_URL="postgresql://user:pass@host:port/dbname" ./migrations/apply_migration.sh
#   ./migrations/apply_migration.sh postgresql://user:pass@host:port/dbname
#
# Defaults to postgresql://crawler:@localhost:5432/craw
#
# Required environment variables (when using default DATABASE_URL):
#   None — database URL is passed as argument or hardcoded default.
#
# Transaction boundaries:
#   - 0001a runs via psql (each statement auto-committed, DDL is transactional).
#   - 0001b runs via psql (each statement auto-committed, CONCURRENTLY requires this).
#   - Neither file contains BEGIN/COMMIT; psql handles each statement individually.
#
# Idempotency:
#   - ADD COLUMN IF NOT EXISTS, CREATE INDEX IF NOT EXISTS, CREATE TABLE IF NOT EXISTS
#     are idempotent.
#   - ADD CONSTRAINT is NOT idempotent (PostgreSQL limitation). Re-running will fail
#     with "constraint already exists". This is expected and non-destructive.
#   - If the script fails mid-way, the database is in a valid state (partial DDL
#     was committed). Review the error, fix, and re-run from the failed step.

set -euo pipefail

DATABASE_URL="${1:-postgresql://crawler:@localhost:5432/craw}"
MIGRATIONS_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "=== Health Scorer Migration Runner ==="
echo "Database: $DATABASE_URL"
echo ""

# Step 1: Apply transactional DDL (0001a)
echo "[1/2] Applying 0001a_health_scoring_tables.sql (transactional)..."
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
    -f "$MIGRATIONS_DIR/0001a_health_scoring_tables.sql"
echo "  -> OK"

# Step 2: Apply non-transactional DDL (0001b) — NO BEGIN/COMMIT
echo "[2/2] Applying 0001b_health_scoring_indexes.sql (CONCURRENTLY, non-transactional)..."
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
    -f "$MIGRATIONS_DIR/0001b_health_scoring_indexes.sql"
echo "  -> OK"

echo ""
echo "=== Migrations applied successfully ==="
