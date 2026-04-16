#!/bin/sh
set -eu

URL="http://127.0.0.1:3000/"
CHROMIUM="/usr/bin/chromium"
XDG_RUNTIME_DIR_VALUE="${XDG_RUNTIME_DIR:-/run/user/1000}"
WAYLAND_DISPLAY_VALUE="${WAYLAND_DISPLAY:-wayland-0}"
DBUS_SESSION_BUS_ADDRESS_VALUE="${DBUS_SESSION_BUS_ADDRESS:-unix:path=/run/user/1000/bus}"
LOG_DIR="${HOME:-/home/pi}/.cache"
LOG_FILE="$LOG_DIR/amplifier-chromium.log"

mkdir -p "$LOG_DIR"

for _ in $(seq 1 60); do
    if curl --silent --fail --output /dev/null "$URL"; then
        break
    fi
    sleep 1
done

if ! curl --silent --fail --output /dev/null "$URL"; then
    echo "Amplifier backend did not become ready at $URL" >&2
    exit 1
fi

for _ in $(seq 1 60); do
    if [ -S "$XDG_RUNTIME_DIR_VALUE/$WAYLAND_DISPLAY_VALUE" ] &&
        [ -S "/run/user/1000/bus" ]; then
        break
    fi
    sleep 1
done

{
    echo "$(date): opening $URL"
    exec env \
        XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR_VALUE" \
        WAYLAND_DISPLAY="$WAYLAND_DISPLAY_VALUE" \
        DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS_VALUE" \
        "$CHROMIUM" --ozone-platform=wayland --new-window "$URL"
} >>"$LOG_FILE" 2>&1
