#!/usr/bin/env bash
# Toggle maintenance mode for Azeroth Private Server.
# Usage: maintenance.sh on | off | status
#
# "on"  — creates /tmp/azeroth-maintenance, saves current MOTD, sets maintenance MOTD
# "off" — removes the flag file, restores the original MOTD
# "status" — prints current maintenance state

set -euo pipefail

FLAG_FILE="/tmp/azeroth-maintenance"
SAVED_MOTD_FILE="/tmp/azeroth-maintenance-saved-motd"
MAINTENANCE_MOTD="Server is under active maintenance. Expect frequent restarts."

# Load DB credentials from .env
ENV_FILE="$(dirname "$0")/../.env"
if [[ -f "$ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$ENV_FILE"
fi

DB_USER="${DB_USER:-mangos}"
DB_PASSWORD="${DB_PASSWORD:-}"
DB_HOST="${DB_HOST:-127.0.0.1}"
DB_PORT="${DB_PORT:-3306}"

mysql_cmd() {
    mysql -u"$DB_USER" -p"$DB_PASSWORD" -h"$DB_HOST" -P"$DB_PORT" --batch --skip-column-names "$@"
}

get_current_motd() {
    mysql_cmd -e "SELECT motd FROM realmd.realmlist WHERE id = 1;" 2>/dev/null || echo ""
}

set_motd() {
    local motd="$1"
    mysql_cmd -e "UPDATE realmd.realmlist SET motd = '$(printf '%s' "$motd" | sed "s/'/\\\\'/g")' WHERE id = 1;" 2>/dev/null
}

case "${1:-status}" in
    on)
        if [[ -f "$FLAG_FILE" ]]; then
            echo "Maintenance mode is already ON."
            exit 0
        fi
        # Save current MOTD before overwriting
        current_motd="$(get_current_motd)"
        echo "$current_motd" > "$SAVED_MOTD_FILE"
        set_motd "$MAINTENANCE_MOTD"
        touch "$FLAG_FILE"
        echo "Maintenance mode ON. MOTD updated."
        ;;
    off)
        if [[ ! -f "$FLAG_FILE" ]]; then
            echo "Maintenance mode is already OFF."
            exit 0
        fi
        # Restore saved MOTD
        if [[ -f "$SAVED_MOTD_FILE" ]]; then
            saved_motd="$(cat "$SAVED_MOTD_FILE")"
            set_motd "$saved_motd"
            rm -f "$SAVED_MOTD_FILE"
            echo "MOTD restored."
        fi
        rm -f "$FLAG_FILE"
        echo "Maintenance mode OFF."
        ;;
    status)
        if [[ -f "$FLAG_FILE" ]]; then
            echo "Maintenance mode is ON."
        else
            echo "Maintenance mode is OFF."
        fi
        ;;
    *)
        echo "Usage: $0 {on|off|status}"
        exit 1
        ;;
esac
