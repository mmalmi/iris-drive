#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/cross-vm-e2e.sh --host LABEL=KIND:SSH_HOST [--host ...]
  scripts/cross-vm-three-platform-e2e.sh

KIND is one of:
  posix     Linux or macOS host reachable by SSH with bash
  windows   Windows host reachable by SSH with PowerShell

For the standard macOS + Ubuntu + Windows matrix, prefer
scripts/cross-vm-three-platform-e2e.sh with hostnames supplied through
environment variables. That keeps private SSH hostnames out of the repo while
still making the intended three-platform test obvious.

The script creates isolated temp config/work directories on every host, links
them into one Iris Drive account, starts real idrive daemons, mutates files,
and waits until every host has the same visible SHA-256 snapshot.

Environment:
  IRIS_DRIVE_E2E_RELAYS          Space-separated relay URLs passed to daemons.
  IRIS_DRIVE_E2E_TIMEOUT_SECS    Convergence timeout per step (default: 60).
  IRIS_DRIVE_E2E_REMOTE_TIMEOUT_SECS
                                  Per SSH command timeout; 0 disables (default: 60).
  IRIS_DRIVE_E2E_MANY_FILES      Many-file test count (default: 32).
  IRIS_DRIVE_E2E_LARGE_BYTES     Large-file test bytes (default: 262144).
  IRIS_DRIVE_E2E_MOUNT_LABELS    Space-separated POSIX labels that should expose FUSE mounts.
                                  Defaults to every POSIX host in this run.
  IRIS_DRIVE_E2E_PROVIDER_MUTATIONS
                                  Use provider commands instead of projection surfaces when set to 1.
  IRIS_DRIVE_E2E_SIDELOAD_APPKEYS
                                  Copy the owner AppKeys snapshot into temp peer configs after approval
                                  so VM file-sync tests do not depend on public relay timing (default: 1).
  IRIS_DRIVE_E2E_KEEP            Keep remote temp dirs/daemons when set to 1.
USAGE
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_ID="run-$(date +%Y%m%d%H%M%S)-$$"
TIMEOUT_SECS="${IRIS_DRIVE_E2E_TIMEOUT_SECS:-60}"
REMOTE_TIMEOUT_SECS="${IRIS_DRIVE_E2E_REMOTE_TIMEOUT_SECS:-60}"
POLL_SECS="${IRIS_DRIVE_E2E_POLL_SECS:-3}"
MANY_FILES="${IRIS_DRIVE_E2E_MANY_FILES:-32}"
LARGE_BYTES="${IRIS_DRIVE_E2E_LARGE_BYTES:-262144}"
KEEP="${IRIS_DRIVE_E2E_KEEP:-0}"
MOUNT_LABELS="${IRIS_DRIVE_E2E_MOUNT_LABELS:-}"
SIDELOAD_APPKEYS="${IRIS_DRIVE_E2E_SIDELOAD_APPKEYS:-1}"
PROVIDER_MUTATIONS="${IRIS_DRIVE_E2E_PROVIDER_MUTATIONS:-0}"

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

vpath() {
  local rel="${1#/}"
  printf "e2e/%s/%s" "$RUN_ID" "$rel"
}

projection_enabled() {
  local label="$1"
  local kind
  kind="$(host_value "$label" kind)"
  if [[ "$PROVIDER_MUTATIONS" == "1" ]]; then
    return 1
  fi
  if [[ "$kind" == "windows" ]]; then
    return 0
  fi
  case " $MOUNT_LABELS " in
    *" $label "*) return 0 ;;
    *) return 1 ;;
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

run_remote_exec() {
  local label="$1"
  local script="$2"
  local kind
  local ssh_host
  kind="$(host_value "$label" kind)"
  ssh_host="$(host_value "$label" ssh)"
  if [[ "$kind" == "windows" ]]; then
    printf "%s" "$script" | ssh "$ssh_host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -'
  else
    printf "%s" "$script" | ssh "$ssh_host" 'bash -se'
  fi
}

