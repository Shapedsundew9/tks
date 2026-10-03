#!/usr/bin/env bash
#
# scripts/db-backup.sh: Back up the PostgreSQL database to a timestamped SQL dump.
#
# Usage:
#   ./scripts/db-backup.sh [destination_file]
#
# Default destination:
#   .substrate/backups/tks-db-<YYYYMMDD-HHMMSS>.sql

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

BACKUP_DIR="${WORKSPACE_ROOT}/.substrate/backups"
mkdir -p "${BACKUP_DIR}"

TIMESTAMP="$(date +"%Y%m%d-%H%M%S")"
DEST_FILE="${1:-${BACKUP_DIR}/tks-db-${TIMESTAMP}.sql}"

# Resolve database connection
if [[ -n "${TKS_DATABASE_URL:-}" ]]; then
    DB_URL="${TKS_DATABASE_URL}"
elif [[ -n "${DATABASE_URL:-}" ]]; then
    DB_URL="${DATABASE_URL}"
elif getent hosts postgres >/dev/null 2>&1; then
    DB_URL="postgresql://postgres:postgres@postgres:5432/postgres"
else
    DB_URL="postgresql://postgres:postgres@localhost:5432/postgres"
fi

echo "==> Backing up TKS database..."
echo "    Database URL: ${DB_URL}"
echo "    Destination:  ${DEST_FILE}"

pg_dump "${DB_URL}" \
    --clean \
    --if-exists \
    --no-owner \
    --no-privileges | sed '/transaction_timeout/d' > "${DEST_FILE}"

FILE_SIZE="$(du -h "${DEST_FILE}" | cut -f1)"
echo "==> Backup completed successfully (${FILE_SIZE}): ${DEST_FILE}"
