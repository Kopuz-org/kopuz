#!/usr/bin/env bash
# Clear the lyrics cache from Kopuz's SQLite database.
# Usage: ./scripts/clear-lyrics-cache.sh [--debug|--release]

set -euo pipefail

DB_DIR="${KOPUZ_DB_DIR:-$HOME/.config/kopuz}"

case "${1:---debug}" in
    --debug)  DB="$DB_DIR/kopuz-debug.db" ;;
    --release) DB="$DB_DIR/kopuz.db" ;;
    *) echo "Usage: $0 [--debug|--release]"; exit 1 ;;
esac

if [ ! -f "$DB" ]; then
    echo "Database not found: $DB"
    exit 1
fi

sqlite3 "$DB" "PRAGMA wal_checkpoint(TRUNCATE);" >/dev/null 2>/dev/null || true
sqlite3 "$DB" "DELETE FROM metadata_cache WHERE kind = 'lyrics';"
echo "OK"
