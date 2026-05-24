#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MACOS_REMOTE="${IRIS_DRIVE_DEV_VM_MACOS_REMOTE:-macos-utm}"
UBUNTU_REMOTE="${IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE:-ubuntu-dev}"
WINDOWS_REMOTE="${IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE:-win11-dev}"
RUN_ID="${IRIS_DRIVE_DEV_VM_SMOKE_ID:-$(date -u +%Y%m%d-%H%M%S)}"
SMOKE_DIR="codex-lab-smoke/$RUN_ID"

log() {
  printf '[dev-vm-smoke] %s\n' "$*" >&2
}

die() {
  printf '[dev-vm-smoke] ERROR: %s\n' "$*" >&2
  exit 1
}

win_ps() {
  ssh "$WINDOWS_REMOTE" \
    'powershell -NoProfile -ExecutionPolicy Bypass -Command "`$script = [Console]::In.ReadToEnd(); Invoke-Expression `$script"'
}

win_idrive_json() {
  local args=("$@")
  local ps_args=""
  local arg
  for arg in "${args[@]}"; do
    ps_args+=" '$(printf "%s" "$arg" | sed "s/'/''/g")'"
  done
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$IrisRepo = Join-Path \$HOME "src\\iris-drive"
\$Idrive = Join-Path \$IrisRepo "windows\\bin\\Debug\\net8.0-windows\\win-x64\\publish\\idrive.exe"
if (-not (Test-Path \$Idrive)) {
  \$Idrive = Join-Path \$IrisRepo "target\\debug\\idrive.exe"
}
\$ConfigDir = Join-Path \$env:APPDATA "iris-drive"
& \$Idrive --config-dir \$ConfigDir$ps_args
REMOTE_PS
}

macos_idrive_json() {
  local args=("$@")
  ssh "$MACOS_REMOTE" 'bash -se' "${args[@]}" <<'REMOTE_SH'
set -Eeuo pipefail
config_dir="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$HOME/Library/Containers/to.iris.drive.macos/Data/Library/Application Support/Iris Drive Dev}/Config"
"$HOME/src/iris-drive/target/debug/idrive" --config-dir "$config_dir" "$@"
REMOTE_SH
}

ubuntu_provider_has() {
  local path="$1"
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
"$HOME/src/iris-drive/target/debug/idrive" provider list \
  | python3 -c 'import json, sys; needle = sys.argv[1]; data = json.load(sys.stdin); raise SystemExit(0 if any(e.get("path") == needle for e in data.get("entries", [])) else 1)' "$path"
REMOTE_SH
}

macos_provider_has() {
  local path="$1"
  ssh "$MACOS_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
config_dir="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$HOME/Library/Containers/to.iris.drive.macos/Data/Library/Application Support/Iris Drive Dev}/Config"
"$HOME/src/iris-drive/target/debug/idrive" --config-dir "$config_dir" provider list \
  | python3 -c 'import json, sys; needle = sys.argv[1]; data = json.load(sys.stdin); raise SystemExit(0 if any(e.get("path") == needle for e in data.get("entries", [])) else 1)' "$path"
REMOTE_SH
}

windows_provider_has() {
  local path="$1"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$IrisRepo = Join-Path \$HOME "src\\iris-drive"
\$Idrive = Join-Path \$IrisRepo "windows\\bin\\Debug\\net8.0-windows\\win-x64\\publish\\idrive.exe"
if (-not (Test-Path \$Idrive)) {
  \$Idrive = Join-Path \$IrisRepo "target\\debug\\idrive.exe"
}
\$ConfigDir = Join-Path \$env:APPDATA "iris-drive"
\$Provider = & \$Idrive --config-dir \$ConfigDir provider list | ConvertFrom-Json
if (@(\$Provider.entries | Where-Object { \$_.path -eq "$path" }).Count -gt 0) {
  exit 0
}
exit 1
REMOTE_PS
}

windows_provider_missing() {
  ! windows_provider_has "$1"
}

macos_provider_missing() {
  ! macos_provider_has "$1"
}

