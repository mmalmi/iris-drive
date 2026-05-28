#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${IRIS_DRIVE_DEV_LAB_ENV:-$HOME/.config/iris-drive/dev-lab.env}"
if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  . "$ENV_FILE"
  set +a
fi
RUN_ID="${IRIS_DRIVE_DEV_VM_SMOKE_ID:-$(date -u +%Y%m%d-%H%M%S)}"
SMOKE_BASE_ROOT="${IRIS_DRIVE_DEV_VM_SMOKE_ROOT:-codex-lab-smoke}"
SMOKE_ROOT="$SMOKE_BASE_ROOT"
SMOKE_DIR="${IRIS_DRIVE_DEV_VM_SMOKE_DIR:-$SMOKE_ROOT/$RUN_ID}"
SMOKE_CLEANUP_ROOT="${IRIS_DRIVE_DEV_VM_SMOKE_CLEANUP_ROOT:-}"
TIMINGS_FILE="${IRIS_DRIVE_DEV_VM_SMOKE_TIMINGS_FILE:-$ROOT/target/e2e-3vms-$RUN_ID-timings.jsonl}"
MAX_SYNC_WAIT_TIMEOUT="${IRIS_DRIVE_DEV_VM_MAX_SYNC_WAIT_TIMEOUT:-30}"

cap_wait_timeout() {
  local value="$1"
  local max="$2"
  case "$value:$max" in
    *[!0-9:]* | :* | *:) printf '%s\n' "$value"; return 0 ;;
  esac
  if (( value > max )); then
    printf '%s\n' "$max"
  else
    printf '%s\n' "$value"
  fi
}

SYNC_WAIT_TIMEOUT="$(cap_wait_timeout "${IRIS_DRIVE_DEV_VM_SYNC_WAIT_TIMEOUT:-30}" "$MAX_SYNC_WAIT_TIMEOUT")"
MACOS_PROVIDER_SYNC_WAIT_TIMEOUT="$(cap_wait_timeout "${IRIS_DRIVE_DEV_VM_MACOS_PROVIDER_SYNC_WAIT_TIMEOUT:-30}" "$MAX_SYNC_WAIT_TIMEOUT")"
WINDOWS_PLACEHOLDER_DELETE_SYNC_WAIT_TIMEOUT="$(cap_wait_timeout "${IRIS_DRIVE_DEV_VM_WINDOWS_PLACEHOLDER_DELETE_SYNC_WAIT_TIMEOUT:-30}" "$MAX_SYNC_WAIT_TIMEOUT")"
SYNC_QUIET_POLL_INTERVAL="${IRIS_DRIVE_DEV_VM_SYNC_QUIET_POLL_INTERVAL:-2}"
MACOS_VISIBLE_PROBE_TIMEOUT="${IRIS_DRIVE_DEV_VM_MACOS_VISIBLE_PROBE_TIMEOUT:-3}"
SMOKE_CLEANUP_TIMEOUT="${IRIS_DRIVE_DEV_VM_SMOKE_CLEANUP_TIMEOUT:-15}"
WINDOWS_PROJECTION_STABILITY_SECONDS="${IRIS_DRIVE_DEV_VM_WINDOWS_PROJECTION_STABILITY_SECONDS:-10}"
PROJECTION_STRESS_FILES="${IRIS_DRIVE_DEV_VM_PROJECTION_STRESS_FILES:-32}"
PROJECTION_STRESS_LARGE_BYTES="${IRIS_DRIVE_DEV_VM_PROJECTION_STRESS_LARGE_BYTES:-262144}"
mkdir -p "$(dirname "$TIMINGS_FILE")"
: >"$TIMINGS_FILE"

log() {
  printf '[dev-vm-smoke] %s\n' "$*" >&2
}

die() {
  printf '[dev-vm-smoke] ERROR: %s\n' "$*" >&2
  exit 1
}

json_escape() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  printf '%s' "$value"
}

base64_arg() {
  printf '%s' "$1" | base64 | tr -d '\n'
}

record_timing() {
  local label="$1"
  local elapsed="$2"
  local status="$3"
  printf '{"run_id":"%s","label":"%s","elapsed_seconds":%s,"status":"%s"}\n' \
    "$(json_escape "$RUN_ID")" \
    "$(json_escape "$label")" \
    "$elapsed" \
    "$(json_escape "$status")" \
    >>"$TIMINGS_FILE"
}

remote_or_die() {
  local env_var="$1"
  local generic_name="$2"
  local value="${!env_var:-}"
  if [[ -n "$value" ]]; then
    printf '%s\n' "$value"
    return 0
  fi
  if git -C "$ROOT" remote get-url "$generic_name" >/dev/null 2>&1; then
    printf '%s\n' "$generic_name"
    return 0
  fi
  die "set $env_var in $ENV_FILE or add a local git remote named $generic_name"
}

ssh_host_for_label() {
  local label="$1"
  local default_host="$2"
  local env_var
  local value
  env_var="IRIS_DRIVE_DEV_VM_$(printf '%s' "$label" | tr '[:lower:]-' '[:upper:]_')_SSH_HOST"
  value="${!env_var:-}"
  printf '%s\n' "${value:-$default_host}"
}

MACOS_REMOTE="$(remote_or_die IRIS_DRIVE_DEV_VM_MACOS_REMOTE macos)"
UBUNTU_REMOTE="$(remote_or_die IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE ubuntu)"
WINDOWS_REMOTE="$(remote_or_die IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE windows)"
MACOS_SSH_HOST="$(ssh_host_for_label macos "$MACOS_REMOTE")"
UBUNTU_SSH_HOST="$(ssh_host_for_label ubuntu "$UBUNTU_REMOTE")"
WINDOWS_SSH_HOST="$(ssh_host_for_label windows "$WINDOWS_REMOTE")"

ps_single_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/''/g")"
}

win_ps() {
  ssh "$WINDOWS_SSH_HOST" \
    'cmd /d /s /c "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""`$script = [Console]::In.ReadToEnd(); & ([scriptblock]::Create(`$script))"""'
}

