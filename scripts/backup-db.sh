#!/bin/bash
# Database backup script for Azeroth Server
# Dumps all 4 databases with timestamps, compresses, and prunes old backups.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Load secrets from .env
if [ -f "$PROJECT_DIR/.env" ]; then
    export $(grep -v '^#' "$PROJECT_DIR/.env" | grep -v '^$' | xargs)
fi

BACKUP_DIR="$PROJECT_DIR/backups/db"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
MYSQL_USER="${DB_USER:-mangos}"
MYSQL_PASS="${DB_PASSWORD:?DB_PASSWORD not set}"
MYSQL_HOST="${DB_HOST:-127.0.0.1}"
MYSQL_PORT="${DB_PORT:-3306}"
DATABASES=("mangos" "characters" "realmd" "logs")
RETENTION_DAYS=14

mkdir -p "$BACKUP_DIR"

for DB in "${DATABASES[@]}"; do
    OUTFILE="${BACKUP_DIR}/${DB}_${TIMESTAMP}.sql.gz"
    mysqldump -h "$MYSQL_HOST" -P "$MYSQL_PORT" -u "$MYSQL_USER" -p"$MYSQL_PASS" \
        --single-transaction "$DB" 2>/dev/null | gzip > "$OUTFILE"
    SIZE=$(du -h "$OUTFILE" | cut -f1)
    echo "$(date): Backed up $DB -> $OUTFILE ($SIZE)"
done

# Prune backups older than retention period
PRUNED=$(find "$BACKUP_DIR" -name "*.sql.gz" -mtime +$RETENTION_DAYS -delete -print | wc -l)
echo "$(date): Backup complete. Pruned $PRUNED old files."
