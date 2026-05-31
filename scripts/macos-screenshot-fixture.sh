#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT="${IRIS_DRIVE_MACOS_SCREENSHOT_OUTPUT:-$ROOT/artifacts/screenshots/iris-drive-macos-window.png}"
TAB="${IRIS_DRIVE_MACOS_SCREENSHOT_TAB:-devices}"
BUILD=1
KEEP_OPEN=0
APP_PATH="${IRIS_DRIVE_MACOS_SCREENSHOT_APP:-}"

usage() {
  cat <<'EOF'
usage: scripts/macos-screenshot-fixture.sh [--output PATH] [--tab drive|devices|backups|settings] [--no-build] [--keep-open]

Builds or reuses the macOS app, launches it in debug screenshot fixture mode,
and captures only the Iris Drive window.

Environment:
  IRIS_DRIVE_MACOS_SCREENSHOT_OUTPUT   output PNG path
  IRIS_DRIVE_MACOS_SCREENSHOT_TAB      initial tab (default devices)
  IRIS_DRIVE_MACOS_SCREENSHOT_APP      existing app bundle for --no-build
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      OUTPUT="$2"
      shift 2
      ;;
    --tab)
      TAB="$2"
      shift 2
      ;;
    --no-build)
      BUILD=0
      shift
      ;;
    --keep-open)
      KEEP_OPEN=1
      shift
      ;;
    -h|--help|help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$BUILD" -eq 1 ]]; then
  APP_PATH="$("$ROOT/scripts/macos-dev-app.sh" build)"
fi

if [[ -z "$APP_PATH" ]]; then
  APP_PATH="$ROOT/macos/.build/DerivedData/Build/Products/Debug/Iris Drive.app"
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "Iris Drive app not found at $APP_PATH" >&2
  exit 1
fi

executable="$APP_PATH/Contents/MacOS/Iris Drive"
if [[ ! -x "$executable" ]]; then
  echo "Iris Drive executable not found at $executable" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUTPUT")"

IRIS_DRIVE_MACOS_SCREENSHOT_FIXTURE=1 \
IRIS_DRIVE_DISABLE_FILEPROVIDER=1 \
  "$executable" --iris-drive-screenshot-fixture --iris-drive-screenshot-tab "$TAB" &
pid="$!"

cleanup() {
  if [[ "$KEEP_OPEN" -eq 0 ]]; then
    kill "$pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

bounds="$(
  osascript <<OSA
tell application "System Events"
  repeat 100 times
    try
      set fixtureProcess to first process whose unix id is $pid
      set frontmost of fixtureProcess to true
      if exists window 1 of fixtureProcess then
        set windowPosition to position of window 1 of fixtureProcess
        set windowSize to size of window 1 of fixtureProcess
        return (item 1 of windowPosition as text) & "," & (item 2 of windowPosition as text) & "," & (item 1 of windowSize as text) & "," & (item 2 of windowSize as text)
      end if
    end try
    delay 0.1
  end repeat
end tell
error "Timed out waiting for Iris Drive fixture window"
OSA
)"

screencapture -x -R"$bounds" "$OUTPUT"
printf '%s\n' "$OUTPUT"