windows_disk_state() {
  local path="$1"
  win_ps <<REMOTE_PS | tr -d '\r'
\$ErrorActionPreference = "Stop"
\$Path = Join-Path \$HOME ("Iris Drive\\$path")
if (Test-Path -LiteralPath \$Path) {
  \$Item = Get-Item -LiteralPath \$Path -Force
  Write-Output ("yes:" + \$Item.Attributes.ToString())
} else {
  Write-Output "no"
}
REMOTE_PS
}

wait_for() {
  local label="$1"
  local timeout="$2"
  shift 2
  local start
  start="$(date +%s)"
  while true; do
    if "$@"; then
      return 0
    fi
    if (( $(date +%s) - start >= timeout )); then
      die "timed out waiting for $label"
    fi
    sleep 1
  done
}

wait_windows_disk_has() {
  local path="$1"
  [[ "$(windows_disk_state "$path")" == yes:* ]]
}

wait_windows_disk_missing() {
  local path="$1"
  [[ "$(windows_disk_state "$path")" == "no" ]]
}

wait_windows_disk_reparse() {
  local path="$1"
  [[ "$(windows_disk_state "$path")" == *ReparsePoint* ]]
}

wait_ubuntu_file_has() {
  local path="$1"
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
test -f "$HOME/Iris Drive/$path"
REMOTE_SH
}

wait_ubuntu_missing() {
  local path="$1"
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
test ! -e "$HOME/Iris Drive/$path"
REMOTE_SH
}

assert_no_ignored_provider_paths() {
  local label="$1"
  local json="$2"
  printf '%s\n' "$json" | python3 -c '
import json
import sys

label = sys.argv[1]
data = json.load(sys.stdin)
bad = []
for entry in data.get("entries", []):
    parts = entry.get("path", "").split("/")
    if any(part == ".Trash" or part.startswith(".Trash-") or part == "$RECYCLE.BIN" for part in parts):
        bad.append(entry.get("path", ""))
if bad:
    raise SystemExit(f"{label} provider exposes ignored trash paths: {bad}")
' "$label"
}

check_revisions() {
  local local_head
  local_head="$(git -C "$ROOT" rev-parse --short HEAD)"
  log "checking VM revisions against $local_head"
  [[ "$(ssh "$UBUNTU_REMOTE" 'git -C ~/src/iris-drive rev-parse --short HEAD')" == "$local_head" ]] \
    || die "ubuntu VM is not on $local_head"
  [[ "$(ssh "$MACOS_REMOTE" 'git -C ~/src/iris-drive rev-parse --short HEAD')" == "$local_head" ]] \
    || die "macOS VM is not on $local_head"
  [[ "$(win_ps <<'REMOTE_PS' | tr -d '\r'
git -C (Join-Path $HOME "src\iris-drive") rev-parse --short HEAD
REMOTE_PS
)" == "$local_head" ]] || die "Windows VM is not on $local_head"
}

check_fips_online() {
  log "checking FIPS roster online state"
  local ubuntu_status
  local macos_status
  local windows_status
  ubuntu_status="$(ssh "$UBUNTU_REMOTE" '"$HOME/src/iris-drive/target/debug/idrive" status')"
  macos_status="$(ssh "$MACOS_REMOTE" 'config_dir="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$HOME/Library/Containers/to.iris.drive.macos/Data/Library/Application Support/Iris Drive Dev}/Config"; "$HOME/src/iris-drive/target/debug/idrive" --config-dir "$config_dir" status')"
  windows_status="$(win_idrive_json status)"
  STATUS_UBUNTU="$ubuntu_status" STATUS_MACOS="$macos_status" STATUS_WINDOWS="$windows_status" python3 <<'PY'
import json
import os

statuses = {
    "ubuntu": json.loads(os.environ["STATUS_UBUNTU"]),
    "macos": json.loads(os.environ["STATUS_MACOS"]),
    "windows": json.loads(os.environ["STATUS_WINDOWS"]),
}
for name, status in statuses.items():
    peers = {
        peer.get("label"): peer
        for peer in status.get("peers", [])
        if not peer.get("is_current_device")
    }
    offline = [
        f"{label}: online={peer.get('fips_online')} state={peer.get('sync_state')}"
        for label, peer in sorted(peers.items())
        if not peer.get("fips_online")
    ]
    if offline:
        raise SystemExit(f"{name} has offline FIPS peers: {offline}")
PY
}

