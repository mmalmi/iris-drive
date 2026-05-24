#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${IRIS_DRIVE_DEV_LAB_ENV:-$HOME/.config/iris-drive/dev-lab.env}"

log() {
  printf '[dev-lab] %s\n' "$*" >&2
}

remote_host_for_git_remote() {
  local remote="$1"
  local url
  url="$(git -C "$ROOT" remote get-url "$remote" 2>/dev/null || true)"
  [[ "$url" == *:* && "$url" != *"://"* ]] || return 1
  printf '%s\n' "${url%%:*}"
}

detect_macos_team_id() {
  local remote="${IRIS_DRIVE_DEV_VM_MACOS_REMOTE:-macos}"
  local host
  local identities
  local team
  host="$(remote_host_for_git_remote "$remote")" || return 1
  identities="$(ssh "$host" 'security find-identity -p codesigning -v 2>/dev/null')" || return 1
  team="$(printf '%s\n' "$identities" \
    | sed -En 's/.*"(Apple Distribution|Developer ID Application): .* \(([A-Z0-9]+)\)".*/\2/p' \
    | head -n 1)"
  if [[ -z "$team" ]]; then
    team="$(printf '%s\n' "$identities" \
      | sed -En 's/.*"Apple Development: .* \(([A-Z0-9]+)\)".*/\1/p' \
      | head -n 1)"
  fi
  printf '%s\n' "$team"
}

if [[ -f "$ENV_FILE" ]]; then
  log "loading $ENV_FILE"
  set -a
  # shellcheck disable=SC1090
  . "$ENV_FILE"
  set +a
fi

export IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER="${IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER:-1}"
export IRIS_DRIVE_DEV_VM_MIN_FREE_KB="${IRIS_DRIVE_DEV_VM_MIN_FREE_KB:-1048576}"
export IRIS_DRIVE_DEV_VM_CONNECTIVITY_TIMEOUT="${IRIS_DRIVE_DEV_VM_CONNECTIVITY_TIMEOUT:-75}"

if [[ -z "${IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM:-}" && "${IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER}" != "0" ]]; then
  if team_id="$(detect_macos_team_id)" && [[ -n "$team_id" ]]; then
    export IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM="$team_id"
    log "using macOS Apple Development team $team_id"
  else
    log "no macOS Apple Development team auto-detected; set IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM in $ENV_FILE if FileProvider signing fails"
  fi
fi

exec "$ROOT/scripts/dev-vm-update-run.sh" "$@"
