#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/cross-vm-e2e.sh --host LABEL=KIND:SSH_HOST [--host ...]

KIND is one of:
  posix     Linux or macOS host reachable by SSH with bash
  windows   Windows host reachable by SSH with PowerShell

The script creates isolated temp config/work directories on every host, links
them into one Iris Drive account, starts real idrive daemons, mutates files,
and waits until every host has the same visible SHA-256 snapshot.

Environment:
  IRIS_DRIVE_E2E_RELAYS          Space-separated relay URLs passed to daemons.
  IRIS_DRIVE_E2E_TIMEOUT_SECS    Convergence timeout per step (default: 180).
  IRIS_DRIVE_E2E_MANY_FILES      Many-file test count (default: 32).
  IRIS_DRIVE_E2E_LARGE_BYTES     Large-file test bytes (default: 262144).
  IRIS_DRIVE_E2E_KEEP            Keep remote temp dirs/daemons when set to 1.
USAGE
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_ID="run-$(date +%Y%m%d%H%M%S)-$$"
TIMEOUT_SECS="${IRIS_DRIVE_E2E_TIMEOUT_SECS:-180}"
POLL_SECS="${IRIS_DRIVE_E2E_POLL_SECS:-3}"
MANY_FILES="${IRIS_DRIVE_E2E_MANY_FILES:-32}"
LARGE_BYTES="${IRIS_DRIVE_E2E_LARGE_BYTES:-262144}"
KEEP="${IRIS_DRIVE_E2E_KEEP:-0}"

declare -a LABELS=()
declare -a KINDS=()
declare -a SSH_HOSTS=()
declare -a BASES=()
declare -a CONFIGS=()
declare -a WORKS=()
declare -a IDRIVES=()
declare -a LOGS=()
declare -a ERRS=()
declare -a PIDS=()
declare -a DAEMON_SSH_PIDS=()

find_label_index() {
  local needle="$1"
  local i
  for i in "${!LABELS[@]}"; do
    if [[ "${LABELS[$i]}" == "$needle" ]]; then
      printf "%s" "$i"
      return 0
    fi
  done
  return 1
}

label_index() {
  local needle="$1"
  local idx
  if idx="$(find_label_index "$needle")"; then
    printf "%s" "$idx"
    return 0
  fi
  echo "unknown host label: $needle" >&2
  exit 1
}

host_value() {
  local label="$1"
  local field="$2"
  local idx
  idx="$(label_index "$label")"
  case "$field" in
    kind) printf "%s" "${KINDS[$idx]}" ;;
    ssh) printf "%s" "${SSH_HOSTS[$idx]}" ;;
    base) printf "%s" "${BASES[$idx]:-}" ;;
    config) printf "%s" "${CONFIGS[$idx]:-}" ;;
    work) printf "%s" "${WORKS[$idx]:-}" ;;
    idrive) printf "%s" "${IDRIVES[$idx]:-}" ;;
    log) printf "%s" "${LOGS[$idx]:-}" ;;
    err) printf "%s" "${ERRS[$idx]:-}" ;;
    pid) printf "%s" "${PIDS[$idx]:-}" ;;
    daemon_ssh_pid) printf "%s" "${DAEMON_SSH_PIDS[$idx]:-}" ;;
    *) echo "unknown host field: $field" >&2; exit 1 ;;
  esac
}

