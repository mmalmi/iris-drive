#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
POLL_INTERVAL="${IRIS_DRIVE_DEV_POLL_INTERVAL:-1}"

usage() {
  cat <<'EOF'
usage: scripts/macos-dev-watch.sh

Builds and launches the macOS dev app, then rebuilds and relaunches when source
files change.

Environment:
  IRIS_DRIVE_DEV_POLL_INTERVAL=<seconds>
      Watch polling interval. Defaults to 1.
  IRIS_DRIVE_DEV_ONCE=1
      Build and launch once, then exit. Useful for smoke-checking this script.
EOF
}

log() {
  printf '[iris-drive dev] %s\n' "$*" >&2
}

watched_files_snapshot() {
  (
    cd "$ROOT"
    find \
      Cargo.toml Cargo.lock Justfile scripts macos/Sources macos/FileProvider crates \
      \( -name .git -o -name target -o -name .build -o -name DerivedData \) -prune \
      -o -type f \
      \( \
        -name '*.rs' \
        -o -name '*.swift' \
        -o -name '*.toml' \
        -o -name '*.yml' \
        -o -name '*.yaml' \
        -o -name '*.plist' \
        -o -name '*.entitlements' \
        -o -name '*.sh' \
        -o -name 'Justfile' \
      \) \
      -print0 2>/dev/null \
      | sort -z \
      | xargs -0 stat -f '%m %N' 2>/dev/null
  )
}

run_once() {
  log "building and launching macOS app"
  "$ROOT/scripts/macos-dev-app.sh" run
}

main() {
  case "${1:-}" in
    -h|--help|help)
      usage
      return 0
      ;;
    "")
      ;;
    *)
      usage >&2
      return 2
      ;;
  esac

  run_once
  if [[ "${IRIS_DRIVE_DEV_ONCE:-0}" == "1" ]]; then
    return 0
  fi

  log "watching source files; press Ctrl-C to stop"
  local previous
  local current
  previous="$(watched_files_snapshot)"

  while true; do
    sleep "$POLL_INTERVAL"
    current="$(watched_files_snapshot)"
    if [[ "$current" == "$previous" ]]; then
      continue
    fi
    previous="$current"
    log "change detected; rebuilding and relaunching"
    if ! run_once; then
      log "build or launch failed; waiting for the next change"
    fi
    previous="$(watched_files_snapshot)"
  done
}

main "$@"
