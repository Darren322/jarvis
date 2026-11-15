#!/bin/sh
# Opens the Jarvis UI in a full-screen Chromium kiosk (jarvis-ui spec D6).
# Started from ~/.config/labwc/autostart. Waits for the UI server (a systemd user
# service, scripts/pi/jarvis-ui.service) so Chromium never opens a dead page.
#
# --password-store=basic is required: without it Chromium 154 on this Pi never
# sends the page request (notes/ui-spike/http-hang-2026-11-15.md).
# XCURSOR_THEME=jarvis-blank hides the pointer (see scripts/pi/make_blank_cursor.py).
UI_PORT=8766
UI_URL="http://localhost:$UI_PORT/"
WAIT_ATTEMPTS=60      # times 1 s: how long to wait for the UI server
PROFILE_DIR="$HOME/.config/jarvis-ui-chromium"

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
export XCURSOR_THEME=jarvis-blank

attempt=0
# A bare TCP connect, so the probe leaves no "GET /" in the server log (that line
# is how a Chromium launch is checked).
until python3 -c "import socket; socket.create_connection(('127.0.0.1', $UI_PORT), timeout=2).close()" 2>/dev/null; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge "$WAIT_ATTEMPTS" ]; then
        echo "jarvis-kiosk: $UI_URL did not answer after $WAIT_ATTEMPTS tries; starting Chromium anyway" >&2
        break
    fi
    sleep 1
done

exec chromium \
    --user-data-dir="$PROFILE_DIR" \
    --password-store=basic \
    --ozone-platform=wayland \
    --kiosk --no-first-run --noerrdialogs --disable-infobars \
    "$UI_URL"