set_host_value() {
  local label="$1"
  local field="$2"
  local value="$3"
  local idx
  idx="$(label_index "$label")"
  case "$field" in
    base) BASES[$idx]="$value" ;;
    config) CONFIGS[$idx]="$value" ;;
    work) WORKS[$idx]="$value" ;;
    idrive) IDRIVES[$idx]="$value" ;;
    log) LOGS[$idx]="$value" ;;
    err) ERRS[$idx]="$value" ;;
    pid) PIDS[$idx]="$value" ;;
    daemon_ssh_pid) DAEMON_SSH_PIDS[$idx]="$value" ;;
    *) echo "unknown mutable host field: $field" >&2; exit 1 ;;
  esac
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)
      [[ $# -ge 2 ]] || { echo "--host needs a value" >&2; exit 2; }
      spec="$2"
      shift 2
      [[ "$spec" == *=*:* ]] || { echo "invalid host spec: $spec" >&2; exit 2; }
      label="${spec%%=*}"
      rest="${spec#*=}"
      kind="${rest%%:*}"
      ssh_host="${rest#*:}"
      [[ "$kind" == "posix" || "$kind" == "windows" ]] || {
        echo "invalid host kind for $label: $kind" >&2
        exit 2
      }
      if find_label_index "$label" >/dev/null; then
        echo "duplicate host label: $label" >&2
        exit 2
      fi
      LABELS+=("$label")
      KINDS+=("$kind")
      SSH_HOSTS+=("$ssh_host")
      BASES+=("")
      CONFIGS+=("")
      WORKS+=("")
      IDRIVES+=("")
      LOGS+=("")
      ERRS+=("")
      PIDS+=("")
      DAEMON_SSH_PIDS+=("")
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ${#LABELS[@]} -lt 2 ]]; then
  usage >&2
  echo "at least two --host entries are required" >&2
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

sh_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/'\\\\''/g")"
}

ps_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/''/g")"
}

remote_exec() {
  local label="$1"
  local script="$2"
  local kind
  local ssh_host
  kind="$(host_value "$label" kind)"
  ssh_host="$(host_value "$label" ssh)"
  if [[ "$kind" == "windows" ]]; then
    printf "%s" "$script" | ssh "$ssh_host" 'powershell -NoProfile -ExecutionPolicy Bypass -Command "`$script = [Console]::In.ReadToEnd(); Invoke-Expression `$script"'
  else
    printf "%s" "$script" | ssh "$ssh_host" 'bash -se'
  fi
}

setup_host() {
  local label="$1"
  local kind
  local script meta key value
  kind="$(host_value "$label" kind)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$ErrorActionPreference = 'Stop'
\$label = $(ps_quote "$label")
\$run = $(ps_quote "$RUN_ID")
\$base = Join-Path \$env:TEMP (\"iris-drive-e2e-\$run-\$label\")
if (Test-Path -LiteralPath \$base) { Remove-Item -LiteralPath \$base -Recurse -Force }
\$config = Join-Path \$base 'config'
\$work = Join-Path \$base 'work'
New-Item -ItemType Directory -Force -Path \$config,\$work | Out-Null
\$idrive = Join-Path \$HOME '.cargo\bin\idrive.exe'
if (-not (Test-Path -LiteralPath \$idrive)) {
  \$cmd = Get-Command idrive.exe -ErrorAction SilentlyContinue
  if (\$cmd) { \$idrive = \$cmd.Source }
}
if (-not (Test-Path -LiteralPath \$idrive)) { throw \"idrive.exe not found for \$label\" }
Write-Output \"base=\$base\"
Write-Output \"config=\$config\"
Write-Output \"work=\$work\"
Write-Output \"idrive=\$idrive\"
Write-Output \"log=\$(Join-Path \$base 'daemon.out.log')\"
Write-Output \"err=\$(Join-Path \$base 'daemon.err.log')\"
Write-Output \"pid=\$(Join-Path \$base 'daemon.pid')\"
"
  else
    script="
set -Eeuo pipefail
label=$(sh_quote "$label")
run=$(sh_quote "$RUN_ID")
base=\"\${TMPDIR:-/tmp}/iris-drive-e2e-\${run}-\${label}\"
rm -rf \"\$base\"
mkdir -p \"\$base/config\" \"\$base/work\"
idrive=\"\${IRIS_DRIVE_E2E_IDRIVE:-\$HOME/.cargo/bin/idrive}\"
if [[ ! -x \"\$idrive\" ]]; then
  idrive=\"\$(command -v idrive || true)\"
fi
if [[ -z \"\$idrive\" || ! -x \"\$idrive\" ]]; then
  echo \"idrive not found for \$label\" >&2
  exit 1
fi
printf 'base=%s\n' \"\$base\"
printf 'config=%s\n' \"\$base/config\"
printf 'work=%s\n' \"\$base/work\"
printf 'idrive=%s\n' \"\$idrive\"
printf 'log=%s\n' \"\$base/daemon.out.log\"
printf 'err=%s\n' \"\$base/daemon.err.log\"
printf 'pid=%s\n' \"\$base/daemon.pid\"
"
  fi
  meta="$(remote_exec "$label" "$script")"
  while IFS='=' read -r key value; do
    value="${value%$'\r'}"
    case "$key" in
      base|config|work|idrive|log|err|pid) set_host_value "$label" "$key" "$value" ;;
    esac
  done <<<"$meta"
}

idrive_cmd() {
  local label="$1"
  shift
  local kind
  local idrive
  local config
  local script
  kind="$(host_value "$label" kind)"
  idrive="$(host_value "$label" idrive)"
  config="$(host_value "$label" config)"
  if [[ "$kind" == "windows" ]]; then
    local args=""
    local arg
    for arg in "$@"; do
      args+=", $(ps_quote "$arg")"
    done
    script="
\$ErrorActionPreference = 'Stop'
\$idrive = $(ps_quote "$idrive")
\$config = $(ps_quote "$config")
\$idriveArgs = @('--config-dir', \$config$args)
& \$idrive @idriveArgs
exit \$LASTEXITCODE
"
  else
    local args=""
    local arg
    for arg in "$@"; do
      args+=" $(sh_quote "$arg")"
    done
    script="
set -Eeuo pipefail
idrive=$(sh_quote "$idrive")
config=$(sh_quote "$config")
\"\$idrive\" --config-dir \"\$config\"$args
"
  fi
  remote_exec "$label" "$script" | tr -d '\r'
}

daemon_relay_args_posix() {
  local args=""
  local relay
  for relay in ${IRIS_DRIVE_E2E_RELAYS:-}; do
    args+=" --relay $(sh_quote "$relay")"
  done
  printf "%s" "$args"
}

daemon_relay_args_windows() {
  local args=""
  local relay
  for relay in ${IRIS_DRIVE_E2E_RELAYS:-}; do
    args+="; \$daemonArgs += @('--relay', $(ps_quote "$relay"))"
  done
  printf "%s" "$args"
}

start_daemon() {
  local label="$1"
  local kind
  local idrive
  local config
  local log
  local err
  local pidfile
  local script
  local ssh_host
  local daemon_ssh_pid
  kind="$(host_value "$label" kind)"
  idrive="$(host_value "$label" idrive)"
  config="$(host_value "$label" config)"
  log="$(host_value "$label" log)"
  err="$(host_value "$label" err)"
  pidfile="$(host_value "$label" pid)"
  if [[ "$kind" == "windows" ]]; then
    ssh_host="$(host_value "$label" ssh)"
    daemon_ssh_pid="$(host_value "$label" daemon_ssh_pid)"
    if [[ -n "$daemon_ssh_pid" ]]; then
      kill "$daemon_ssh_pid" 2>/dev/null || true
      wait "$daemon_ssh_pid" 2>/dev/null || true
      set_host_value "$label" daemon_ssh_pid ""
    fi
    script="
\$ErrorActionPreference = 'Stop'
\$idrive = $(ps_quote "$idrive")
\$config = $(ps_quote "$config")
\$log = $(ps_quote "$log")
\$err = $(ps_quote "$err")
\$pidFile = $(ps_quote "$pidfile")
if (Test-Path -LiteralPath \$pidFile) {
  \$old = Get-Content -LiteralPath \$pidFile -ErrorAction SilentlyContinue
  if (\$old) { Stop-Process -Id ([int]\$old) -Force -ErrorAction SilentlyContinue }
}
\$daemonArgs = @('--config-dir', \$config, 'daemon', '--watch-interval', '2', '--watch-debounce-ms', '100', '--no-gateway')
$(daemon_relay_args_windows)
Set-Content -LiteralPath \$pidFile -Value \$PID
\$ErrorActionPreference = 'Continue'
& \$idrive @daemonArgs > \$log 2> \$err
"
    printf "%s" "$script" | ssh "$ssh_host" 'powershell -NoProfile -ExecutionPolicy Bypass -Command "`$script = [Console]::In.ReadToEnd(); Invoke-Expression `$script"' >/dev/null 2>&1 &
    set_host_value "$label" daemon_ssh_pid "$!"
    sleep 1
    if ! kill -0 "$(host_value "$label" daemon_ssh_pid)" 2>/dev/null; then
      wait "$(host_value "$label" daemon_ssh_pid)" 2>/dev/null || true
      set_host_value "$label" daemon_ssh_pid ""
      echo "windows daemon ssh session exited early for $label" >&2
      return 1
    fi
    return 0
  else
    script="
set -Eeuo pipefail
idrive=$(sh_quote "$idrive")
config=$(sh_quote "$config")
log=$(sh_quote "$log")
err=$(sh_quote "$err")
pidfile=$(sh_quote "$pidfile")
if [[ -f \"\$pidfile\" ]]; then
  old=\"\$(cat \"\$pidfile\" 2>/dev/null || true)\"
  if [[ -n \"\$old\" ]]; then kill \"\$old\" 2>/dev/null || true; fi
fi
nohup \"\$idrive\" --config-dir \"\$config\" daemon --watch-interval 2 --watch-debounce-ms 100 --no-gateway$(daemon_relay_args_posix) >\"\$log\" 2>\"\$err\" < /dev/null &
echo \$! >\"\$pidfile\"
"
  fi
  remote_exec "$label" "$script" | tr -d '\r'
}

stop_daemon() {
  local label="$1"
  local kind
  local pidfile
  local daemon_ssh_pid
  local script
  kind="$(host_value "$label" kind)"
  pidfile="$(host_value "$label" pid)"
  if [[ "$kind" == "windows" ]]; then
    daemon_ssh_pid="$(host_value "$label" daemon_ssh_pid)"
    if [[ -n "$daemon_ssh_pid" ]]; then
      kill "$daemon_ssh_pid" 2>/dev/null || true
      wait "$daemon_ssh_pid" 2>/dev/null || true
      set_host_value "$label" daemon_ssh_pid ""
    fi
    if [[ -z "$pidfile" ]]; then
      return
    fi
    script="
\$pidFile = $(ps_quote "$pidfile")
if (Test-Path -LiteralPath \$pidFile) {
  \$oldPid = Get-Content -LiteralPath \$pidFile -ErrorAction SilentlyContinue
  if (\$oldPid) { Stop-Process -Id ([int]\$oldPid) -Force -ErrorAction SilentlyContinue }
}
"
  else
    if [[ -z "$pidfile" ]]; then
      return
    fi
    script="
pidfile=$(sh_quote "$pidfile")
if [[ -f \"\$pidfile\" ]]; then
  pid=\"\$(cat \"\$pidfile\" 2>/dev/null || true)\"
  if [[ -n \"\$pid\" ]]; then kill \"\$pid\" 2>/dev/null || true; fi
fi
"
  fi
  remote_exec "$label" "$script" || true
}

cleanup() {
  if [[ "$KEEP" == "1" ]]; then
    echo "keeping remote temp dirs because IRIS_DRIVE_E2E_KEEP=1"
    return
  fi
  local label kind base script
  for label in "${LABELS[@]}"; do
    stop_daemon "$label"
    base="$(host_value "$label" base)"
    [[ -n "$base" ]] || continue
    kind="$(host_value "$label" kind)"
    if [[ "$kind" == "windows" ]]; then
      script="\$base = $(ps_quote "$base"); if (Test-Path -LiteralPath \$base) { Remove-Item -LiteralPath \$base -Recurse -Force -ErrorAction SilentlyContinue }"
    else
      script="rm -rf $(sh_quote "$base")"
    fi
    remote_exec "$label" "$script" || true
  done
}
trap cleanup EXIT

write_file() {
  local label="$1"
  local rel="$2"
  local content="$3"
  local kind
  local work
  local b64 script
  kind="$(host_value "$label" kind)"
  work="$(host_value "$label" work)"
  b64="$(printf "%s" "$content" | base64 | tr -d '\n')"
  if [[ "$kind" == "windows" ]]; then
    script="
\$work = $(ps_quote "$work")
\$rel = $(ps_quote "$rel")
\$path = \$work
foreach (\$part in (\$rel -split '/')) { \$path = Join-Path \$path \$part }
\$parent = Split-Path -Parent \$path
New-Item -ItemType Directory -Force -Path \$parent | Out-Null
[IO.File]::WriteAllBytes(\$path, [Convert]::FromBase64String($(ps_quote "$b64")))
"
  else
    script="
set -Eeuo pipefail
work=$(sh_quote "$work")
rel=$(sh_quote "$rel")
path=\"\$work/\$rel\"
mkdir -p \"\$(dirname \"\$path\")\"
printf '%s' $(sh_quote "$b64") | base64 -d >\"\$path\"
"
  fi
  remote_exec "$label" "$script"
}

write_zero_file() {
  local label="$1"
  local rel="$2"
  local bytes="$3"
  local kind
  local work
  local script
  kind="$(host_value "$label" kind)"
  work="$(host_value "$label" work)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$work = $(ps_quote "$work")
\$rel = $(ps_quote "$rel")
\$bytes = [int]$(ps_quote "$bytes")
\$path = \$work
foreach (\$part in (\$rel -split '/')) { \$path = Join-Path \$path \$part }
\$parent = Split-Path -Parent \$path
New-Item -ItemType Directory -Force -Path \$parent | Out-Null
[IO.File]::WriteAllBytes(\$path, [byte[]]::new(\$bytes))
"
  else
    script="
set -Eeuo pipefail
work=$(sh_quote "$work")
rel=$(sh_quote "$rel")
bytes=$(sh_quote "$bytes")
path=\"\$work/\$rel\"
mkdir -p \"\$(dirname \"\$path\")\"
dd if=/dev/zero of=\"\$path\" bs=\"\$bytes\" count=1 2>/dev/null
"
  fi
  remote_exec "$label" "$script"
}

mkdir_remote() {
  local label="$1"
  local rel="$2"
  local kind
  local work
  local script
  kind="$(host_value "$label" kind)"
  work="$(host_value "$label" work)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$path = $(ps_quote "$work")
foreach (\$part in ($(ps_quote "$rel") -split '/')) { \$path = Join-Path \$path \$part }
New-Item -ItemType Directory -Force -Path \$path | Out-Null
"
  else
    script="mkdir -p $(sh_quote "$work/$rel")"
  fi
  remote_exec "$label" "$script"
}

rename_remote() {
  local label="$1"
  local from="$2"
  local to="$3"
  local kind
  local work
  local script
  kind="$(host_value "$label" kind)"
  work="$(host_value "$label" work)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$work = $(ps_quote "$work")
function Join-Rel([string]\$root, [string]\$rel) {
  \$path = \$root
  foreach (\$part in (\$rel -split '/')) { \$path = Join-Path \$path \$part }
  return \$path
}
\$src = Join-Rel \$work $(ps_quote "$from")
\$dst = Join-Rel \$work $(ps_quote "$to")
New-Item -ItemType Directory -Force -Path (Split-Path -Parent \$dst) | Out-Null
Move-Item -LiteralPath \$src -Destination \$dst -Force
"
  else
    script="
set -Eeuo pipefail
work=$(sh_quote "$work")
from=$(sh_quote "$from")
to=$(sh_quote "$to")
src=\"\$work/\$from\"
dst=\"\$work/\$to\"
mkdir -p \"\$(dirname \"\$dst\")\"
mv \"\$src\" \"\$dst\"
"
  fi
  remote_exec "$label" "$script"
}

remove_remote() {
  local label="$1"
  local rel="$2"
  local kind
  local work
  local script
  kind="$(host_value "$label" kind)"
  work="$(host_value "$label" work)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$path = $(ps_quote "$work")
foreach (\$part in ($(ps_quote "$rel") -split '/')) { \$path = Join-Path \$path \$part }
Remove-Item -LiteralPath \$path -Recurse -Force -ErrorAction SilentlyContinue
"
  else
    script="rm -rf $(sh_quote "$work/$rel")"
  fi
  remote_exec "$label" "$script"
}

snapshot() {
  local label="$1"
  local kind
  local work
  local script
  kind="$(host_value "$label" kind)"
  work="$(host_value "$label" work)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$work = $(ps_quote "$work")
\$tab = [char]9
function Is-Ignored([string]\$rel) {
  \$leaf = Split-Path -Leaf \$rel
  if (\$rel -eq '.hashtree' -or \$rel.StartsWith('.hashtree/')) { return \$true }
  if (@('.DS_Store', 'Thumbs.db', 'desktop.ini') -contains \$leaf) { return \$true }
  if (\$leaf.StartsWith('._')) { return \$true }
  if (\$leaf.EndsWith('~')) { return \$true }
  if (\$leaf.StartsWith('#') -and \$leaf.EndsWith('#')) { return \$true }
  if (\$leaf.EndsWith('.sbak')) { return \$true }
  return \$false
}
\$rows = @()
Get-ChildItem -LiteralPath \$work -Recurse -File -Force | ForEach-Object {
  \$rel = \$_.FullName.Substring(\$work.Length).TrimStart([char]92, [char]47).Replace([string][char]92, '/')
  if (-not (Is-Ignored \$rel)) {
    \$hash = (Get-FileHash -LiteralPath \$_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    \$rows += (\$hash + \$tab + ([int64]\$_.Length).ToString() + \$tab + \$rel)
  }
}
\$rows | Sort-Object
"
  else
    script='
set -Eeuo pipefail
root='"$(sh_quote "$work")"'
is_ignored() {
  rel="$1"
  name="${rel##*/}"
  case "$rel" in .hashtree|.hashtree/*) return 0 ;; esac
  case "$name" in .DS_Store|Thumbs.db|desktop.ini) return 0 ;; esac
  [[ "$name" == ._* ]] && return 0
  [[ "$name" == *~ ]] && return 0
  [[ "$name" == \#*\# ]] && return 0
  [[ "$name" == *.sbak ]] && return 0
  return 1
}
while IFS= read -r -d "" file; do
  rel="${file#"$root"/}"
  if is_ignored "$rel"; then
    continue
  fi
  hash="$(shasum -a 256 "$file" | awk "{print \$1}")"
  size="$(wc -c <"$file" | tr -d " ")"
  printf "%s\t%s\t%s\n" "$hash" "$size" "$rel"
done < <(find "$root" -type f -print0) | LC_ALL=C sort
'
  fi
  remote_exec "$label" "$script" | tr -d '\r'
}

union_snapshots() {
  local label
  for label in "${LABELS[@]}"; do
    snapshot "$label"
  done | LC_ALL=C sort -u
}

print_statuses() {
  local label
  for label in "${LABELS[@]}"; do
    echo "---- $label status ----" >&2
    idrive_cmd "$label" status >&2 || true
    echo "---- $label snapshot ----" >&2
    snapshot "$label" >&2 || true
  done
}

wait_until() {
  local label="$1"
  local check="$2"
  local start now
  start="$(date +%s)"
  while true; do
    if "$check"; then
      echo "ok: $label"
      return 0
    fi
    now="$(date +%s)"
    if (( now - start >= TIMEOUT_SECS )); then
      echo "timed out waiting for $label" >&2
      print_statuses
      return 1
    fi
    sleep "$POLL_SECS"
  done
}

all_authorized() {
  local label status
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e '.account.authorization_state == "authorized"' >/dev/null 2>&1 <<<"$status" || return 1
  done
}

all_fresh() {
  local label status
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e '.daemon.running == true and .daemon.fresh == true' >/dev/null 2>&1 <<<"$status" || return 1
  done
}

all_have_direct_peer() {
  local label status
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e '.network.fips.connected_peer_count >= 1' >/dev/null 2>&1 <<<"$status" || return 1
  done
}

wait_for_snapshot() {
  local expected="$1"
  local label="$2"
  EXPECTED_SNAPSHOT="$expected"
  wait_until "$label" snapshots_match_expected
}

snapshot_file_count() {
  local snapshot="$1"
  if [[ -z "$snapshot" ]]; then
    echo 0
    return
  fi
  printf "%s\n" "$snapshot" | sed '/^$/d' | wc -l | tr -d ' '
}

wait_for_converged_union() {
  local label="$1"
  wait_until "$label" snapshots_match_current_union
}

snapshots_match_expected() {
  local host_label current
  for host_label in "${LABELS[@]}"; do
    current="$(snapshot "$host_label")"
    if [[ "$current" != "$EXPECTED_SNAPSHOT" ]]; then
      return 1
    fi
  done
  return 0
}

snapshots_match_current_union() {
  local expected host_label current
  expected="$(union_snapshots)"
  for host_label in "${LABELS[@]}"; do
    current="$(snapshot "$host_label")"
    if [[ "$current" != "$expected" ]]; then
      return 1
    fi
  done
  return 0
}

wait_for_source_snapshot() {
  local label="$1"
  local step="$2"
  local expected
  expected="$(snapshot "$label")"
  EXPECTED_SNAPSHOT="$expected"
  EXPECTED_SOURCE_LABEL="$label"
  EXPECTED_SOURCE_FILE_COUNT="$(snapshot_file_count "$expected")"
  wait_until "$step" source_and_snapshots_match_expected
}

source_root_matches_expected_count() {
  local status
  status="$(idrive_cmd "$EXPECTED_SOURCE_LABEL" status 2>/dev/null || true)"
  jq -e --argjson count "$EXPECTED_SOURCE_FILE_COUNT" \
    '.daemon.running == true and .daemon.fresh == true and .hashtree.file_count == $count' \
    >/dev/null 2>&1 <<<"$status"
}

source_and_snapshots_match_expected() {
  snapshots_match_expected && source_root_matches_expected_count
}

run_step() {
  local name="$1"
  shift
  echo
  echo "== $name =="
  "$@"
}

step_create_edit_rename_delete() {
  write_file "$source_label" "ops/create-edit.txt" "version 1 from $source_label"
  wait_for_source_snapshot "$source_label" "create from source"
  write_file "$source_label" "ops/create-edit.txt" "version 2 from $source_label"
  wait_for_source_snapshot "$source_label" "edit from source"
  rename_remote "$source_label" "ops/create-edit.txt" "ops/renamed.txt"
  wait_for_source_snapshot "$source_label" "rename from source"
  remove_remote "$source_label" "ops/renamed.txt"
  wait_for_source_snapshot "$source_label" "delete from source"
}

step_nested_create_delete() {
  write_file "$target_label" "download/dir1/one.txt" "nested from $target_label"
  rename_remote "$target_label" "download/dir1" "download/dir2"
  wait_for_source_snapshot "$target_label" "nested rename"
  remove_remote "$target_label" "download/dir2/one.txt"
  remove_remote "$target_label" "download/dir2"
  wait_for_source_snapshot "$target_label" "nested delete"
}

step_file_type_replacements() {
  write_file "$source_label" "types/file-to-dir" "old file"
  write_file "$source_label" "types/dir-to-file/old.txt" "old child"
  wait_for_source_snapshot "$source_label" "initial file type setup"
  remove_remote "$source_label" "types/file-to-dir"
  write_file "$source_label" "types/file-to-dir/new.txt" "new child"
  remove_remote "$source_label" "types/dir-to-file"
  write_file "$source_label" "types/dir-to-file" "new file"
  wait_for_source_snapshot "$source_label" "file type replacements"
}

step_rename_chain() {
  write_file "$source_label" "release/rename/1.txt" "111"
  rename_remote "$source_label" "release/rename/1.txt" "release/rename/2.txt"
  write_file "$source_label" "release/rename/3.txt" "222"
  rename_remote "$source_label" "release/rename/2.txt" "release/rename/3.txt"
  write_file "$source_label" "release/rename/test.txt" "test"
  mkdir_remote "$source_label" "release/rename/test"
  write_file "$source_label" "release/rename/4.txt" "444"
  rename_remote "$source_label" "release/rename/test.txt" "release/rename/test/test.txt"
  rename_remote "$source_label" "release/rename/3.txt" "release/rename/test/3.txt"
  rename_remote "$source_label" "release/rename/4.txt" "release/rename/test/4.txt"
  mkdir_remote "$source_label" "release/rename/test2"
  rename_remote "$source_label" "release/rename/test" "release/rename/test2/test"
  rename_remote "$source_label" "release/rename/test2/test" "release/rename/test"
  write_file "$source_label" "release/rename/test/4.txt" "444555"
  rename_remote "$source_label" "release/rename/test" "release/rename/test2/test"
  rename_remote "$source_label" "release/rename/test2" "release/rename/test3"
  wait_for_source_snapshot "$source_label" "rename chain"
}

step_ignored_noise() {
  write_file "$source_label" "noise/keep.txt" "keep"
  write_file "$source_label" "noise/.DS_Store" "finder"
  write_file "$source_label" "noise/._keep.txt" "resource fork"
  write_file "$source_label" "noise/Thumbs.db" "thumbs"
  write_file "$source_label" "noise/desktop.ini" "desktop"
  write_file "$source_label" "noise/draft~" "backup"
  write_file "$source_label" "noise/#draft#" "emacs"
  write_file "$source_label" "noise/backup.sbak" "seafile backup"
  write_file "$source_label" ".hashtree/prev" "internal"
  wait_for_source_snapshot "$source_label" "ignored noise"
}

step_receiver_restart() {
  local i
  stop_daemon "$target_label"
  for i in $(seq 1 12); do
    write_file "$source_label" "reconnect/file-$i.txt" "file $i while $target_label stopped"
  done
  start_daemon "$target_label"
  wait_until "target daemon fresh after restart" all_fresh
  wait_for_source_snapshot "$source_label" "receiver restart"
}

step_many_small_files() {
  local i
  for i in $(seq 1 "$MANY_FILES"); do
    write_file "$source_label" "many/$(printf "%03d" "$i").txt" "many file $i from $source_label"
  done
  wait_for_source_snapshot "$source_label" "many small files"
}

step_large_file() {
  write_zero_file "$target_label" "large/zero.bin" "$LARGE_BYTES"
  wait_for_source_snapshot "$target_label" "large file"
}

owner_label="${LABELS[0]}"
windows_label=""
ubuntu_label=""
macos_label=""
for label in "${LABELS[@]}"; do
  case "$label" in
    win*|windows) windows_label="$label" ;;
    ubuntu*|linux*) ubuntu_label="$label" ;;
    mac*|darwin*) macos_label="$label" ;;
  esac
done
source_label="${windows_label:-${LABELS[0]}}"
target_label="${ubuntu_label:-${LABELS[1]}}"

echo "run id: $RUN_ID"
echo "hosts: ${LABELS[*]}"

for label in "${LABELS[@]}"; do
  echo "setting up $label ($(host_value "$label" ssh))"
  setup_host "$label"
done

echo "initializing owner on $owner_label"
owner_json="$(idrive_cmd "$owner_label" init --label "$owner_label")"
owner_npub="$(jq -r '.owner_npub' <<<"$owner_json")"

for label in "${LABELS[@]}"; do
  if [[ "$label" == "$owner_label" ]]; then
    continue
  fi
  echo "linking $label"
  link_json="$(idrive_cmd "$label" link "$owner_npub" --label "$label")"
  request_url="$(jq -r '.device_link_request.url' <<<"$link_json")"
  idrive_cmd "$owner_label" approve "$request_url" --label "$label" >/dev/null
done

for label in "${LABELS[@]}"; do
  write_file "$label" "seed/$label.txt" "seed from $label in $RUN_ID
"
  write_file "$label" "shared/same.txt" "same bytes from all devices
"
  idrive_cmd "$label" import "$(host_value "$label" work)" >/dev/null
done

for label in "${LABELS[@]}"; do
  start_daemon "$label"
done

run_step "authorization" wait_until "all devices authorized" all_authorized
run_step "fresh daemons" wait_until "all daemon statuses fresh" all_fresh

run_step "initial multi-device merge" wait_for_converged_union "initial merge"

run_step "direct FIPS peer discovery" wait_until "every device has a direct peer" all_have_direct_peer

run_step "create edit rename delete from $source_label" step_create_edit_rename_delete
run_step "nested create/delete from $target_label" step_nested_create_delete
run_step "file type replacements" step_file_type_replacements
run_step "seafile-style rename chain" step_rename_chain
run_step "ignored desktop/editor noise" step_ignored_noise
run_step "receiver restart convergence" step_receiver_restart
run_step "many small files" step_many_small_files
run_step "large file" step_large_file

run_step "final fresh daemons" wait_until "all daemon statuses fresh" all_fresh
run_step "final direct FIPS peer discovery" wait_until "every device has a direct peer" all_have_direct_peer

echo
echo "cross-vm e2e passed for: ${LABELS[*]}"
