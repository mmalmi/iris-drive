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
SMOKE_ROOT="codex-lab-smoke"
SMOKE_DIR="$SMOKE_ROOT/$RUN_ID"
TIMINGS_FILE="${IRIS_DRIVE_DEV_VM_SMOKE_TIMINGS_FILE:-$ROOT/target/e2e-3vms-$RUN_ID-timings.jsonl}"
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

MACOS_REMOTE="$(remote_or_die IRIS_DRIVE_DEV_VM_MACOS_REMOTE macos)"
UBUNTU_REMOTE="$(remote_or_die IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE ubuntu)"
WINDOWS_REMOTE="$(remote_or_die IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE windows)"

ps_single_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/''/g")"
}

win_ps() {
  ssh "$WINDOWS_REMOTE" \
    'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -'
}

win_idrive_json() {
  local args=("$@")
  local ps_args=""
  local arg
  if [[ ${#args[@]} -eq 1 && "${args[0]}" == "status" ]]; then
    ssh "$WINDOWS_REMOTE" 'cmd /d /s /c ""%USERPROFILE%\src\iris-drive\windows\bin\Debug\net8.0-windows\win-x64\publish\idrive.exe" --config-dir "%APPDATA%\iris-drive" status"'
    return
  fi
  if [[ ${#args[@]} -eq 2 && "${args[0]}" == "provider" && "${args[1]}" == "list" ]]; then
    ssh "$WINDOWS_REMOTE" 'cmd /d /s /c ""%USERPROFILE%\src\iris-drive\windows\bin\Debug\net8.0-windows\win-x64\publish\idrive.exe" --config-dir "%APPDATA%\iris-drive" provider list"'
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
  ssh "$MACOS_REMOTE" 'bash -se' "${args[@]}" <<'REMOTE_SH'
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
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
"$HOME/src/iris-drive/target/debug/idrive" provider list \
  | python3 -c 'import json, sys; needle = sys.argv[1]; data = json.load(sys.stdin); raise SystemExit(0 if any(e.get("path") == needle for e in data.get("entries", [])) else 1)' "$path"
REMOTE_SH
}

macos_provider_has() {
  local path="$1"
  macos_idrive_json provider list \
    | python3 -c 'import json, sys; needle = sys.argv[1]; data = json.load(sys.stdin); raise SystemExit(0 if any(e.get("path") == needle for e in data.get("entries", [])) else 1)' "$path"
}

macos_visible_drive_has() {
  local path="$1"
  ssh "$MACOS_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
enumerate_parent_chain() {
  local root="$1"
  local relative="$2"
  local parent
  local current
  local part

  parent="$(dirname "$relative")"
  current="$root"
  /bin/ls -la "$current" >/dev/null 2>&1 || true
  [[ "$parent" != "." ]] || return 0
  IFS='/' read -r -a parts <<< "$parent"
  for part in "${parts[@]}"; do
    [[ -n "$part" ]] || continue
    current="$current/$part"
    /bin/ls -la "$current" >/dev/null 2>&1 || true
  done
}

while IFS= read -r root; do
  [[ -n "$root" ]] || continue
  enumerate_parent_chain "$root" "$path"
  if [[ -e "$root/$path" ]]; then
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
if (Test-Path -LiteralPath \$Path) {
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

wait_ubuntu_file_has() {
  local path="$1"
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
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
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
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
  assert_no_ignored_provider_paths ubuntu "$(ssh "$UBUNTU_REMOTE" '"$HOME/src/iris-drive/target/debug/idrive" provider list')"
  assert_no_ignored_provider_paths macos "$(macos_idrive_json provider list)"
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
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" "$content" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
content="$2"
if command -v timeout >/dev/null 2>&1; then
  timeout 10s mkdir -p "$(dirname "$HOME/Iris Drive/$path")"
  printf '%s\n' "$content" | timeout 10s tee "$HOME/Iris Drive/$path" >/dev/null
else
  mkdir -p "$(dirname "$HOME/Iris Drive/$path")"
  printf '%s\n' "$content" > "$HOME/Iris Drive/$path"
fi
REMOTE_SH
}

delete_ubuntu_path() {
  local path="$1"
  ssh "$UBUNTU_REMOTE" 'bash -se' "$path" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
if command -v timeout >/dev/null 2>&1; then
  timeout 10s rm -rf "$HOME/Iris Drive/$path"
else
  rm -rf "$HOME/Iris Drive/$path"
fi
REMOTE_SH
}

cleanup_previous_smoke_root() {
  case "${IRIS_DRIVE_DEV_VM_SMOKE_CLEAN_ROOT:-1}" in
    1|true|TRUE|yes|YES|on|ON) ;;
    *) log "skipping previous smoke root cleanup"; return 0 ;;
  esac

  log "cleaning previous native smoke root"
  delete_ubuntu_path "$SMOKE_ROOT" || true
  delete_windows_path "$SMOKE_ROOT" || true
  delete_macos_provider_path "$SMOKE_ROOT" || true

  local start
  start="$(date +%s)"
  while (( $(date +%s) - start < 45 )); do
    if wait_ubuntu_missing "$SMOKE_ROOT" &&
      wait_windows_disk_missing "$SMOKE_ROOT" &&
      macos_provider_missing "$SMOKE_ROOT"; then
      local elapsed=$(( $(date +%s) - start ))
      record_timing "previous smoke root best-effort cleanup" "$elapsed" "ok"
      log "ok in ${elapsed}s: previous smoke root best-effort cleanup"
      return 0
    fi
    sleep 1
  done
  local elapsed=$(( $(date +%s) - start ))
  record_timing "previous smoke root best-effort cleanup" "$elapsed" "warning"
  log "warning after ${elapsed}s: previous smoke root still has local remnants"
}

write_macos_provider_file() {
  local path="$1"
  local content="$2"
  ssh "$MACOS_REMOTE" 'bash -se' "$path" "$content" <<'REMOTE_SH'
set -Eeuo pipefail
path="$1"
content="$2"
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

delete_macos_provider_path() {
  local path="$1"
  macos_idrive_json provider delete "$path" >/dev/null
}

macos_app_log_line_count() {
  ssh "$MACOS_REMOTE" 'test -f /tmp/iris-drive-macos-app.err && wc -l < /tmp/iris-drive-macos-app.err || echo 0'
}

macos_log_has_fileprovider_signal_after() {
  local before="$1"
  ssh "$MACOS_REMOTE" "tail -n +$((before + 1)) /tmp/iris-drive-macos-app.err 2>/dev/null || true" \
    | grep -F "Iris Drive FileProvider signal working set ok" >/dev/null
}

ubuntu_start_directory_monitor() {
  local dir="$1"
  local token="$2"
  ssh "$UBUNTU_REMOTE" 'bash -se' "$dir" "$token" <<'REMOTE_SH'
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
  ssh "$UBUNTU_REMOTE" 'bash -se' "$token" "$@" <<'REMOTE_SH'
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
  ssh "$UBUNTU_REMOTE" 'bash -se' "$token" <<'REMOTE_SH' || true
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
  ssh "$UBUNTU_REMOTE" 'bash -se' "$dir" <<'REMOTE_SH'
set -Eeuo pipefail
dir="$1"
root="$HOME/Iris Drive/$dir"
python3 - "$root" <<'PY'
import hashlib
import json
import os
import sys

root = sys.argv[1]
ignored = {".DS_Store", "Thumbs.db", "desktop.ini", ".iris-drive-refresh"}
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
  ssh "$MACOS_REMOTE" 'bash -se' "$dir" <<'REMOTE_SH'
set -Eeuo pipefail
dir="$1"
python3 - "$HOME/Library/CloudStorage" "$dir" <<'PY'
import hashlib
import json
import os
import sys

cloud_root = sys.argv[1]
relative = sys.argv[2]
ignored = {".DS_Store", "Thumbs.db", "desktop.ini", ".iris-drive-refresh"}
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
@(".DS_Store", "Thumbs.db", "desktop.ini", ".iris-drive-refresh") | ForEach-Object { [void]\$Ignored.Add(\$_) }
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

run_sync_smoke() {
  local windows_file="$SMOKE_DIR/from-windows.txt"
  local ubuntu_file="$SMOKE_DIR/from-ubuntu-placeholder.txt"
  local macos_file="$SMOKE_DIR/from-macos-provider.txt"
  local macos_delete_file="$SMOKE_DIR/delete-from-macos-provider.txt"
  local windows_rename_src="$SMOKE_DIR/windows-rename-src.txt"
  local windows_rename_dst="$SMOKE_DIR/windows-rename-dst.txt"
  local live_file="$SMOKE_DIR/live-from-windows.txt"
  local windows_live_file="$SMOKE_DIR/live-from-ubuntu-for-windows.txt"
  local monitor_token="${RUN_ID//[^A-Za-z0-9]/}-ubuntu-live"
  local windows_monitor_token="${RUN_ID//[^A-Za-z0-9]/}-windows-live"

  log "checking Windows-origin create then Ubuntu-origin delete"
  write_windows_file "$windows_file" "from windows $RUN_ID"
  wait_for "Windows file reaches Ubuntu" 60 wait_ubuntu_file_has "$windows_file"
  wait_for "Windows file reaches macOS provider" 60 macos_provider_has "$windows_file"
  wait_for "Windows file reaches macOS visible FileProvider folder" 60 \
    macos_visible_drive_has "$windows_file"
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

  log "checking Windows-origin rename/create updates other live providers"
  write_windows_file "$windows_rename_src" "rename from windows $RUN_ID"
  wait_for "Windows rename source reaches Ubuntu" 60 wait_ubuntu_file_has "$windows_rename_src"
  wait_for "Windows rename source reaches macOS provider" 60 macos_provider_has "$windows_rename_src"
  local macos_log_before
  macos_log_before="$(macos_app_log_line_count)"
  rename_windows_path "$windows_rename_src" "$windows_rename_dst"
  wait_for "Windows rename destination reaches Ubuntu" 75 wait_ubuntu_file_has "$windows_rename_dst"
  wait_for "Windows rename source disappears from Ubuntu" 75 wait_ubuntu_missing "$windows_rename_src"
  wait_for "Windows rename destination reaches macOS provider" 75 macos_provider_has "$windows_rename_dst"
  wait_for "Windows rename source disappears from macOS provider" 75 macos_provider_missing "$windows_rename_src"
  wait_for "macOS FileProvider was signaled after Windows rename" 30 \
    macos_log_has_fileprovider_signal_after "$macos_log_before"

  log "checking Linux directory monitor sees a remote Windows create"
  ubuntu_start_directory_monitor "$SMOKE_DIR" "$monitor_token"
  write_windows_file "$live_file" "live from windows $RUN_ID"
  wait_for "Ubuntu directory monitor wakes for Windows create" 45 \
    ubuntu_monitor_saw_any "$monitor_token" "$(basename "$live_file")" ".iris-drive-refresh"
  wait_for "Ubuntu live create is visible after monitor wake" 15 wait_ubuntu_file_has "$live_file"
  ubuntu_stop_directory_monitor "$monitor_token"
  wait_for "Windows live create reaches macOS provider" 75 macos_provider_has "$live_file"

  log "checking Windows directory monitor sees a remote Ubuntu create"
  windows_start_directory_monitor "$SMOKE_DIR" "$windows_monitor_token"
  write_ubuntu_file "$windows_live_file" "live from ubuntu $RUN_ID"
  wait_for "Windows directory monitor wakes for Ubuntu create" 75 \
    windows_monitor_saw_any "$windows_monitor_token" "$(basename "$windows_live_file")"
  wait_for "Ubuntu live create is visible on Windows disk" 15 wait_windows_disk_has "$windows_live_file"
  windows_stop_directory_monitor "$windows_monitor_token"

  log "checking native visible directories converge without conflict fan-out"
  wait_for "native visible smoke directory manifests converge" 75 \
    visible_smoke_dir_matches "$SMOKE_DIR" \
    "$(basename "$live_file")" "live from windows $RUN_ID" \
    "$(basename "$windows_live_file")" "live from ubuntu $RUN_ID" \
    "$(basename "$windows_rename_dst")" "rename from windows $RUN_ID"

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
cleanup_previous_smoke_root
run_sync_smoke
run_macos_open_smoke
log "timings written to $TIMINGS_FILE"
log "ok"