check_provider_noise() {
  log "checking provider views for ignored trash paths"
  assert_no_ignored_provider_paths ubuntu "$(ssh "$UBUNTU_REMOTE" '"$HOME/src/iris-drive/target/debug/idrive" provider list')"
  assert_no_ignored_provider_paths macos "$(ssh "$MACOS_REMOTE" 'config_dir="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$HOME/Library/Containers/to.iris.drive.macos/Data/Library/Application Support/Iris Drive Dev}/Config"; "$HOME/src/iris-drive/target/debug/idrive" --config-dir "$config_dir" provider list')"
  assert_no_ignored_provider_paths windows "$(win_idrive_json provider list)"
}

write_windows_file() {
  local path="$1"
  local content="$2"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$Path = Join-Path \$HOME ("Iris Drive\\$path")
\$Parent = Split-Path -Parent \$Path
New-Item -ItemType Directory -Force -Path \$Parent | Out-Null
Set-Content -LiteralPath \$Path -Encoding ASCII -Value "$content"
REMOTE_PS
}

delete_windows_path() {
  local path="$1"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$Path = Join-Path \$HOME ("Iris Drive\\$path")
Remove-Item -LiteralPath \$Path -Force -Recurse
REMOTE_PS
}

write_ubuntu_file() {
  local path="$1"
  local content="$2"
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" "$content" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
content="$2"
mkdir -p "$(dirname "$HOME/Iris Drive/$path")"
printf '%s\n' "$content" > "$HOME/Iris Drive/$path"
REMOTE_SH
}

delete_ubuntu_path() {
  local path="$1"
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
rm -rf "$HOME/Iris Drive/$path"
REMOTE_SH
}

write_macos_provider_file() {
  local path="$1"
  local content="$2"
  ssh "$MACOS_REMOTE" 'bash -se' "$path" "$content" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
content="$2"
config_dir="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$HOME/Library/Containers/to.iris.drive.macos/Data/Library/Application Support/Iris Drive Dev}/Config"
tmp="$(mktemp -t iris-drive-macos-provider-write)"
trap 'rm -f "$tmp"' EXIT
printf '%s\n' "$content" > "$tmp"
"$HOME/src/iris-drive/target/debug/idrive" --config-dir "$config_dir" provider write "$path" "$tmp" >/dev/null
REMOTE_SH
}

delete_macos_provider_path() {
  local path="$1"
  macos_idrive_json provider delete "$path" >/dev/null
}