remote_exec() {
  local label="$1"
  local script="$2"
  local pid
  local watchdog
  local status

  if (( REMOTE_TIMEOUT_SECS <= 0 )); then
    run_remote_exec "$label" "$script"
    return
  fi

  run_remote_exec "$label" "$script" &
  pid="$!"
  (
    deadline=$((SECONDS + REMOTE_TIMEOUT_SECS))
    while kill -0 "$pid" 2>/dev/null; do
      if (( SECONDS >= deadline )); then
        echo "remote command timed out after ${REMOTE_TIMEOUT_SECS}s on $label" >&2
        kill "$pid" 2>/dev/null || true
        sleep 1
        kill -9 "$pid" 2>/dev/null || true
        exit 0
      fi
      sleep 1
    done
  ) >/dev/null &
  watchdog="$!"

  set +e
  wait "$pid"
  status="$?"
  set -e
  kill "$watchdog" 2>/dev/null || true
  wait "$watchdog" 2>/dev/null || true
  return "$status"
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

owner_app_keys_b64() {
  local label="$1"
  local kind
  local config
  local script
  kind="$(host_value "$label" kind)"
  config="$(host_value "$label" config)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$config = $(ps_quote "$config")
\$text = Get-Content -LiteralPath (Join-Path \$config 'config.toml') -Raw
\$match = [regex]::Match(\$text, '(?s)\\[account\\.app_keys\\].*?(?=\\r?\\n\\[\\[drives\\]\\])')
if (-not \$match.Success) { throw 'owner AppKeys block not found' }
[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(\$match.Value))
"
  else
    script="
set -Eeuo pipefail
config=$(sh_quote "$config")
awk 'BEGIN{copy=0} /^\\[account\\.app_keys\\]/{copy=1} /^\\[\\[drives\\]\\]/{copy=0} copy{print}' \"\$config/config.toml\" | base64 | tr -d '\\n'
"
  fi
  remote_exec "$label" "$script" | tr -d '\r\n'
}

sideload_app_keys() {
  local label="$1"
  local appkeys_b64="$2"
  local kind
  local config
  local script
  kind="$(host_value "$label" kind)"
  config="$(host_value "$label" config)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$config = $(ps_quote "$config")
\$path = Join-Path \$config 'config.toml'
\$appKeys = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($(ps_quote "$appkeys_b64")))
\$text = Get-Content -LiteralPath \$path -Raw
\$text = \$text.Replace('authorization_state = \"awaiting_approval\"', 'authorization_state = \"authorized\"')
\$lf = [string][char]10
\$pattern = '(?s)\\r?\\n\\[account\\.app_keys\\].*?(?=\\r?\\n\\[\\[drives\\]\\])'
if ([regex]::IsMatch(\$text, \$pattern)) {
  \$text = [regex]::Replace(\$text, \$pattern, (\$lf + \$appKeys + \$lf), 1)
} else {
  \$text = \$text.Replace(\$lf + '[[drives]]', \$lf + \$appKeys + \$lf + '[[drives]]')
}
Set-Content -LiteralPath \$path -Value \$text -NoNewline
"
  else
    script="
set -Eeuo pipefail
CONFIG_PATH=$(sh_quote "$config") APPKEYS_B64=$(sh_quote "$appkeys_b64") python3 - <<'PY'
import base64, os, re
from pathlib import Path

path = Path(os.environ['CONFIG_PATH']) / 'config.toml'
appkeys = base64.b64decode(os.environ['APPKEYS_B64']).decode()
text = path.read_text()
text = text.replace('authorization_state = \"awaiting_approval\"', 'authorization_state = \"authorized\"')
text = re.sub(r'\\n\\[account\\.app_keys\\].*?(?=\\n\\[\\[drives\\]\\])', '\\n' + appkeys + '\\n', text, count=1, flags=re.S)
if '[account.app_keys]' not in text:
    text = text.replace('\\n[[drives]]', '\\n' + appkeys + '\\n[[drives]]', 1)
path.write_text(text)
PY
"
  fi
  remote_exec "$label" "$script" >/dev/null
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
  local work
  local pidfile
  local script
  local ssh_host
  local daemon_ssh_pid
  kind="$(host_value "$label" kind)"
  idrive="$(host_value "$label" idrive)"
  config="$(host_value "$label" config)"
  log="$(host_value "$label" log)"
  err="$(host_value "$label" err)"
  work="$(host_value "$label" work)"
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
\$daemonArgs = @('--config-dir', \$config, 'daemon', '--watch-debounce-ms', '100', '--gateway-port', '0')
$(daemon_relay_args_windows)
Set-Content -LiteralPath \$pidFile -Value \$PID
\$ErrorActionPreference = 'Continue'
& \$idrive @daemonArgs > \$log 2> \$err
"
    printf "%s" "$script" | ssh "$ssh_host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' >/dev/null 2>&1 &
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
label=$(sh_quote "$label")
idrive=$(sh_quote "$idrive")
config=$(sh_quote "$config")
log=$(sh_quote "$log")
err=$(sh_quote "$err")
pidfile=$(sh_quote "$pidfile")
work=$(sh_quote "$work")
mount_labels=$(sh_quote "$MOUNT_LABELS")
mount_enabled=0
case \" \$mount_labels \" in
  *\" \$label \"*) mount_enabled=1 ;;
esac
if [[ -f \"\$pidfile\" ]]; then
  old=\"\$(cat \"\$pidfile\" 2>/dev/null || true)\"
  if [[ -n \"\$old\" ]]; then
    kill \"\$old\" 2>/dev/null || true
    for _ in {1..30}; do
      if kill -0 \"\$old\" 2>/dev/null; then
        sleep 0.1
      else
        break
      fi
    done
    kill -0 \"\$old\" 2>/dev/null && kill -9 \"\$old\" 2>/dev/null || true
  fi
fi
case \" \$mount_labels \" in
  *\" \$label \"*)
    fusermount3 -u \"\$work\" 2>/dev/null || fusermount -u \"\$work\" 2>/dev/null || umount \"\$work\" 2>/dev/null || true
    mkdir -p \"\$work\"
    ;;
esac
if (( mount_enabled )); then
  nohup \"\$idrive\" --config-dir \"\$config\" daemon --watch-debounce-ms 100 --gateway-port 0 --mount --mountpoint \"\$work\"$(daemon_relay_args_posix) >\"\$log\" 2>\"\$err\" < /dev/null &
else
  nohup \"\$idrive\" --config-dir \"\$config\" daemon --watch-debounce-ms 100 --gateway-port 0$(daemon_relay_args_posix) >\"\$log\" 2>\"\$err\" < /dev/null &
fi
echo \$! >\"\$pidfile\"
"
  fi
  remote_exec "$label" "$script" | tr -d '\r'
}

stop_daemon() {
  local label="$1"
  local kind
  local pidfile
  local work
  local daemon_ssh_pid
  local script
  kind="$(host_value "$label" kind)"
  pidfile="$(host_value "$label" pid)"
  work="$(host_value "$label" work)"
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
label=$(sh_quote "$label")
pidfile=$(sh_quote "$pidfile")
work=$(sh_quote "$work")
mount_labels=$(sh_quote "$MOUNT_LABELS")
if [[ -f \"\$pidfile\" ]]; then
  pid=\"\$(cat \"\$pidfile\" 2>/dev/null || true)\"
  if [[ -n \"\$pid\" ]]; then
    kill \"\$pid\" 2>/dev/null || true
    for _ in {1..30}; do
      if kill -0 \"\$pid\" 2>/dev/null; then
        sleep 0.1
      else
        break
      fi
    done
    kill -0 \"\$pid\" 2>/dev/null && kill -9 \"\$pid\" 2>/dev/null || true
  fi
fi
case \" \$mount_labels \" in
  *\" \$label \"*)
    fusermount3 -u \"\$work\" 2>/dev/null || fusermount -u \"\$work\" 2>/dev/null || umount \"\$work\" 2>/dev/null || true
    ;;
esac
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
      script="
\$base = $(ps_quote "$base")
\$projectionScope = Join-Path (Join-Path (Join-Path \$HOME 'Iris Drive') 'e2e') $(ps_quote "$RUN_ID")
if (Test-Path -LiteralPath \$projectionScope) { Remove-Item -LiteralPath \$projectionScope -Recurse -Force -ErrorAction SilentlyContinue }
if (Test-Path -LiteralPath \$base) { Remove-Item -LiteralPath \$base -Recurse -Force -ErrorAction SilentlyContinue }
"
    else
      script="rm -rf $(sh_quote "$base")"
    fi
    remote_exec "$label" "$script" || true
  done
}
trap cleanup EXIT

write_file() {
  local label="$1"
  local rel
  rel="$(vpath "$2")"
  local content="$3"
  local kind
  local idrive
  local config
  local base
  local b64 script
  kind="$(host_value "$label" kind)"
  idrive="$(host_value "$label" idrive)"
  config="$(host_value "$label" config)"
  base="$(host_value "$label" base)"
  b64="$(printf "%s" "$content" | base64 | tr -d '\n')"
  if projection_enabled "$label"; then
    if [[ "$kind" == "windows" ]]; then
      script="
\$ErrorActionPreference = 'Stop'
\$root = Join-Path \$HOME 'Iris Drive'
\$path = Join-Path \$root ($(ps_quote "$rel") -replace '/', [IO.Path]::DirectorySeparatorChar)
\$parent = Split-Path -Parent \$path
New-Item -ItemType Directory -Force -Path \$parent | Out-Null
\$bytes = [Convert]::FromBase64String($(ps_quote "$b64"))
[IO.File]::WriteAllBytes(\$path, \$bytes)
"
    else
      script="
set -Eeuo pipefail
root=$(sh_quote "$(host_value "$label" work)")
path=\"\$root/$(printf "%s" "$rel" | sed "s/'/'\\\\''/g")\"
mkdir -p \"\$(dirname \"\$path\")\"
printf '%s' $(sh_quote "$b64") | base64 -d >\"\$path\"
"
    fi
  elif [[ "$kind" == "windows" ]]; then
    script="
\$ErrorActionPreference = 'Stop'
\$idrive = $(ps_quote "$idrive")
\$config = $(ps_quote "$config")
\$source = Join-Path $(ps_quote "$base") 'provider-source.bin'
\$bytes = [Convert]::FromBase64String($(ps_quote "$b64"))
[IO.File]::WriteAllBytes(\$source, \$bytes)
& \$idrive --config-dir \$config provider write $(ps_quote "$rel") \$source | Out-Null
exit \$LASTEXITCODE
"
  else
    script="
set -Eeuo pipefail
idrive=$(sh_quote "$idrive")
config=$(sh_quote "$config")
source=$(sh_quote "$base/provider-source.bin")
printf '%s' $(sh_quote "$b64") | base64 -d >\"\$source\"
\"\$idrive\" --config-dir \"\$config\" provider write $(sh_quote "$rel") \"\$source\" >/dev/null
"
  fi
  remote_exec "$label" "$script"
}

write_zero_file() {
  local label="$1"
  local rel
  rel="$(vpath "$2")"
  local bytes="$3"
  local kind
  local idrive
  local config
  local base
  local script
  kind="$(host_value "$label" kind)"
  idrive="$(host_value "$label" idrive)"
  config="$(host_value "$label" config)"
  base="$(host_value "$label" base)"
  if projection_enabled "$label"; then
    if [[ "$kind" == "windows" ]]; then
      script="
\$ErrorActionPreference = 'Stop'
\$bytes = [int]$(ps_quote "$bytes")
\$root = Join-Path \$HOME 'Iris Drive'
\$path = Join-Path \$root ($(ps_quote "$rel") -replace '/', [IO.Path]::DirectorySeparatorChar)
\$parent = Split-Path -Parent \$path
New-Item -ItemType Directory -Force -Path \$parent | Out-Null
[IO.File]::WriteAllBytes(\$path, [byte[]]::new(\$bytes))
"
    else
      script="
set -Eeuo pipefail
bytes=$(sh_quote "$bytes")
root=$(sh_quote "$(host_value "$label" work)")
path=\"\$root/$rel\"
mkdir -p \"\$(dirname \"\$path\")\"
head -c \"\$bytes\" /dev/zero >\"\$path\"
"
    fi
  elif [[ "$kind" == "windows" ]]; then
    script="
\$bytes = [int]$(ps_quote "$bytes")
\$idrive = $(ps_quote "$idrive")
\$config = $(ps_quote "$config")
\$source = Join-Path $(ps_quote "$base") 'provider-source-zero.bin'
[IO.File]::WriteAllBytes(\$source, [byte[]]::new(\$bytes))
& \$idrive --config-dir \$config provider write $(ps_quote "$rel") \$source | Out-Null
exit \$LASTEXITCODE
"
  else
    script="
set -Eeuo pipefail
bytes=$(sh_quote "$bytes")
idrive=$(sh_quote "$idrive")
config=$(sh_quote "$config")
source=$(sh_quote "$base/provider-source-zero.bin")
head -c \"\$bytes\" /dev/zero >\"\$source\"
\"\$idrive\" --config-dir \"\$config\" provider write $(sh_quote "$rel") \"\$source\" >/dev/null
"
  fi
  remote_exec "$label" "$script"
}

mkdir_remote() {
  local label="$1"
  local rel
  rel="$(vpath "$2")"
  local kind script
  kind="$(host_value "$label" kind)"
  if projection_enabled "$label"; then
    if [[ "$kind" == "windows" ]]; then
      script="
\$ErrorActionPreference = 'Stop'
\$root = Join-Path \$HOME 'Iris Drive'
\$path = Join-Path \$root ($(ps_quote "$rel") -replace '/', [IO.Path]::DirectorySeparatorChar)
New-Item -ItemType Directory -Force -Path \$path | Out-Null
"
    else
      script="
set -Eeuo pipefail
root=$(sh_quote "$(host_value "$label" work)")
mkdir -p \"\$root/$rel\"
"
    fi
    remote_exec "$label" "$script"
  else
    idrive_cmd "$label" provider mkdir "$rel" >/dev/null
  fi
}

rename_remote() {
  local label="$1"
  local from to kind script
  from="$(vpath "$2")"
  to="$(vpath "$3")"
  kind="$(host_value "$label" kind)"
  if projection_enabled "$label"; then
    if [[ "$kind" == "windows" ]]; then
      script="
\$ErrorActionPreference = 'Stop'
\$root = Join-Path \$HOME 'Iris Drive'
\$from = Join-Path \$root ($(ps_quote "$from") -replace '/', [IO.Path]::DirectorySeparatorChar)
\$to = Join-Path \$root ($(ps_quote "$to") -replace '/', [IO.Path]::DirectorySeparatorChar)
\$parent = Split-Path -Parent \$to
New-Item -ItemType Directory -Force -Path \$parent | Out-Null
Move-Item -LiteralPath \$from -Destination \$to -Force
"
    else
      script="
set -Eeuo pipefail
root=$(sh_quote "$(host_value "$label" work)")
mkdir -p \"\$(dirname \"\$root/$to\")\"
mv -f \"\$root/$from\" \"\$root/$to\"
"
    fi
    remote_exec "$label" "$script"
  else
    idrive_cmd "$label" provider rename "$from" "$to" >/dev/null
  fi
}

remove_remote() {
  local label="$1"
  local rel
  rel="$(vpath "$2")"
  local kind
  local idrive
  local config
  local script
  kind="$(host_value "$label" kind)"
  idrive="$(host_value "$label" idrive)"
  config="$(host_value "$label" config)"
  if projection_enabled "$label"; then
    if [[ "$kind" == "windows" ]]; then
      script="
\$ErrorActionPreference = 'Stop'
\$root = Join-Path \$HOME 'Iris Drive'
\$path = Join-Path \$root ($(ps_quote "$rel") -replace '/', [IO.Path]::DirectorySeparatorChar)
if (Test-Path -LiteralPath \$path) { Remove-Item -LiteralPath \$path -Recurse -Force }
"
    else
      script="
set -Eeuo pipefail
root=$(sh_quote "$(host_value "$label" work)")
rm -rf \"\$root/$rel\"
"
    fi
    remote_exec "$label" "$script"
  elif [[ "$kind" == "windows" ]]; then
    script="
\$idrive = $(ps_quote "$idrive")
\$config = $(ps_quote "$config")
Set-Variable -Name output -Value (& \$idrive --config-dir \$config provider delete $(ps_quote "$rel") 2>&1)
if (\$LASTEXITCODE -ne 0 -and (\$output -notmatch 'not found|NotFound')) {
  throw \$output
}
"
  else
    script="
set -Eeuo pipefail
idrive=$(sh_quote "$idrive")
config=$(sh_quote "$config")
if ! output=\"\$(\"\$idrive\" --config-dir \"\$config\" provider delete $(sh_quote "$rel") 2>&1 >/dev/null)\"; then
  [[ \"\$output\" == *\"not found\"* || \"\$output\" == *\"NotFound\"* ]] || {
    printf '%s\n' \"\$output\" >&2
    exit 1
  }
fi
"
  fi
  remote_exec "$label" "$script"
}

snapshot() {
  local label="$1"
  idrive_cmd "$label" list |
    jq -r '.files[] | [.sha256, (.size | tostring), .path] | @tsv' |
    LC_ALL=C sort
}

projection_snapshot() {
  local label="$1"
  local kind
  kind="$(host_value "$label" kind)"
  if [[ "$kind" != "posix" ]] || ! projection_enabled "$label"; then
    return 0
  fi
  local root
  root="$(host_value "$label" work)"
  local prefix="e2e/$RUN_ID"
  local script="
set -Eeuo pipefail
root=$(sh_quote "$root")
prefix=$(sh_quote "$prefix")
python3 - \"\$root\" \"\$prefix\" <<'PY' | LC_ALL=C sort
import hashlib, os, sys
from pathlib import Path

root = Path(sys.argv[1])
prefix = sys.argv[2].strip('/')
base = root / prefix
if not base.exists():
    raise SystemExit(0)
for path in sorted(p for p in base.rglob('*') if p.is_file()):
    data = path.read_bytes()
    rel = path.relative_to(root).as_posix()
    print(f\"{hashlib.sha256(data).hexdigest()}\\t{len(data)}\\t{rel}\")
PY
"
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

projection_snapshots_match_expected() {
  local host_label current
  if [[ "$PROVIDER_MUTATIONS" == "1" ]]; then
    return 0
  fi
  for host_label in "${LABELS[@]}"; do
    [[ "$(host_value "$host_label" kind)" == "posix" ]] || continue
    projection_enabled "$host_label" || continue
    current="$(projection_snapshot "$host_label")"
    if [[ "$current" != "$EXPECTED_SNAPSHOT" ]]; then
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
  snapshots_match_expected &&
    source_root_matches_expected_count &&
    projection_snapshots_match_expected
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

windows_projection_not_stale() {
  local logical_path="$1"
  local stale_content="$2"
  local rel
  rel="$(vpath "$logical_path")"
  local b64
  b64="$(printf "%s" "$stale_content" | base64 | tr -d '\n')"
  local script="
\$ErrorActionPreference = 'Stop'
\$root = Join-Path \$HOME 'Iris Drive'
\$path = Join-Path \$root ($(ps_quote "$rel") -replace '/', [IO.Path]::DirectorySeparatorChar)
if (-not (Test-Path -LiteralPath \$path -PathType Leaf)) { exit 0 }
\$actual = [IO.File]::ReadAllBytes(\$path)
\$stale = [Convert]::FromBase64String($(ps_quote "$b64"))
if (\$actual.Length -eq \$stale.Length) {
  for (\$i = 0; \$i -lt \$actual.Length; \$i++) {
    if (\$actual[\$i] -ne \$stale[\$i]) { exit 0 }
  }
  throw \"Windows projection still has stale bytes at $rel\"
}
"
  remote_exec "$windows_label" "$script"
}

step_windows_projection_replaces_materialized_remote_edit() {
  if [[ -z "$windows_label" || -z "$ubuntu_label" ]]; then
    echo "skip: windows+ubuntu projection stale-edit check needs both labels"
    return 0
  fi
  if ! projection_enabled "$windows_label" || ! projection_enabled "$ubuntu_label"; then
    echo "skip: projection stale-edit check disabled"
    return 0
  fi

  write_file "$windows_label" "projection/materialized-edit.txt" "old bytes"
  wait_for_source_snapshot "$windows_label" "windows materialized edit baseline"
  write_file "$ubuntu_label" "projection/materialized-edit.txt" "new bytes"
  wait_for_source_snapshot "$ubuntu_label" "ubuntu remote edit over windows materialized file"
  windows_projection_not_stale "projection/materialized-edit.txt" "old bytes"
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

step_source_restart_delete() {
  write_file "$source_label" "stopped-source-delete/from-source.txt" "delete while $source_label is stopped"
  wait_for_source_snapshot "$source_label" "source restart delete baseline"
  remove_remote "$source_label" "stopped-source-delete/from-source.txt"
  wait_for_source_snapshot "$source_label" "source restart delete"
  stop_daemon "$source_label"
  start_daemon "$source_label"
  wait_until "source daemon fresh after restart" all_fresh
  wait_for_source_snapshot "$source_label" "source restart delete after restart"
}

step_concurrent_same_path_edits() {
  CONCURRENT_SOURCE_CONTENT="concurrent edit from $source_label in $RUN_ID"
  CONCURRENT_TARGET_CONTENT="concurrent edit from $target_label in $RUN_ID"

  write_file "$source_label" "conflicts/concurrent.txt" "concurrent baseline in $RUN_ID"
  wait_for_source_snapshot "$source_label" "concurrent edit baseline"

  write_file "$source_label" "conflicts/concurrent.txt" "$CONCURRENT_SOURCE_CONTENT" &
  local source_pid="$!"
  write_file "$target_label" "conflicts/concurrent.txt" "$CONCURRENT_TARGET_CONTENT" &
  local target_pid="$!"
  wait "$source_pid"
  wait "$target_pid"

  wait_for_converged_union "concurrent edit convergence"
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

if [[ -z "${IRIS_DRIVE_E2E_MOUNT_LABELS+x}" ]]; then
  MOUNT_LABELS=""
  for label in "${LABELS[@]}"; do
    if [[ "$(host_value "$label" kind)" == "posix" ]]; then
      MOUNT_LABELS+="${MOUNT_LABELS:+ }$label"
    fi
  done
fi

echo "run id: $RUN_ID"
echo "hosts: ${LABELS[*]}"
if [[ "$PROVIDER_MUTATIONS" == "1" ]]; then
  echo "mutation surface: provider commands"
else
  echo "mutation surface: projections (windows Cloud Files root; POSIX mounts: ${MOUNT_LABELS:-none})"
fi

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

if [[ "$SIDELOAD_APPKEYS" == "1" ]]; then
  echo "side-loading approved AppKeys into peer temp configs"
  appkeys_b64="$(owner_app_keys_b64 "$owner_label")"
  for label in "${LABELS[@]}"; do
    if [[ "$label" == "$owner_label" ]]; then
      continue
    fi
    sideload_app_keys "$label" "$appkeys_b64"
  done
fi

for label in "${LABELS[@]}"; do
  start_daemon "$label"
done

run_step "authorization" wait_until "all devices authorized" all_authorized
run_step "fresh daemons" wait_until "all daemon statuses fresh" all_fresh

for label in "${LABELS[@]}"; do
  write_file "$label" "seed/$label.txt" "seed from $label in $RUN_ID
"
  write_file "$label" "shared/same.txt" "same bytes from all devices
"
done

run_step "initial multi-device merge" wait_for_converged_union "initial merge"

run_step "direct FIPS peer discovery" wait_until "every device has a direct peer" all_have_direct_peer

run_step "create edit rename delete from $source_label" step_create_edit_rename_delete
run_step "nested create/delete from $target_label" step_nested_create_delete
run_step "windows projection materialized remote edit" step_windows_projection_replaces_materialized_remote_edit
run_step "file type replacements" step_file_type_replacements
run_step "seafile-style rename chain" step_rename_chain
run_step "ignored desktop/editor noise" step_ignored_noise
run_step "receiver restart convergence" step_receiver_restart
run_step "source restart delete propagation" step_source_restart_delete
run_step "same-path concurrent edits" step_concurrent_same_path_edits
run_step "many small files" step_many_small_files
run_step "large file" step_large_file

run_step "final fresh daemons" wait_until "all daemon statuses fresh" all_fresh
run_step "final direct FIPS peer discovery" wait_until "every device has a direct peer" all_have_direct_peer

echo
echo "cross-vm e2e passed for: ${LABELS[*]}"
