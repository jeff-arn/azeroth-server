#!/usr/bin/env bash
# Toggle maintenance mode for Azeroth Private Server.
# Usage: maintenance.sh on | off | status
#
# "on"  — creates /tmp/azeroth-maintenance, saves current MOTD, sets maintenance MOTD
# "off" — removes the flag file, restores the original MOTD
# "status" — prints current maintenance state
#
# The MOTD is the Motd line in mangosd.conf. Changes take effect on next server start.

set -euo pipefail

FLAG_FILE="/tmp/azeroth-maintenance"
SAVED_MOTD_FILE="/tmp/azeroth-maintenance-saved-motd"
MAINTENANCE_MOTD="Server is under active maintenance. Expect frequent restarts."
MANGOSD_CONF="/home/wow/wow-private-server/vmangos/run/etc/mangosd.conf"

get_current_motd() {
    grep -Po '^Motd\s*=\s*"\K[^"]*' "$MANGOSD_CONF" 2>/dev/null || echo ""
}

set_motd() {
    local motd="$1"
    # Escape sed special characters in the replacement string
    local escaped
    escaped=$(printf '%s' "$motd" | sed 's/[&/\]/\\&/g')
    sed -i "s/^Motd\s*=\s*\".*\"/Motd = \"${escaped}\"/" "$MANGOSD_CONF"
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
        echo "Maintenance mode ON. MOTD updated (takes effect on next server start)."
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