run_sync_smoke() {
  local windows_file="$SMOKE_DIR/from-windows.txt"
  local ubuntu_file="$SMOKE_DIR/from-ubuntu-placeholder.txt"
  local macos_file="$SMOKE_DIR/from-macos-provider.txt"
  local macos_delete_file="$SMOKE_DIR/delete-from-macos-provider.txt"

  log "checking Windows-origin create then Ubuntu-origin delete"
  write_windows_file "$windows_file" "from windows $RUN_ID"
  wait_for "Windows file reaches Ubuntu" 60 wait_ubuntu_file_has "$windows_file"
  wait_for "Windows file reaches macOS provider" 60 macos_provider_has "$windows_file"
  delete_ubuntu_path "$windows_file"
  wait_for "Ubuntu delete removes Windows disk file" 60 wait_windows_disk_missing "$windows_file"
  wait_for "Ubuntu delete removes Windows provider file" 60 windows_provider_missing "$windows_file"

  log "checking Windows placeholder delete publishes back to Ubuntu"
  write_ubuntu_file "$ubuntu_file" "from ubuntu $RUN_ID"
  wait_for "Ubuntu file reaches Windows disk" 60 wait_windows_disk_has "$ubuntu_file"
  wait_for "Ubuntu file is represented as a Windows Cloud Files placeholder" 60 wait_windows_disk_reparse "$ubuntu_file"
  delete_windows_path "$ubuntu_file"
  wait_for "Windows placeholder delete removes Ubuntu file" 75 wait_ubuntu_missing "$ubuntu_file"
  wait_for "Windows placeholder delete removes Windows provider file" 75 windows_provider_missing "$ubuntu_file"
  wait_for "Windows placeholder delete removes macOS provider file" 75 macos_provider_missing "$ubuntu_file"

  log "checking macOS-origin provider create then Windows-origin delete"
  write_macos_provider_file "$macos_file" "from macos $RUN_ID"
  wait_for "macOS provider file reaches Ubuntu" 60 wait_ubuntu_file_has "$macos_file"
  wait_for "macOS provider file reaches Windows disk" 60 wait_windows_disk_has "$macos_file"
  wait_for "macOS provider file is represented as a Windows Cloud Files placeholder" 60 wait_windows_disk_reparse "$macos_file"
  delete_windows_path "$macos_file"
  wait_for "Windows delete removes macOS provider file" 75 macos_provider_missing "$macos_file"
  wait_for "Windows delete removes Ubuntu copy of macOS file" 75 wait_ubuntu_missing "$macos_file"

  log "checking Ubuntu-origin create then macOS-origin provider delete"
  write_ubuntu_file "$macos_delete_file" "delete from macos $RUN_ID"
  wait_for "Ubuntu file reaches macOS provider" 60 macos_provider_has "$macos_delete_file"
  wait_for "Ubuntu file reaches Windows disk before macOS delete" 60 wait_windows_disk_has "$macos_delete_file"
  delete_macos_provider_path "$macos_delete_file"
  wait_for "macOS provider delete removes Ubuntu file" 75 wait_ubuntu_missing "$macos_delete_file"
  wait_for "macOS provider delete removes Windows disk file" 75 wait_windows_disk_missing "$macos_delete_file"
  wait_for "macOS provider delete removes Windows provider file" 75 windows_provider_missing "$macos_delete_file"

  delete_ubuntu_path "$SMOKE_DIR" || true
}

run_macos_open_smoke() {
  case "${IRIS_DRIVE_DEV_VM_SMOKE_MACOS_UI:-1}" in
    1|true|TRUE|yes|YES|on|ON) ;;
    *) log "skipping macOS UI smoke"; return 0 ;;
  esac

  log "requesting macOS Open Drive Folder"
  local before
  before="$(ssh "$MACOS_REMOTE" 'test -f /tmp/iris-drive-macos-app.err && wc -l < /tmp/iris-drive-macos-app.err || echo 0')"
  ssh "$MACOS_REMOTE" '/usr/bin/swift -' <<'REMOTE_SWIFT' >/dev/null
import Foundation

DistributedNotificationCenter.default().postNotificationName(
    Notification.Name("to.iris.drive.showDriveFolder"),
    object: nil,
    userInfo: nil,
    deliverImmediately: true
)
RunLoop.current.run(until: Date().addingTimeInterval(0.2))
REMOTE_SWIFT

  local start
  start="$(date +%s)"
  while true; do
    local recent
    recent="$(ssh "$MACOS_REMOTE" "tail -n +$((before + 1)) /tmp/iris-drive-macos-app.err 2>/dev/null || true")"
    if grep -F "Iris Drive FileProvider open failed" <<<"$recent" >/dev/null ||
      grep -F "Iris Drive failed to open mounted drive folder" <<<"$recent" >/dev/null; then
      printf '%s\n' "$recent" >&2
      die "macOS Open Drive Folder logged a FileProvider open failure"
    fi
    if grep -F "Iris Drive mounted drive folder opened" <<<"$recent" >/dev/null ||
      grep -F "Iris Drive mounted drive folder revealed" <<<"$recent" >/dev/null; then
      return 0
    fi
    if (( $(date +%s) - start >= 15 )); then
      printf '%s\n' "$recent" >&2
      die "macOS Open Drive Folder did not log success"
    fi
    sleep 1
  done
}

check_revisions
check_fips_online
check_provider_noise
run_sync_smoke
run_macos_open_smoke
log "ok"
