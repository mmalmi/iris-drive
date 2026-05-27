#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP_BIN="$ROOT/linux/target/debug/iris-drive"
DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
DESKTOP_FILE="$DESKTOP_DIR/iris-drive.desktop"

if [[ ! -x "$APP_BIN" ]]; then
    echo "Build the Linux app first: just linux-build" >&2
    exit 1
fi

mkdir -p "$DESKTOP_DIR" "$ICON_DIR"
cp "$ROOT/linux/resources/iris-drive.svg" "$ICON_DIR/iris-drive.svg"
sed "s|^Exec=.*|Exec=$APP_BIN %u|" \
    "$ROOT/linux/resources/iris-drive.desktop" > "$DESKTOP_FILE"
chmod 0644 "$DESKTOP_FILE"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "$DESKTOP_FILE"