win_idrive_json() {
  local args=("$@")
  local ps_args=""
  local arg
  if [[ ${#args[@]} -eq 1 && "${args[0]}" == "status" ]]; then
    ssh "$WINDOWS_SSH_HOST" 'cmd /d /s /c ""%USERPROFILE%\src\iris-drive\windows\bin\Debug\net8.0-windows\win-x64\publish\idrive.exe" --config-dir "%APPDATA%\iris-drive" status"'
    return
  fi
  if [[ ${#args[@]} -eq 2 && "${args[0]}" == "provider" && "${args[1]}" == "list" ]]; then
    ssh "$WINDOWS_SSH_HOST" 'cmd /d /s /c ""%USERPROFILE%\src\iris-drive\windows\bin\Debug\net8.0-windows\win-x64\publish\idrive.exe" --config-dir "%APPDATA%\iris-drive" provider list"'
    return
  fi
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
  ssh "$MACOS_SSH_HOST" 'bash -se' "${args[@]}" <<'REMOTE_SH'
set -Eeuo pipefail
macos_config_dir() {
  if [[ -n "${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-}" ]]; then
    printf '%s\n' "$IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR/Config"
    return 0
  fi
  local candidate
  for candidate in \
    "$HOME"/Library/Group\ Containers/*.to.iris.drive/Iris\ Drive\ Dev/Config \
    "$HOME"/Library/Group\ Containers/group.to.iris.drive/Iris\ Drive\ Dev/Config \
    "$HOME"/Library/Containers/to.iris.drive.macos/Data/Library/Application\ Support/Iris\ Drive\ Dev/Config
  do
    [[ -d "$candidate" ]] || continue
    [[ -f "$candidate/key" || -f "$candidate/config.json" ]] || continue
    printf '%s\n' "$candidate"
    return 0
  done
  return 1
}
config_dir="$(macos_config_dir)"
"$HOME/src/iris-drive/target/debug/idrive" --config-dir "$config_dir" "$@"
REMOTE_SH
}

ubuntu_provider_has() {
  local path="$1"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
"$HOME/src/iris-drive/target/debug/idrive" provider list \
  | python3 -c 'import json, sys; needle = sys.argv[1]; data = json.load(sys.stdin); raise SystemExit(0 if any(e.get("path") == needle for e in data.get("entries", [])) else 1)' "$path"
REMOTE_SH
}

ubuntu_provider_missing() {
  ! ubuntu_provider_has "$1"
}

macos_provider_has() {
  local path="$1"
  macos_idrive_json provider list \
    | python3 -c 'import json, sys; needle = sys.argv[1]; data = json.load(sys.stdin); raise SystemExit(0 if any(e.get("path") == needle for e in data.get("entries", [])) else 1)' "$path"
}

macos_visible_drive_has() {
  local path="$1"
  ssh "$MACOS_SSH_HOST" 'bash -se' "$path" "$MACOS_VISIBLE_PROBE_TIMEOUT" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
probe_timeout="$2"

run_limited() {
  local limit="$1"
  shift
  "$@" &
  local pid=$!
  (
    sleep "$limit"
    kill -TERM "$pid" >/dev/null 2>&1 || true
    sleep 1
    kill -KILL "$pid" >/dev/null 2>&1 || true
  ) &
  local watchdog=$!
  local status=0
  if wait "$pid"; then
    status=0
  else
    status=$?
  fi
  kill "$watchdog" >/dev/null 2>&1 || true
  wait "$watchdog" 2>/dev/null || true
  return "$status"
}

enumerate_parent_chain() {
  local root="$1"
  local relative="$2"
  local parent
  local current
  local part

  parent="$(dirname "$relative")"
  current="$root"
  run_limited "$probe_timeout" /bin/ls -la "$current" >/dev/null 2>&1 || true
  [[ "$parent" != "." ]] || return 0
  IFS='/' read -r -a parts <<< "$parent"
  for part in "${parts[@]}"; do
    [[ -n "$part" ]] || continue
    current="$current/$part"
    run_limited "$probe_timeout" /bin/ls -la "$current" >/dev/null 2>&1 || true
  done
}

while IFS= read -r root; do
  [[ -n "$root" ]] || continue
  enumerate_parent_chain "$root" "$path"
  if run_limited "$probe_timeout" /bin/test -e "$root/$path"; then
    exit 0
  fi
done < <(find "$HOME/Library/CloudStorage" -maxdepth 1 -type d -name 'IrisDrive*' -print 2>/dev/null || true)
exit 1
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
\$Exists = Test-Path -LiteralPath \$Path
if (\$Exists) {
  \$Item = Get-Item -LiteralPath \$Path -Force
  \$Kind = if ((\$Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    "reparse"
  } else {
    "regular"
  }
  Write-Output ("yes:" + \$Kind + ":" + \$Item.Attributes.ToString())
} else {
  Write-Output "no"
}
REMOTE_PS
}

windows_start_directory_monitor() {
  local dir="$1"
  local token="$2"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$Target = Join-Path \$HOME ("Iris Drive\\$dir")
New-Item -ItemType Directory -Force -Path \$Target | Out-Null
Get-ChildItem -LiteralPath \$Target -Force | Out-Null
\$Log = Join-Path \$env:TEMP "iris-drive-fsw-$token.log"
\$PidFile = Join-Path \$env:TEMP "iris-drive-fsw-$token.pid"
\$Script = Join-Path \$env:TEMP "iris-drive-fsw-$token.ps1"
Remove-Item -LiteralPath \$Log, \$PidFile, \$Script -Force -ErrorAction SilentlyContinue
@'
param(
  [Parameter(Mandatory = \$true)][string]\$Target,
  [Parameter(Mandatory = \$true)][string]\$Log
)
\$ErrorActionPreference = "Stop"
\$Watcher = [System.IO.FileSystemWatcher]::new(\$Target)
\$Watcher.IncludeSubdirectories = \$false
\$Watcher.EnableRaisingEvents = \$true
\$Registrations = @()
foreach (\$EventName in @("Created", "Deleted", "Changed", "Renamed")) {
  \$Registrations += Register-ObjectEvent -InputObject \$Watcher -EventName \$EventName -MessageData \$Log -Action {
    \$Name = \$EventArgs.Name
    if (-not \$Name -and \$EventArgs.FullPath) {
      \$Name = [System.IO.Path]::GetFileName(\$EventArgs.FullPath)
    }
    Add-Content -LiteralPath \$Event.MessageData -Value ("{0} {1}" -f \$EventArgs.ChangeType, \$Name)
  }
}
try {
  \$Deadline = (Get-Date).AddSeconds(90)
  while ((Get-Date) -lt \$Deadline) {
    Wait-Event -Timeout 1 | Out-Null
  }
} finally {
  foreach (\$Registration in \$Registrations) {
    Unregister-Event -SubscriptionId \$Registration.Id -ErrorAction SilentlyContinue
  }
  \$Watcher.Dispose()
}
'@ | Set-Content -LiteralPath \$Script -Encoding UTF8
\$Process = Start-Process -FilePath "powershell" -ArgumentList @(
  "-NoProfile",
  "-ExecutionPolicy",
  "Bypass",
  "-File",
  \$Script,
  "-Target",
  \$Target,
  "-Log",
  \$Log
) -WindowStyle Hidden -PassThru
Set-Content -LiteralPath \$PidFile -Encoding ASCII -Value \$Process.Id
REMOTE_PS
}

windows_monitor_saw_any() {
  local token="$1"
  shift
  local needles=("$@")
  local ps_needles="@("
  local sep=""
  local needle
  for needle in "${needles[@]}"; do
    ps_needles+="$sep$(ps_single_quote "$needle")"
    sep=","
  done
  ps_needles+=")"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$Log = Join-Path \$env:TEMP "iris-drive-fsw-$token.log"
if (-not (Test-Path -LiteralPath \$Log)) {
  exit 1
}
\$Text = Get-Content -LiteralPath \$Log -Raw
foreach (\$Needle in $ps_needles) {
  if (\$Text.Contains(\$Needle)) {
    exit 0
  }
}
exit 1
REMOTE_PS
}

windows_monitor_saw_any_or_disk_has() {
  local token="$1"
  local path="$2"
  local name
  name="$(basename "$path")"
  windows_monitor_saw_any "$token" "$name" || wait_windows_disk_has "$path"
}

windows_stop_directory_monitor() {
  local token="$1"
  win_ps <<REMOTE_PS || true
\$ErrorActionPreference = "SilentlyContinue"
\$PidFile = Join-Path \$env:TEMP "iris-drive-fsw-$token.pid"
\$Log = Join-Path \$env:TEMP "iris-drive-fsw-$token.log"
\$Script = Join-Path \$env:TEMP "iris-drive-fsw-$token.ps1"
if (Test-Path -LiteralPath \$PidFile) {
  \$ProcessId = [int](Get-Content -LiteralPath \$PidFile -Raw)
  Stop-Process -Id \$ProcessId -Force
}
Remove-Item -LiteralPath \$PidFile, \$Log, \$Script -Force
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
      local elapsed=$(( $(date +%s) - start ))
      record_timing "$label" "$elapsed" "ok"
      log "ok in ${elapsed}s: $label"
      return 0
    fi
    if (( $(date +%s) - start >= timeout )); then
      local elapsed=$(( $(date +%s) - start ))
      record_timing "$label" "$elapsed" "timeout"
      die "timed out waiting for $label"
    fi
    sleep 1
  done
}

wait_for_quiet() {
  local label="$1"
  local timeout="$2"
  local interval="$3"
  shift 3
  local start
  start="$(date +%s)"
  while true; do
    if "$@"; then
      local elapsed=$(( $(date +%s) - start ))
      record_timing "$label" "$elapsed" "ok"
      log "ok in ${elapsed}s: $label"
      return 0
    fi
    if (( $(date +%s) - start >= timeout )); then
      local elapsed=$(( $(date +%s) - start ))
      record_timing "$label" "$elapsed" "timeout"
      die "timed out waiting for $label"
    fi
    sleep "$interval"
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
  [[ "$(windows_disk_state "$path")" == yes:reparse:* ]]
}

wait_windows_file_has_content() {
  local path="$1"
  local expected="$2"
  local expected_b64
  expected_b64="$(base64_arg "$expected")"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$Path = Join-Path \$HOME ("Iris Drive\\$path")
if (-not (Test-Path -LiteralPath \$Path -PathType Leaf)) {
  exit 1
}
\$Expected = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String("$expected_b64")) + [string][char]10
\$Actual = [System.IO.File]::ReadAllText(\$Path)
if (\$Actual -ne \$Expected) {
  exit 1
}
REMOTE_PS
}

windows_projection_stays_visible_during_local_create() {
  local local_path="$1"
  shift
  local expected_paths=("$@")
  local expected_ps="@("
  local separator=""
  local expected
  for expected in "${expected_paths[@]}"; do
    expected_ps+="$separator$(ps_single_quote "$expected")"
    separator=", "
  done
  expected_ps+=")"
  local start
  start="$(date +%s)"
  if win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
function FullPath([string]\$Relative) {
  \$Native = \$Relative.Replace("/", [IO.Path]::DirectorySeparatorChar)
  return Join-Path (Join-Path \$HOME "Iris Drive") \$Native
}
\$Expected = $expected_ps
\$LocalPath = FullPath $(ps_single_quote "$local_path")
\$MissingBefore = @(\$Expected | Where-Object { -not (Test-Path -LiteralPath (FullPath \$_)) })
if (\$MissingBefore.Count -gt 0) {
  throw ("expected Windows projection paths missing before local create: " + (\$MissingBefore -join ", "))
}
New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName(\$LocalPath)) | Out-Null
[System.IO.File]::WriteAllText(
  \$LocalPath,
  $(ps_single_quote "projection guard windows $RUN_ID") + [string][char]10,
  [System.Text.Encoding]::ASCII)
\$Deadline = (Get-Date).AddSeconds($WINDOWS_PROJECTION_STABILITY_SECONDS)
while ((Get-Date) -lt \$Deadline) {
  \$Missing = @(\$Expected | Where-Object { -not (Test-Path -LiteralPath (FullPath \$_)) })
  if (\$Missing.Count -gt 0) {
    throw ("Windows projection dropped expected paths during local create: " + (\$Missing -join ", "))
  }
  Start-Sleep -Milliseconds 250
}
REMOTE_PS
  then
    local elapsed=$(( $(date +%s) - start ))
    record_timing "Windows projection stays visible during local create" "$elapsed" "ok"
    log "ok in ${elapsed}s: Windows projection stays visible during local create"
    return 0
  fi
  local elapsed=$(( $(date +%s) - start ))
  record_timing "Windows projection stays visible during local create" "$elapsed" "failed"
  die "Windows projection hid existing paths during local create"
}

wait_ubuntu_file_has() {
  local path="$1"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
if command -v timeout >/dev/null 2>&1; then
  timeout 5s test -f "$HOME/Iris Drive/$path"
else
  test -f "$HOME/Iris Drive/$path"
fi
REMOTE_SH
}

wait_ubuntu_missing() {
  local path="$1"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
if command -v timeout >/dev/null 2>&1; then
  timeout 5s test ! -e "$HOME/Iris Drive/$path"
else
  test ! -e "$HOME/Iris Drive/$path"
fi
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
  case "${IRIS_DRIVE_DEV_VM_SKIP_REVISION_CHECK:-0}" in
    1|true|TRUE|yes|YES|on|ON)
      log "skipping VM revision check"
      return 0
      ;;
  esac
  local local_head
  local_head="$(git -C "$ROOT" rev-parse --short HEAD)"
  log "checking VM revisions against $local_head"
  [[ "$(ssh "$UBUNTU_SSH_HOST" 'git -C ~/src/iris-drive rev-parse --short HEAD')" == "$local_head" ]] \
    || die "ubuntu VM is not on $local_head"
  [[ "$(ssh "$MACOS_SSH_HOST" 'git -C ~/src/iris-drive rev-parse --short HEAD')" == "$local_head" ]] \
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
  ubuntu_status="$(ssh "$UBUNTU_SSH_HOST" '"$HOME/src/iris-drive/target/debug/idrive" status')"
  macos_status="$(macos_idrive_json status)"
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
  assert_no_ignored_provider_paths ubuntu "$(ssh "$UBUNTU_SSH_HOST" '"$HOME/src/iris-drive/target/debug/idrive" provider list')"
  assert_no_ignored_provider_paths macos "$(macos_idrive_json provider list)"
  assert_no_ignored_provider_paths windows "$(win_idrive_json provider list)"
}

write_windows_file() {
  local path="$1"
  local content="$2"
  local content_b64
  content_b64="$(base64_arg "$content")"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$Path = Join-Path \$HOME ("Iris Drive\\$path")
\$Parent = Split-Path -Parent \$Path
New-Item -ItemType Directory -Force -Path \$Parent | Out-Null
\$Content = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String("$content_b64"))
[System.IO.File]::WriteAllText(\$Path, \$Content + [string][char]10, [System.Text.Encoding]::ASCII)
REMOTE_PS
}

delete_windows_path() {
  local path="$1"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$Path = Join-Path \$HOME ("Iris Drive\\$path")
if (Test-Path -LiteralPath \$Path) {
  Remove-Item -LiteralPath \$Path -Force -Recurse
}
REMOTE_PS
}

rename_windows_path() {
  local old_path="$1"
  local new_path="$2"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$OldPath = Join-Path \$HOME ("Iris Drive\\$old_path")
\$NewPath = Join-Path \$HOME ("Iris Drive\\$new_path")
\$Parent = Split-Path -Parent \$NewPath
New-Item -ItemType Directory -Force -Path \$Parent | Out-Null
\$OldParent = Split-Path -Parent \$OldPath
if (\$OldParent -eq \$Parent) {
  Rename-Item -LiteralPath \$OldPath -NewName (Split-Path -Leaf \$NewPath) -Force
} else {
  Move-Item -LiteralPath \$OldPath -Destination \$NewPath -Force
}
REMOTE_PS
}

write_ubuntu_file() {
  local path="$1"
  local content="$2"
  local content_b64
  content_b64="$(base64_arg "$content")"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$path" "$content_b64" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
content_b64="$2"
content="$(python3 -c 'import base64, sys; sys.stdout.write(base64.b64decode(sys.argv[1]).decode("utf-8"))' "$content_b64")"
if command -v timeout >/dev/null 2>&1; then
  timeout 10s mkdir -p "$(dirname "$HOME/Iris Drive/$path")"
  printf '%s\n' "$content" | timeout 10s tee "$HOME/Iris Drive/$path" >/dev/null
else
  mkdir -p "$(dirname "$HOME/Iris Drive/$path")"
  printf '%s\n' "$content" > "$HOME/Iris Drive/$path"
fi
REMOTE_SH
}

write_ubuntu_zero_file() {
  local path="$1"
  local bytes="$2"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$path" "$bytes" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
bytes="$2"
target="$HOME/Iris Drive/$path"
mkdir -p "$(dirname "$target")"
head -c "$bytes" /dev/zero >"$target"
REMOTE_SH
}

delete_ubuntu_path() {
  local path="$1"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
if command -v timeout >/dev/null 2>&1; then
  timeout 10s rm -rf "$HOME/Iris Drive/$path"
else
  rm -rf "$HOME/Iris Drive/$path"
fi
REMOTE_SH
}

delete_ubuntu_provider_path() {
  local path="$1"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
"$HOME/src/iris-drive/target/debug/idrive" provider delete "$path" >/dev/null
REMOTE_SH
}

cleanup_previous_smoke_root() {
  case "${IRIS_DRIVE_DEV_VM_SMOKE_CLEAN_ROOT:-1}" in
    1|true|TRUE|yes|YES|on|ON) ;;
    *) log "skipping smoke root cleanup"; return 0 ;;
  esac
  if [[ -z "$SMOKE_CLEANUP_ROOT" ]]; then
    log "skipping smoke root cleanup"
    return 0
  fi

  local cleanup_is_current=0
  if [[ "$SMOKE_CLEANUP_ROOT" == "$SMOKE_DIR" ]]; then
    cleanup_is_current=1
  elif [[ "$SMOKE_DIR" == "$SMOKE_CLEANUP_ROOT/"* ]]; then
    die "smoke cleanup root '$SMOKE_CLEANUP_ROOT' must not contain current run dir '$SMOKE_DIR'"
  fi

  log "cleaning previous native smoke root $SMOKE_CLEANUP_ROOT"
  delete_ubuntu_path "$SMOKE_CLEANUP_ROOT" || true
  delete_ubuntu_provider_path "$SMOKE_CLEANUP_ROOT" >/dev/null 2>&1 || true
  delete_windows_path "$SMOKE_CLEANUP_ROOT" || true
  delete_windows_provider_path "$SMOKE_CLEANUP_ROOT" >/dev/null 2>&1 || true
  delete_macos_provider_path "$SMOKE_CLEANUP_ROOT" >/dev/null 2>&1 || true

  local start
  start="$(date +%s)"
  while (( $(date +%s) - start < SMOKE_CLEANUP_TIMEOUT )); do
    if wait_ubuntu_missing "$SMOKE_CLEANUP_ROOT" &&
      ubuntu_provider_missing "$SMOKE_CLEANUP_ROOT" &&
      wait_windows_disk_missing "$SMOKE_CLEANUP_ROOT" &&
      windows_provider_missing "$SMOKE_CLEANUP_ROOT" &&
      macos_provider_missing "$SMOKE_CLEANUP_ROOT"; then
      local elapsed=$(( $(date +%s) - start ))
      record_timing "smoke root best-effort cleanup" "$elapsed" "ok"
      log "ok in ${elapsed}s: smoke root best-effort cleanup"
      return 0
    fi
    sleep 1
  done
  local elapsed=$(( $(date +%s) - start ))
  local remnants=()
  local remnant_text
  wait_ubuntu_missing "$SMOKE_CLEANUP_ROOT" || remnants+=("ubuntu-disk")
  ubuntu_provider_missing "$SMOKE_CLEANUP_ROOT" || remnants+=("ubuntu-provider")
  wait_windows_disk_missing "$SMOKE_CLEANUP_ROOT" || remnants+=("windows-disk")
  windows_provider_missing "$SMOKE_CLEANUP_ROOT" || remnants+=("windows-provider")
  macos_provider_missing "$SMOKE_CLEANUP_ROOT" || remnants+=("macos-provider")
  remnant_text="none observed"
  if (( ${#remnants[@]} > 0 )); then
    remnant_text="${remnants[*]}"
  fi
  record_timing "smoke root best-effort cleanup" "$elapsed" "warning"
  if (( cleanup_is_current )); then
    die "current smoke dir cleanup incomplete after ${elapsed}s: $remnant_text"
  fi
  log "warning after ${elapsed}s: smoke root still has local remnants: $remnant_text"
}

write_windows_zero_file() {
  local path="$1"
  local bytes="$2"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$Path = Join-Path \$HOME ("Iris Drive\\$path")
\$Parent = Split-Path -Parent \$Path
New-Item -ItemType Directory -Force -Path \$Parent | Out-Null
[System.IO.File]::WriteAllBytes(\$Path, [byte[]]::new($bytes))
REMOTE_PS
}

write_macos_provider_file() {
  local path="$1"
  local content="$2"
  local content_b64
  content_b64="$(base64_arg "$content")"
  ssh "$MACOS_SSH_HOST" 'bash -se' "$path" "$content_b64" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
content_b64="$2"
content="$(python3 -c 'import base64, sys; sys.stdout.write(base64.b64decode(sys.argv[1]).decode("utf-8"))' "$content_b64")"
macos_config_dir() {
  if [[ -n "${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-}" ]]; then
    printf '%s\n' "$IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR/Config"
    return 0
  fi
  local candidate
  for candidate in \
    "$HOME"/Library/Group\ Containers/*.to.iris.drive/Iris\ Drive\ Dev/Config \
    "$HOME"/Library/Group\ Containers/group.to.iris.drive/Iris\ Drive\ Dev/Config \
    "$HOME"/Library/Containers/to.iris.drive.macos/Data/Library/Application\ Support/Iris\ Drive\ Dev/Config
  do
    [[ -d "$candidate" ]] || continue
    [[ -f "$candidate/key" || -f "$candidate/config.json" ]] || continue
    printf '%s\n' "$candidate"
    return 0
  done
  return 1
}
config_dir="$(macos_config_dir)"
tmp="$(mktemp -t iris-drive-macos-provider-write)"
trap 'rm -f "$tmp"' EXIT
printf '%s\n' "$content" > "$tmp"
"$HOME/src/iris-drive/target/debug/idrive" --config-dir "$config_dir" provider write "$path" "$tmp" >/dev/null
REMOTE_SH
}

write_macos_visible_file() {
  local path="$1"
  local content="$2"
  local content_b64
  content_b64="$(base64_arg "$content")"
  ssh "$MACOS_SSH_HOST" 'bash -se' "$path" "$content_b64" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
content_b64="$2"
cloud_root="$HOME/Library/CloudStorage"
drive_root=""
if [[ -d "$cloud_root" ]]; then
  while IFS= read -r candidate; do
    drive_root="$candidate"
    break
  done < <(find "$cloud_root" -maxdepth 1 -type d -name 'IrisDrive*' -print 2>/dev/null | sort)
fi
[[ -n "$drive_root" ]] || {
  echo "macOS visible IrisDrive root not found" >&2
  exit 1
}
target="$drive_root/$path"
mkdir -p "$(dirname "$target")"
python3 - "$content_b64" "$target" <<'PY'
import base64
import sys

content = base64.b64decode(sys.argv[1]).decode("utf-8")
with open(sys.argv[2], "w", encoding="utf-8") as handle:
    handle.write(content + "\n")
PY
REMOTE_SH
}

write_macos_visible_zero_file() {
  local path="$1"
  local bytes="$2"
  ssh "$MACOS_SSH_HOST" 'bash -se' "$path" "$bytes" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
bytes="$2"
cloud_root="$HOME/Library/CloudStorage"
drive_root=""
if [[ -d "$cloud_root" ]]; then
  while IFS= read -r candidate; do
    drive_root="$candidate"
    break
  done < <(find "$cloud_root" -maxdepth 1 -type d -name 'IrisDrive*' -print 2>/dev/null | sort)
fi
[[ -n "$drive_root" ]] || {
  echo "macOS visible IrisDrive root not found" >&2
  exit 1
}
target="$drive_root/$path"
mkdir -p "$(dirname "$target")"
head -c "$bytes" /dev/zero >"$target"
REMOTE_SH
}

delete_macos_provider_path() {
  local path="$1"
  macos_idrive_json provider delete "$path" >/dev/null
}

delete_windows_provider_path() {
  local path="$1"
  win_ps <<REMOTE_PS
\$ErrorActionPreference = "Stop"
\$IrisRepo = Join-Path \$HOME "src\\iris-drive"
\$Idrive = Join-Path \$IrisRepo "windows\\bin\\Debug\\net8.0-windows\\win-x64\\publish\\idrive.exe"
if (-not (Test-Path \$Idrive)) {
  \$Idrive = Join-Path \$IrisRepo "target\\debug\\idrive.exe"
}
\$ConfigDir = Join-Path \$env:APPDATA "iris-drive"
& \$Idrive --config-dir \$ConfigDir provider delete "$path" | Out-Null
REMOTE_PS
}

macos_app_log_line_count() {
  ssh "$MACOS_SSH_HOST" 'test -f /tmp/iris-drive-macos-app.err && wc -l < /tmp/iris-drive-macos-app.err || echo 0'
}

macos_log_has_fileprovider_signal_after() {
  local before="$1"
  ssh "$MACOS_SSH_HOST" "tail -n +$((before + 1)) /tmp/iris-drive-macos-app.err 2>/dev/null || true" \
    | grep -F "Iris Drive FileProvider signal working set ok" >/dev/null
}

ubuntu_start_directory_monitor() {
  local dir="$1"
  local token="$2"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$dir" "$token" <<'REMOTE_SH'
set -Eeuo pipefail
dir="$1"
token="$2"
target="$HOME/Iris Drive/$dir"
mkdir -p "$target"
find "$target" -maxdepth 1 -mindepth 1 >/dev/null 2>&1 || true
log="/tmp/iris-drive-gio-monitor-$token.log"
pidfile="/tmp/iris-drive-gio-monitor-$token.pid"
rm -f "$log" "$pidfile"
(timeout 90 gio monitor "$target" >"$log" 2>&1 & echo $! >"$pidfile")
REMOTE_SH
}

ubuntu_monitor_saw_any() {
  local token="$1"
  shift
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$token" "$@" <<'REMOTE_SH'
set -Eeuo pipefail
token="$1"
shift
for needle in "$@"; do
  grep -F "$needle" "/tmp/iris-drive-gio-monitor-$token.log" >/dev/null && exit 0
done
exit 1
REMOTE_SH
}

ubuntu_stop_directory_monitor() {
  local token="$1"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$token" <<'REMOTE_SH' || true
set -Eeuo pipefail
token="$1"
pidfile="/tmp/iris-drive-gio-monitor-$token.pid"
if [[ -f "$pidfile" ]]; then
  kill "$(cat "$pidfile")" 2>/dev/null || true
fi
REMOTE_SH
}

ubuntu_visible_manifest() {
  local dir="$1"
  ssh "$UBUNTU_SSH_HOST" 'bash -se' "$dir" <<'REMOTE_SH'
set -Eeuo pipefail
dir="$1"
root="$HOME/Iris Drive/$dir"
python3 - "$root" <<'PY'
import hashlib
import json
import os
import sys

root = sys.argv[1]
ignored = {".DS_Store", "Thumbs.db", "desktop.ini", ".iris-drive-refresh", "iris-drive-refresh"}
if not os.path.isdir(root):
    raise SystemExit(1)
entries = []
for current, dirs, files in os.walk(root):
    dirs[:] = sorted(name for name in dirs if name not in ignored)
    for name in dirs:
        path = os.path.relpath(os.path.join(current, name), root).replace(os.sep, "/")
        entries.append({"path": path, "kind": "directory", "size": 0, "sha256": None})
    for name in sorted(files):
        if name in ignored:
            continue
        full = os.path.join(current, name)
        path = os.path.relpath(full, root).replace(os.sep, "/")
        digest = hashlib.sha256()
        with open(full, "rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        entries.append({
            "path": path,
            "kind": "file",
            "size": os.path.getsize(full),
            "sha256": digest.hexdigest(),
        })
entries.sort(key=lambda entry: entry["path"])
print(json.dumps({"entries": entries}, sort_keys=True))
PY
REMOTE_SH
}

macos_visible_manifest() {
  local dir="$1"
  ssh "$MACOS_SSH_HOST" 'bash -se' "$dir" <<'REMOTE_SH'
set -Eeuo pipefail
dir="$1"
python3 - "$HOME/Library/CloudStorage" "$dir" <<'PY'
import hashlib
import json
import os
import sys

cloud_root = sys.argv[1]
relative = sys.argv[2]
ignored = {".DS_Store", "Thumbs.db", "desktop.ini", ".iris-drive-refresh", "iris-drive-refresh"}
roots = []
if os.path.isdir(cloud_root):
    roots = [
        os.path.join(cloud_root, name)
        for name in sorted(os.listdir(cloud_root))
        if name.startswith("IrisDrive")
    ]
for drive_root in roots:
    root = os.path.join(drive_root, relative)
    if not os.path.isdir(root):
        continue
    entries = []
    for current, dirs, files in os.walk(root):
        dirs[:] = sorted(name for name in dirs if name not in ignored)
        for name in dirs:
            path = os.path.relpath(os.path.join(current, name), root).replace(os.sep, "/")
            entries.append({"path": path, "kind": "directory", "size": 0, "sha256": None})
        for name in sorted(files):
            if name in ignored:
                continue
            full = os.path.join(current, name)
            path = os.path.relpath(full, root).replace(os.sep, "/")
            digest = hashlib.sha256()
            with open(full, "rb") as handle:
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(chunk)
            entries.append({
                "path": path,
                "kind": "file",
                "size": os.path.getsize(full),
                "sha256": digest.hexdigest(),
            })
    entries.sort(key=lambda entry: entry["path"])
    print(json.dumps({"entries": entries}, sort_keys=True))
    raise SystemExit(0)
raise SystemExit(1)
PY
REMOTE_SH
}

windows_visible_manifest() {
  local dir="$1"
  win_ps <<REMOTE_PS | tr -d '\r'
\$ErrorActionPreference = "Stop"
\$Root = Join-Path \$HOME ("Iris Drive\\$dir")
if (-not (Test-Path -LiteralPath \$Root -PathType Container)) {
  exit 1
}
\$Ignored = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
@(".DS_Store", "Thumbs.db", "desktop.ini", ".iris-drive-refresh", "iris-drive-refresh") | ForEach-Object { [void]\$Ignored.Add(\$_) }
\$Entries = @()
Get-ChildItem -LiteralPath \$Root -Force -Recurse | Sort-Object FullName | ForEach-Object {
  if (\$Ignored.Contains(\$_.Name)) {
    return
  }
  \$Relative = \$_.FullName.Substring(\$Root.Length).TrimStart([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar).Replace("\\", "/")
  if ([string]::IsNullOrWhiteSpace(\$Relative)) {
    return
  }
  if (\$_.PSIsContainer) {
    \$Entries += [pscustomobject]@{
      path = \$Relative
      kind = "directory"
      size = 0
      sha256 = \$null
    }
  } else {
    \$Hash = (Get-FileHash -LiteralPath \$_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    \$Entries += [pscustomobject]@{
      path = \$Relative
      kind = "file"
      size = \$_.Length
      sha256 = \$Hash
    }
  }
}
[pscustomobject]@{ entries = @(\$Entries) } | ConvertTo-Json -Depth 5 -Compress
REMOTE_PS
}

visible_smoke_dir_matches() {
  local dir="$1"
  shift
  local expected_json
  local ubuntu_json
  local macos_json
  local windows_json
  expected_json="$(python3 - "$@" <<'PY'
import hashlib
import json
import sys

args = sys.argv[1:]
if len(args) % 2:
    raise SystemExit("expected path/content pairs")
entries = []
for path, content in zip(args[0::2], args[1::2]):
    data = (content + "\n").encode()
    entries.append({
        "path": path,
        "kind": "file",
        "size": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    })
entries.sort(key=lambda entry: entry["path"])
print(json.dumps({"entries": entries}, sort_keys=True))
PY
)"
  ubuntu_json="$(ubuntu_visible_manifest "$dir")" || return 1
  macos_json="$(macos_visible_manifest "$dir")" || return 1
  windows_json="$(windows_visible_manifest "$dir")" || return 1
  EXPECTED_JSON="$expected_json" \
    UBUNTU_JSON="$ubuntu_json" \
    MACOS_JSON="$macos_json" \
    WINDOWS_JSON="$windows_json" \
    python3 <<'PY'
import json
import os
import sys

expected = json.loads(os.environ["EXPECTED_JSON"])
manifests = {
    "ubuntu": json.loads(os.environ["UBUNTU_JSON"]),
    "macos": json.loads(os.environ["MACOS_JSON"]),
    "windows": json.loads(os.environ["WINDOWS_JSON"]),
}

for name, manifest in manifests.items():
    conflicts = [
        entry["path"]
        for entry in manifest.get("entries", [])
        if "(conflict from " in entry.get("path", "")
    ]
    if conflicts:
        raise SystemExit(f"{name} contains unexpected conflict files: {conflicts}")
    if manifest != expected:
        raise SystemExit(
            f"{name} visible manifest differs: expected={expected} actual={manifest}"
        )
PY
}

visible_smoke_dir_converges_with_paths() {
  local dir="$1"
  shift
  local expected_paths_json
  local ubuntu_json
  local macos_json
  local windows_json
  expected_paths_json="$(python3 - "$@" <<'PY'
import json
import sys

print(json.dumps(sorted(sys.argv[1:])))
PY
)"
  ubuntu_json="$(ubuntu_visible_manifest "$dir")" || return 1
  macos_json="$(macos_visible_manifest "$dir")" || return 1
  windows_json="$(windows_visible_manifest "$dir")" || return 1
  EXPECTED_PATHS_JSON="$expected_paths_json" \
    UBUNTU_JSON="$ubuntu_json" \
    MACOS_JSON="$macos_json" \
    WINDOWS_JSON="$windows_json" \
    python3 <<'PY'
import json
import os

expected_paths = set(json.loads(os.environ["EXPECTED_PATHS_JSON"]))
manifests = {
    "ubuntu": json.loads(os.environ["UBUNTU_JSON"]),
    "macos": json.loads(os.environ["MACOS_JSON"]),
    "windows": json.loads(os.environ["WINDOWS_JSON"]),
}

baseline_name = "ubuntu"
baseline = manifests[baseline_name]
for name, manifest in manifests.items():
    conflicts = [
        entry["path"]
        for entry in manifest.get("entries", [])
        if "(conflict from " in entry.get("path", "")
    ]
    if conflicts:
        raise SystemExit(f"{name} contains unexpected conflict files: {conflicts}")
    if manifest != baseline:
        raise SystemExit(
            f"{name} visible manifest differs from {baseline_name}: baseline={baseline} actual={manifest}"
        )

actual_paths = {entry["path"] for entry in baseline.get("entries", [])}
missing = sorted(expected_paths - actual_paths)
if missing:
    raise SystemExit(f"visible manifest is missing expected stress paths: {missing}")
PY
}

heavy_projection_manifest_matches() {
  local dir="$1"
  local file_count="$2"
  local large_bytes="$3"
  local run_id="$4"
  local ubuntu_json
  local macos_json
  local windows_json
  ubuntu_json="$(ubuntu_visible_manifest "$dir")" || return 1
  macos_json="$(macos_visible_manifest "$dir")" || return 1
  windows_json="$(windows_visible_manifest "$dir")" || return 1
  FILE_COUNT="$file_count" \
    LARGE_BYTES="$large_bytes" \
    RUN_ID="$run_id" \
    UBUNTU_JSON="$ubuntu_json" \
    MACOS_JSON="$macos_json" \
    WINDOWS_JSON="$windows_json" \
    python3 <<'PY'
import hashlib
import json
import os

file_count = int(os.environ["FILE_COUNT"])
large_bytes = int(os.environ["LARGE_BYTES"])
run_id = os.environ["RUN_ID"]
zero_digest = hashlib.sha256(b"\0" * large_bytes).hexdigest()

entries = []
directories = set()

def add_file(path, data=None, size=None, sha256=None):
    parts = path.split("/")[:-1]
    for index in range(1, len(parts) + 1):
        directories.add("/".join(parts[:index]))
    if data is not None:
        size = len(data)
        sha256 = hashlib.sha256(data).hexdigest()
    entries.append({
        "path": path,
        "kind": "file",
        "size": size,
        "sha256": sha256,
    })

for index in range(1, file_count + 1):
    suffix = f"{index:03d}"
    add_file(f"ubuntu/{suffix}.txt", f"stress ubuntu {suffix} {run_id}\n".encode())
    add_file(f"windows/{suffix}.txt", f"stress windows {suffix} {run_id}\n".encode())
    add_file(f"macos/{suffix}.txt", f"stress macos {suffix} {run_id}\n".encode())

add_file("large/ubuntu-zero.bin", size=large_bytes, sha256=zero_digest)
add_file("large/windows-zero.bin", size=large_bytes, sha256=zero_digest)
add_file("large/macos-zero.bin", size=large_bytes, sha256=zero_digest)

expected_entries = [
    {"path": path, "kind": "directory", "size": 0, "sha256": None}
    for path in directories
]
expected_entries.extend(entries)
expected = {"entries": sorted(expected_entries, key=lambda entry: entry["path"])}

manifests = {
    "ubuntu": json.loads(os.environ["UBUNTU_JSON"]),
    "macos": json.loads(os.environ["MACOS_JSON"]),
    "windows": json.loads(os.environ["WINDOWS_JSON"]),
}

for name, manifest in manifests.items():
    conflicts = [
        entry["path"]
        for entry in manifest.get("entries", [])
        if "(conflict from " in entry.get("path", "")
    ]
    if conflicts:
        raise SystemExit(f"{name} contains unexpected conflict files: {conflicts}")
    if manifest != expected:
        raise SystemExit(
            f"{name} heavy projection manifest differs: expected={expected} actual={manifest}"
        )
PY
}

check_native_status_summaries() {
  log "checking native status summaries report files, bytes, and devices"
  local ubuntu_status
  local macos_status
  local windows_status
  ubuntu_status="$(ssh "$UBUNTU_SSH_HOST" '"$HOME/src/iris-drive/target/debug/idrive" status')"
  macos_status="$(macos_idrive_json status)"
  windows_status="$(win_idrive_json status)"
  STATUS_UBUNTU="$ubuntu_status" STATUS_MACOS="$macos_status" STATUS_WINDOWS="$windows_status" python3 <<'PY'
import json
import os

statuses = {
    "ubuntu": json.loads(os.environ["STATUS_UBUNTU"]),
    "macos": json.loads(os.environ["STATUS_MACOS"]),
    "windows": json.loads(os.environ["STATUS_WINDOWS"]),
}
errors = []
for name, status in statuses.items():
    hashtree = status.get("hashtree", {})
    network = status.get("network", {})
    file_count = hashtree.get("file_count") or hashtree.get("top_level_entries") or 0
    visible_file_bytes = hashtree.get("visible_file_bytes") or 0
    authorized_devices = network.get("authorized_device_count") or 0
    if file_count <= 0:
        errors.append(f"{name} status file_count={file_count}")
    if visible_file_bytes <= 0:
        errors.append(f"{name} status visible_file_bytes={visible_file_bytes}")
    if authorized_devices < 3:
        errors.append(f"{name} status authorized_device_count={authorized_devices}")
if errors:
    raise SystemExit("; ".join(errors))
PY
}

run_sync_smoke() {
  local windows_file="$SMOKE_DIR/from-windows.txt"
  local ubuntu_file="$SMOKE_DIR/from-ubuntu-placeholder.txt"
  local macos_file="$SMOKE_DIR/from-macos-provider.txt"
  local macos_delete_file="$SMOKE_DIR/delete-from-macos-provider.txt"
  local windows_rename_src="$SMOKE_DIR/windows-rename-src.txt"
  local windows_rename_dst="$SMOKE_DIR/windows-rename-dst.txt"
  local live_file="$SMOKE_DIR/live-from-windows.txt"
  local windows_live_file="$SMOKE_DIR/live-from-ubuntu-for-windows.txt"
  local windows_projection_guard_ubuntu="$SMOKE_DIR/windows-projection-guard-ubuntu.txt"
  local windows_projection_guard_macos="$SMOKE_DIR/windows-projection-guard-macos.txt"
  local windows_projection_guard_local="$SMOKE_DIR/windows-projection-guard-local.txt"
  local ubuntu_edit_windows_hydrated="$SMOKE_DIR/ubuntu-edit-windows-hydrated.txt"
  local monitor_token="${RUN_ID//[^A-Za-z0-9]/}-ubuntu-live"
  local ubuntu_delete_monitor_token="${RUN_ID//[^A-Za-z0-9]/}-ubuntu-delete"
  local windows_monitor_token="${RUN_ID//[^A-Za-z0-9]/}-windows-live"

  log "checking Windows-origin create then Ubuntu-origin delete"
  write_windows_file "$windows_file" "from windows $RUN_ID"
  wait_for "Windows file reaches Ubuntu" "$SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_file_has "$windows_file"
  wait_for "Windows file reaches macOS provider" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    macos_provider_has "$windows_file"
  wait_for "Windows file reaches macOS visible FileProvider folder" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    macos_visible_drive_has "$windows_file"
  delete_ubuntu_path "$windows_file"
  wait_for_quiet "Ubuntu delete removes Windows disk file" \
    "$SYNC_WAIT_TIMEOUT" "$SYNC_QUIET_POLL_INTERVAL" \
    wait_windows_disk_missing "$windows_file"
  wait_for "Ubuntu delete removes Windows provider file" "$SYNC_WAIT_TIMEOUT" \
    windows_provider_missing "$windows_file"

  log "checking Windows placeholder delete publishes back to Ubuntu"
  write_ubuntu_file "$ubuntu_file" "from ubuntu $RUN_ID"
  wait_for "Ubuntu file reaches Windows disk" "$SYNC_WAIT_TIMEOUT" \
    wait_windows_disk_has "$ubuntu_file"
  wait_for "Ubuntu file is represented as a Windows Cloud Files placeholder" \
    "$SYNC_WAIT_TIMEOUT" wait_windows_disk_reparse "$ubuntu_file"
  delete_windows_path "$ubuntu_file"
  wait_for "Windows placeholder delete removes Ubuntu file" \
    "$WINDOWS_PLACEHOLDER_DELETE_SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_missing "$ubuntu_file"
  wait_for "Windows placeholder delete removes Windows provider file" \
    "$WINDOWS_PLACEHOLDER_DELETE_SYNC_WAIT_TIMEOUT" windows_provider_missing "$ubuntu_file"
  wait_for "Windows placeholder delete removes macOS provider file" \
    "$WINDOWS_PLACEHOLDER_DELETE_SYNC_WAIT_TIMEOUT" macos_provider_missing "$ubuntu_file"

  log "checking macOS-origin provider create then Windows-origin delete"
  write_macos_provider_file "$macos_file" "from macos $RUN_ID"
  wait_for "macOS provider file reaches Ubuntu" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_file_has "$macos_file"
  wait_for "macOS provider file reaches Windows disk" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    wait_windows_disk_has "$macos_file"
  wait_for "macOS provider file is represented as a Windows Cloud Files placeholder" \
    "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" wait_windows_disk_reparse "$macos_file"
  delete_windows_path "$macos_file"
  wait_for "Windows delete removes macOS provider file" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    macos_provider_missing "$macos_file"
  wait_for "Windows delete removes Ubuntu copy of macOS file" \
    "$WINDOWS_PLACEHOLDER_DELETE_SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_missing "$macos_file"

  log "checking Windows local create does not hide existing placeholders"
  write_ubuntu_file "$windows_projection_guard_ubuntu" "projection guard ubuntu $RUN_ID"
  write_macos_provider_file "$windows_projection_guard_macos" "projection guard macos $RUN_ID"
  wait_for "Ubuntu projection guard reaches Windows disk" "$SYNC_WAIT_TIMEOUT" \
    wait_windows_disk_has "$windows_projection_guard_ubuntu"
  wait_for "Ubuntu projection guard is a Windows Cloud Files placeholder" \
    "$SYNC_WAIT_TIMEOUT" wait_windows_disk_reparse "$windows_projection_guard_ubuntu"
  wait_for "macOS projection guard reaches Windows disk" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    wait_windows_disk_has "$windows_projection_guard_macos"
  wait_for "macOS projection guard is a Windows Cloud Files placeholder" \
    "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" wait_windows_disk_reparse "$windows_projection_guard_macos"
  windows_projection_stays_visible_during_local_create \
    "$windows_projection_guard_local" \
    "$windows_projection_guard_ubuntu" \
    "$windows_projection_guard_macos"
  wait_for "Windows projection guard local create reaches Ubuntu" "$SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_file_has "$windows_projection_guard_local"
  wait_for "Windows projection guard local create reaches macOS provider" \
    "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" macos_provider_has "$windows_projection_guard_local"

  log "checking Ubuntu edit replaces hydrated Windows projection bytes"
  write_ubuntu_file "$ubuntu_edit_windows_hydrated" "old ubuntu bytes $RUN_ID"
  wait_for "Ubuntu edit baseline reaches Windows bytes" "$SYNC_WAIT_TIMEOUT" \
    wait_windows_file_has_content "$ubuntu_edit_windows_hydrated" "old ubuntu bytes $RUN_ID"
  write_ubuntu_file "$ubuntu_edit_windows_hydrated" "new ubuntu bytes $RUN_ID"
  wait_for "Ubuntu edit updates hydrated Windows bytes" "$SYNC_WAIT_TIMEOUT" \
    wait_windows_file_has_content "$ubuntu_edit_windows_hydrated" "new ubuntu bytes $RUN_ID"

  log "checking Ubuntu-origin create then macOS-origin provider delete"
  write_ubuntu_file "$macos_delete_file" "delete from macos $RUN_ID"
  wait_for "Ubuntu file reaches macOS provider" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    macos_provider_has "$macos_delete_file"
  wait_for "Ubuntu file reaches Windows disk before macOS delete" "$SYNC_WAIT_TIMEOUT" \
    wait_windows_disk_has "$macos_delete_file"
  ubuntu_start_directory_monitor "$SMOKE_DIR" "$ubuntu_delete_monitor_token"
  delete_macos_provider_path "$macos_delete_file"
  wait_for "macOS provider delete removes Ubuntu file" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_missing "$macos_delete_file"
  wait_for "Ubuntu directory monitor wakes for macOS delete" "$SYNC_WAIT_TIMEOUT" \
    ubuntu_monitor_saw_any "$ubuntu_delete_monitor_token" \
      "$(basename "$macos_delete_file")" "iris-drive-refresh" ".iris-drive-refresh"
  ubuntu_stop_directory_monitor "$ubuntu_delete_monitor_token"
  wait_for_quiet "macOS provider delete removes Windows disk file" \
    "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" "$SYNC_QUIET_POLL_INTERVAL" \
    wait_windows_disk_missing "$macos_delete_file"
  wait_for "macOS provider delete removes Windows provider file" \
    "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" windows_provider_missing "$macos_delete_file"

  log "checking Windows-origin rename/create updates other live providers"
  write_windows_file "$windows_rename_src" "rename from windows $RUN_ID"
  wait_for "Windows rename source reaches Ubuntu" "$SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_file_has "$windows_rename_src"
  wait_for "Windows rename source reaches macOS provider" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    macos_provider_has "$windows_rename_src"
  local macos_log_before
  macos_log_before="$(macos_app_log_line_count)"
  rename_windows_path "$windows_rename_src" "$windows_rename_dst"
  wait_for "Windows rename destination reaches Ubuntu" "$SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_file_has "$windows_rename_dst"
  wait_for "Windows rename source disappears from Ubuntu" "$SYNC_WAIT_TIMEOUT" \
    wait_ubuntu_missing "$windows_rename_src"
  wait_for "Windows rename destination reaches macOS provider" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    macos_provider_has "$windows_rename_dst"
  wait_for "Windows rename source disappears from macOS provider" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" \
    macos_provider_missing "$windows_rename_src"
  wait_for "macOS FileProvider was signaled after Windows rename" "$SYNC_WAIT_TIMEOUT" \
    macos_log_has_fileprovider_signal_after "$macos_log_before"

  log "checking Linux directory monitor sees a remote Windows create"
  ubuntu_start_directory_monitor "$SMOKE_DIR" "$monitor_token"
  write_windows_file "$live_file" "live from windows $RUN_ID"
  wait_for "Ubuntu directory monitor wakes for Windows create" "$SYNC_WAIT_TIMEOUT" \
    ubuntu_monitor_saw_any "$monitor_token" "$(basename "$live_file")" "iris-drive-refresh" ".iris-drive-refresh"
  wait_for "Ubuntu live create is visible after monitor wake" 15 wait_ubuntu_file_has "$live_file"
  ubuntu_stop_directory_monitor "$monitor_token"
  wait_for "Windows live create reaches macOS provider" "$MACOS_PROVIDER_SYNC_WAIT_TIMEOUT" macos_provider_has "$live_file"

  log "checking Windows directory monitor sees a remote Ubuntu create"
  windows_start_directory_monitor "$SMOKE_DIR" "$windows_monitor_token"
  write_ubuntu_file "$windows_live_file" "live from ubuntu $RUN_ID"
  wait_for "Windows directory monitor wakes or disk refreshes for Ubuntu create" \
    "$SYNC_WAIT_TIMEOUT" windows_monitor_saw_any_or_disk_has \
    "$windows_monitor_token" "$windows_live_file"
  wait_for "Ubuntu live create is visible on Windows disk" 15 wait_windows_disk_has "$windows_live_file"
  windows_stop_directory_monitor "$windows_monitor_token"

  log "checking native visible directories converge without conflict fan-out"
  wait_for "native visible smoke directory manifests converge" "$SYNC_WAIT_TIMEOUT" \
    visible_smoke_dir_matches "$SMOKE_DIR" \
    "$(basename "$live_file")" "live from windows $RUN_ID" \
    "$(basename "$windows_live_file")" "live from ubuntu $RUN_ID" \
    "$(basename "$windows_projection_guard_ubuntu")" "projection guard ubuntu $RUN_ID" \
    "$(basename "$windows_projection_guard_macos")" "projection guard macos $RUN_ID" \
    "$(basename "$windows_projection_guard_local")" "projection guard windows $RUN_ID" \
    "$(basename "$ubuntu_edit_windows_hydrated")" "new ubuntu bytes $RUN_ID" \
    "$(basename "$windows_rename_dst")" "rename from windows $RUN_ID"
  check_native_status_summaries

  log "checking heavy native projection stress converges on all OS surfaces"
  local stress_dir="$SMOKE_DIR/heavy-projection"
  local i
  for i in $(seq 1 "$PROJECTION_STRESS_FILES"); do
    local suffix
    suffix="$(printf "%03d" "$i")"
    write_ubuntu_file "$stress_dir/ubuntu/$suffix.txt" "stress ubuntu $suffix $RUN_ID"
    write_windows_file "$stress_dir/windows/$suffix.txt" "stress windows $suffix $RUN_ID"
    write_macos_visible_file "$stress_dir/macos/$suffix.txt" "stress macos $suffix $RUN_ID"
  done
  write_ubuntu_zero_file "$stress_dir/large/ubuntu-zero.bin" "$PROJECTION_STRESS_LARGE_BYTES"
  write_windows_zero_file "$stress_dir/large/windows-zero.bin" "$PROJECTION_STRESS_LARGE_BYTES"
  write_macos_visible_zero_file "$stress_dir/large/macos-zero.bin" "$PROJECTION_STRESS_LARGE_BYTES"
  wait_for "heavy native projection manifests converge with expected bytes" "$SYNC_WAIT_TIMEOUT" \
    heavy_projection_manifest_matches \
    "$stress_dir" "$PROJECTION_STRESS_FILES" "$PROJECTION_STRESS_LARGE_BYTES" "$RUN_ID"

  delete_ubuntu_path "$SMOKE_DIR" || true
}

run_macos_open_smoke() {
  case "${IRIS_DRIVE_DEV_VM_SMOKE_MACOS_UI:-1}" in
    1|true|TRUE|yes|YES|on|ON) ;;
    *) log "skipping macOS UI smoke"; return 0 ;;
  esac

  log "requesting macOS Open Drive Folder"
  local before
  before="$(ssh "$MACOS_SSH_HOST" 'test -f /tmp/iris-drive-macos-app.err && wc -l < /tmp/iris-drive-macos-app.err || echo 0')"
  ssh "$MACOS_SSH_HOST" '/usr/bin/swift -' <<'REMOTE_SWIFT' >/dev/null
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
    recent="$(ssh "$MACOS_SSH_HOST" "tail -n +$((before + 1)) /tmp/iris-drive-macos-app.err 2>/dev/null || true")"
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
cleanup_previous_smoke_root
run_sync_smoke
run_macos_open_smoke
log "timings written to $TIMINGS_FILE"
log "ok"
