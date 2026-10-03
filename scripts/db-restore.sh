#!/usr/bin/env bash
#
# scripts/db-restore.sh: Restore the PostgreSQL database from a SQL dump file.
#
# Usage:
#   ./scripts/db-restore.sh [backup_file]
#
# If no backup_file is specified, restores the latest dump from:
#   .substrate/backups/tks-db-*.sql

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BACKUP_DIR="${WORKSPACE_ROOT}/.substrate/backups"

# Resolve backup file
if [[ -n "${1:-}" ]]; then
    BACKUP_FILE="$1"
else
    # Find latest backup
    BACKUP_FILE="$(ls -t "${BACKUP_DIR}"/tks-db-*.sql 2>/dev/null | head -n1 || true)"
fi

if [[ -z "${BACKUP_FILE}" || ! -f "${BACKUP_FILE}" ]]; then
    echo "ERROR: Backup file not found: ${BACKUP_FILE:-<none>}" >&2
    echo "Usage: $0 [path/to/backup.sql]" >&2
    exit 1
fi

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

echo "==> Restoring TKS database..."
echo "    Database URL: ${DB_URL}"
echo "    Source Dump:  ${BACKUP_FILE}"

# Filter out Postgres 17+ specific options (like transaction_timeout) when targeting Postgres 16
sed '/transaction_timeout/d' "${BACKUP_FILE}" | psql "${DB_URL}" -v ON_ERROR_STOP=1

echo "==> Database restore completed successfully from: ${BACKUP_FILE}"
