#!/usr/bin/env bash
# Builds the Jarvis UI on this machine and copies it to the 8 GB Pi.
#
#   scripts/deploy-ui.sh             build, then copy ui/dist, the service file and the kiosk launcher
#   scripts/deploy-ui.sh --dry-run   build, then print the copy commands without running them
#
# Needs JARVIS_PASS (from the environment or the repo's .env); see docs/agents/pi.md.
# One-time setup on the Pi (enable the service, labwc autostart, cursor theme) is in
# the launch section of docs/agents/pi.md.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

PI_HOST="${JARVIS_PI_HOST:-darrenrasp@jarvis}"
PI_UI_DIST="jarvis/ui/dist"              # relative to the Pi home; the service serves this
PI_SERVICE_DIR=".config/systemd/user"
PI_BIN_DIR=".local/bin"
SSH_OPTS="ssh -o NumberOfPasswordPrompts=1"

dry_run=false
case "${1:-}" in
    "") ;;
    --dry-run) dry_run=true ;;
    *) echo "usage: scripts/deploy-ui.sh [--dry-run]" >&2; exit 2 ;;
esac

# Runs a command, or only prints it in dry-run mode.
run() {
    if $dry_run; then
        printf 'dry run:'; printf ' %q' "$@"; printf '\n'
    else
        "$@"
    fi
}

npm ci --prefix ui
npm run --prefix ui build

if ! $dry_run && [[ -z "${JARVIS_PASS:-}" ]]; then
    JARVIS_PASS="$(grep -E '^JARVIS_PASS=' .env | cut -d= -f2-)"
    [[ -n "$JARVIS_PASS" ]] || { echo "JARVIS_PASS is not set and not in .env" >&2; exit 1; }
    export JARVIS_PASS
fi

pi() { run scripts/pi/ssh.exp "$@"; }

pi rsync -az --delete -e "$SSH_OPTS" ui/dist/ "$PI_HOST:$PI_UI_DIST/"
pi rsync -a -e "$SSH_OPTS" scripts/pi/jarvis-ui.service "$PI_HOST:$PI_SERVICE_DIR/"
pi rsync -a -e "$SSH_OPTS" scripts/pi/jarvis-kiosk.sh "$PI_HOST:$PI_BIN_DIR/"
pi ssh -o NumberOfPasswordPrompts=1 "$PI_HOST" \
    "chmod 755 ~/$PI_BIN_DIR/jarvis-kiosk.sh && systemctl --user daemon-reload"

if $dry_run; then
    echo "dry run: nothing was copied"
else
    echo "deployed ui/dist to $PI_HOST:~/$PI_UI_DIST (the service serves it; no restart needed)"
fi
