#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/cross-vm-e2e.sh --host LABEL=KIND:SSH_HOST [--host ...]
  scripts/cross-vm-three-platform-e2e.sh

KIND is one of:
  posix     Linux or macOS host reachable by SSH with bash, or host "local"
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
  IRIS_DRIVE_E2E_SETUP_REMOTE_TIMEOUT_SECS
                                  Per host setup/build SSH command timeout only;
                                  does not affect file-sync convergence (default: 180).
  IRIS_DRIVE_E2E_MANY_FILES      Many-file test count (default: 32).
  IRIS_DRIVE_E2E_LARGE_BYTES     Large-file test bytes (default: 262144).
  IRIS_DRIVE_E2E_MOUNT_LABELS    Space-separated POSIX labels that should expose FUSE mounts.
                                  Defaults to every POSIX host in this run.
  IRIS_DRIVE_E2E_PROVIDER_MUTATIONS
                                  Use provider commands instead of projection surfaces when set to 1.
  IRIS_DRIVE_E2E_STATIC_FIPS_HINTS
                                  Add deterministic UDP FIPS peer hints between harness daemons
                                  while keeping LAN discovery enabled (default: 1).
  IRIS_DRIVE_E2E_FIPS_PORT_BASE    First UDP port for deterministic FIPS hints. Each label gets
                                  base + index (default: 32000 + pid modulo 10000).
  IRIS_DRIVE_E2E_WINDOWS_CLOUD_ROOT
                                  Windows Cloud Files root for the daemon; defaults to "off" so
                                  provider-bridge e2e runs do not import the VM user's real
                                  ~/Iris Drive contents. Set to empty to use the production default.
  IRIS_DRIVE_E2E_SIDELOAD_APPKEYS
                                  Copy the owner profile roster snapshot into temp peer configs after approval
                                  so VM file-sync tests do not depend on public relay timing (default: 1).
  IRIS_DRIVE_E2E_PROFILE          Build/use idrive from target/debug or target/release (default: debug).
  IRIS_DRIVE_E2E_IDRIVE           Override idrive path on every host.
  IRIS_DRIVE_E2E_IDRIVE_LABEL     Override one host's idrive path, where LABEL is uppercased and
                                  non-alphanumeric characters become underscores.
  IRIS_DRIVE_E2E_WINDOWS_IDRIVE   Override idrive path for every Windows host.
  IRIS_DRIVE_E2E_POSIX_IDRIVE     Override idrive path for every POSIX host.
  IRIS_DRIVE_E2E_IDLE_CPU_GATE    Sample every host daemon's idle CPU after convergence (default: 1).
  IRIS_DRIVE_IDLE_CPU_WARMUP_SECS Override the post-workload idle CPU settle warmup (default: 90).
  IRIS_DRIVE_E2E_KEEP            Keep remote temp dirs/daemons when set to 1.
USAGE
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_ID="run-$(date +%Y%m%d%H%M%S)-$$"
TIMEOUT_SECS="${IRIS_DRIVE_E2E_TIMEOUT_SECS:-60}"
REMOTE_TIMEOUT_SECS="${IRIS_DRIVE_E2E_REMOTE_TIMEOUT_SECS:-60}"
SETUP_REMOTE_TIMEOUT_SECS="${IRIS_DRIVE_E2E_SETUP_REMOTE_TIMEOUT_SECS:-180}"
POLL_SECS="${IRIS_DRIVE_E2E_POLL_SECS:-3}"
MANY_FILES="${IRIS_DRIVE_E2E_MANY_FILES:-32}"
LARGE_BYTES="${IRIS_DRIVE_E2E_LARGE_BYTES:-262144}"
KEEP="${IRIS_DRIVE_E2E_KEEP:-0}"
MOUNT_LABELS="${IRIS_DRIVE_E2E_MOUNT_LABELS:-}"
SIDELOAD_APPKEYS="${IRIS_DRIVE_E2E_SIDELOAD_APPKEYS:-1}"
PROVIDER_MUTATIONS="${IRIS_DRIVE_E2E_PROVIDER_MUTATIONS:-0}"
IDLE_CPU_GATE="${IRIS_DRIVE_E2E_IDLE_CPU_GATE:-1}"
STATIC_FIPS_HINTS="${IRIS_DRIVE_E2E_STATIC_FIPS_HINTS:-1}"
FIPS_PORT_BASE="${IRIS_DRIVE_E2E_FIPS_PORT_BASE:-$((32000 + ($$ % 10000)))}"
E2E_PROFILE="${IRIS_DRIVE_E2E_PROFILE:-debug}"
case "$E2E_PROFILE" in
  debug | release) ;;
  *) echo "IRIS_DRIVE_E2E_PROFILE must be 'debug' or 'release'." >&2; exit 2 ;;
esac

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
declare -a FIPS_PORTS=()
declare -a FIPS_ADDRS=()
declare -a FIPS_STATIC_PEERS=()
declare -a FIPS_BOOTSTRAP=()
declare -a FIPS_OPEN_DISCOVERY=()
declare -a APP_KEY_NPUBS=()

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
    fips_port) printf "%s" "${FIPS_PORTS[$idx]:-}" ;;
    fips_addr) printf "%s" "${FIPS_ADDRS[$idx]:-}" ;;
    fips_static_peers) printf "%s" "${FIPS_STATIC_PEERS[$idx]:-}" ;;
    fips_bootstrap) printf "%s" "${FIPS_BOOTSTRAP[$idx]:-true}" ;;
    fips_open_discovery) printf "%s" "${FIPS_OPEN_DISCOVERY[$idx]:-16}" ;;
    app_key_npub) printf "%s" "${APP_KEY_NPUBS[$idx]:-}" ;;
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
    fips_port) FIPS_PORTS[$idx]="$value" ;;
    fips_addr) FIPS_ADDRS[$idx]="$value" ;;
    fips_static_peers) FIPS_STATIC_PEERS[$idx]="$value" ;;
    fips_bootstrap) FIPS_BOOTSTRAP[$idx]="$value" ;;
    fips_open_discovery) FIPS_OPEN_DISCOVERY[$idx]="$value" ;;
    app_key_npub) APP_KEY_NPUBS[$idx]="$value" ;;
    *) echo "unknown mutable host field: $field" >&2; exit 1 ;;
  esac
}

bool_true() {
  case "${1:-}" in
    1 | true | TRUE | True | yes | YES | Yes | on | ON | On) return 0 ;;
    *) return 1 ;;
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
  if [[ "$kind" == "windows" || "$label" == mac* || "$label" == darwin* || "$label" == ios* || "$label" == android* ]]; then
    [[ "$kind" == "windows" && "${IRIS_DRIVE_E2E_WINDOWS_PROJECTION_MUTATIONS:-0}" == "1" ]]; return
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
      FIPS_PORTS+=("")
      FIPS_ADDRS+=("")
      FIPS_STATIC_PEERS+=("")
      FIPS_BOOTSTRAP+=("")
      FIPS_OPEN_DISCOVERY+=("")
      APP_KEY_NPUBS+=("")
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

env_key_suffix() {
  printf "%s" "$1" | tr '[:lower:]' '[:upper:]' | sed 's/[^A-Z0-9_]/_/g'
}

host_idrive_override() {
  local label="$1"
  local kind
  local label_key
  local kind_key
  kind="$(host_value "$label" kind)"
  label_key="IRIS_DRIVE_E2E_IDRIVE_$(env_key_suffix "$label")"
  kind_key="IRIS_DRIVE_E2E_$(env_key_suffix "$kind")_IDRIVE"
  if [[ -n "${!label_key:-}" ]]; then
    printf "%s" "${!label_key}"
  elif [[ -n "${!kind_key:-}" ]]; then
    printf "%s" "${!kind_key}"
  else
    printf "%s" "${IRIS_DRIVE_E2E_IDRIVE:-}"
  fi
}

run_remote_exec() {
  local label="$1"
  local script="$2"
  local kind
  local ssh_host
  kind="$(host_value "$label" kind)"
  ssh_host="$(host_value "$label" ssh)"
  if [[ "$kind" == "windows" ]]; then
    printf "%s\n" "$script" | ssh "$ssh_host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -'
  elif [[ "$ssh_host" == "local" ]]; then printf "%s\n" "$script" | bash -se
  else
    printf "%s\n" "$script" | ssh "$ssh_host" 'bash -se'
  fi
}

remote_exec() {
  remote_exec_with_timeout "$1" "$2" "$REMOTE_TIMEOUT_SECS"
}

remote_exec_with_timeout() {
  local label="$1"
  local script="$2"
  local timeout_secs="$3"
  local pid
  local watchdog
  local status

  if (( timeout_secs <= 0 )); then
    run_remote_exec "$label" "$script"
    return
  fi

  run_remote_exec "$label" "$script" &
  pid="$!"
  (
    deadline=$((SECONDS + timeout_secs))
    while kill -0 "$pid" 2>/dev/null; do
      if (( SECONDS >= deadline )); then
        echo "remote command timed out after ${timeout_secs}s on $label" >&2
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

detect_local_fips_addr() {
  local ip=""
  if command -v nvpn >/dev/null 2>&1; then
    ip="$(nvpn status --json 2>/dev/null | python3 -c 'import json,sys; print((json.load(sys.stdin).get("tunnel_ip") or "").split("/")[0])' 2>/dev/null || true)"
  elif [[ -x "$HOME/src/nostr-vpn/target/debug/nvpn" ]]; then
    ip="$("$HOME/src/nostr-vpn/target/debug/nvpn" status --json 2>/dev/null | python3 -c 'import json,sys; print((json.load(sys.stdin).get("tunnel_ip") or "").split("/")[0])' 2>/dev/null || true)"
  fi
  if [[ -z "$ip" && "$(uname -s)" == "Darwin" ]]; then
    ip="$(ipconfig getifaddr en0 2>/dev/null || true)"
  fi
  if [[ -z "$ip" && "$(uname -s)" != "Darwin" ]]; then
    ip="$(hostname -I 2>/dev/null | awk '{print $1}' || true)"
  fi
  printf "%s" "${ip:-127.0.0.1}"
}

detect_host_fips_addr() {
  local label="$1"
  local kind
  local ssh_host
  local key
  local override
  local ip=""
  kind="$(host_value "$label" kind)"
  ssh_host="$(host_value "$label" ssh)"
  key="IRIS_DRIVE_E2E_FIPS_ADDR_$(env_key_suffix "$label")"
  override="${!key:-}"
  if [[ -n "$override" ]]; then
    printf "%s" "${override%%/*}"
    return 0
  fi
  if [[ "$ssh_host" == "local" ]]; then
    detect_local_fips_addr
    return 0
  fi
  if [[ "$kind" == "windows" ]]; then
    ip="$(ssh "$ssh_host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' <<'REMOTE_PS' 2>/dev/null || true
$TunnelIp = (Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -eq 'nvpn' -and $_.IPAddress -like '10.44.*' } | Select-Object -First 1 -ExpandProperty IPAddress)
if (-not $TunnelIp) {
  $Nvpn = (Get-Command nvpn -ErrorAction SilentlyContinue).Source
  if (-not $Nvpn) {
    $Candidate = Join-Path $HOME "src\nostr-vpn\target\debug\nvpn.exe"
    if (Test-Path $Candidate) { $Nvpn = $Candidate }
  }
  if ($Nvpn) {
    try {
      $Status = & $Nvpn status --json | ConvertFrom-Json
      if ($Status.tunnel_ip) { $TunnelIp = (($Status.tunnel_ip -as [string]) -replace "/.*$", "") }
    } catch {}
  }
}
if (-not $TunnelIp) {
  $TunnelIp = (Get-NetIPAddress -AddressFamily IPv4 |
    Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } |
    Select-Object -First 1 -ExpandProperty IPAddress)
}
if ($TunnelIp) { Write-Output $TunnelIp }
REMOTE_PS
)"
  else
    ip="$(ssh "$ssh_host" 'bash -se' <<'REMOTE_SH' 2>/dev/null || true
set -Eeuo pipefail
ip=""
for candidate in \
  "$(command -v nvpn 2>/dev/null || true)" \
  "$HOME/src/nostr-vpn/target/debug/nvpn" \
  "$HOME/src/nostr-vpn/target/aarch64-apple-darwin/debug/nvpn" \
  "/Library/PrivilegedHelperTools/to.nostrvpn.nvpn"
do
  [[ -n "$candidate" && -x "$candidate" ]] || continue
  ip="$("$candidate" status --json 2>/dev/null | python3 -c 'import json,sys; print((json.load(sys.stdin).get("tunnel_ip") or "").split("/")[0])' 2>/dev/null || true)"
  [[ -n "$ip" ]] && break
done
if [[ -z "$ip" ]]; then
  ip="$(hostname -I 2>/dev/null | awk '{print $1}' || true)"
fi
printf '%s\n' "$ip"
REMOTE_SH
)"
  fi
  ip="${ip//$'\r'/}"
  ip="$(printf "%s\n" "$ip" | awk 'NF { print $1; exit }')"
  printf "%s" "${ip:-$ssh_host}"
}

configure_fips_static_hints() {
  local mode
  mode="$(printf "%s" "$STATIC_FIPS_HINTS" | tr '[:upper:]' '[:lower:]')"
  case "$mode" in
    ""|1|true|yes|on|auto) ;;
    0|false|no|off|disabled)
      return 0
      ;;
    *) echo "IRIS_DRIVE_E2E_STATIC_FIPS_HINTS must be true/false/auto" >&2; exit 2 ;;
  esac

  local i j label addr port pieces complete peer_key
  for i in "${!LABELS[@]}"; do
    label="${LABELS[$i]}"
    port=$((FIPS_PORT_BASE + i))
    set_host_value "$label" fips_port "$port"
    addr="$(detect_host_fips_addr "$label")"
    set_host_value "$label" fips_addr "$addr"
  done

  for i in "${!LABELS[@]}"; do
    label="${LABELS[$i]}"
    pieces=()
    complete=1
    for j in "${!LABELS[@]}"; do
      [[ "$i" == "$j" ]] && continue
      addr="$(host_value "${LABELS[$j]}" fips_addr)"
      port="$(host_value "${LABELS[$j]}" fips_port)"
      if [[ -z "$addr" || -z "$port" ]]; then
        complete=0
        continue
      fi
      peer_key="$(host_value "${LABELS[$j]}" app_key_npub)"
      peer_key="${peer_key:-${LABELS[$j]}}"
      pieces+=("$peer_key=$addr:$port")
    done
    if [[ ${#pieces[@]} -gt 0 ]]; then
      local IFS=,
      set_host_value "$label" fips_static_peers "${pieces[*]}"
      set_host_value "$label" fips_bootstrap "true"
      set_host_value "$label" fips_open_discovery "16"
      echo "static FIPS hints for $label: $(host_value "$label" fips_static_peers)"
    fi
  done
}

setup_host() {
  local label="$1" kind
  local script meta key value
  kind="$(host_value "$label" kind)"
  local idrive_override
  idrive_override="$(host_idrive_override "$label")"
  if [[ "$kind" == "windows" ]]; then
    script="
\$ErrorActionPreference = 'Stop'
\$label = $(ps_quote "$label")
\$run = $(ps_quote "$RUN_ID")
\$base = Join-Path \$env:TEMP (\"iris-drive-e2e-\$run-\$label\")
if (Test-Path -LiteralPath \$base) { Remove-Item -LiteralPath \$base -Recurse -Force }
\$stale = Get-CimInstance Win32_Process | Where-Object {
  \$_.CommandLine -like '*idrive*' -and
  \$_.CommandLine -like '*--config-dir*' -and
  \$_.CommandLine -like '*iris-drive-e2e-run-*' -and
  \$_.CommandLine -like '* daemon*'
}
foreach (\$proc in \$stale) {
  Stop-Process -Id \$proc.ProcessId -Force -ErrorAction SilentlyContinue
}
\$projectionE2e = Join-Path (Join-Path \$HOME 'Iris Drive') 'e2e'
if (Test-Path -LiteralPath \$projectionE2e) { Remove-Item -LiteralPath \$projectionE2e -Recurse -Force }
\$config = Join-Path \$base 'config'; \$work = Join-Path \$base 'work'
New-Item -ItemType Directory -Force -Path \$config,\$work | Out-Null
\$repo = Join-Path \$HOME 'src\iris-drive'
\$profile = $(ps_quote "$E2E_PROFILE")
\$repoIdrive = Join-Path \$repo (Join-Path (Join-Path 'target' \$profile) 'idrive.exe')
\$overrideIdrive = $(ps_quote "$idrive_override")
\$cargoProfileArgs = @()
if (\$profile -eq 'release') { \$cargoProfileArgs += '--release' }
function Test-IrisDriveCli([string]\$candidate) {
  if ([string]::IsNullOrWhiteSpace(\$candidate) -or -not (Test-Path -LiteralPath \$candidate)) { return \$false }
  & \$candidate app-keys --help *> \$null
  return \$LASTEXITCODE -eq 0
}
\$idrive = \$overrideIdrive
if ([string]::IsNullOrWhiteSpace(\$idrive) -and (Test-IrisDriveCli \$repoIdrive)) {
  \$idrive = \$repoIdrive
}
if ([string]::IsNullOrWhiteSpace(\$idrive)) {
  if (Test-Path -LiteralPath (Join-Path \$repo 'Cargo.toml')) {
    \$cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (\$cargo) {
      Push-Location \$repo
      cargo build -q @cargoProfileArgs -p idrive --bin idrive
      Pop-Location
      \$idrive = \$repoIdrive
    }
  }
}
if ([string]::IsNullOrWhiteSpace(\$idrive) -or -not (Test-IrisDriveCli \$idrive)) {
  \$idrive = \$repoIdrive
}
if (\$profile -eq 'debug' -and -not (Test-IrisDriveCli \$idrive)) {
  \$idrive = Join-Path \$HOME '.cargo\bin\idrive.exe'
  if (-not (Test-Path -LiteralPath \$idrive)) {
    \$cmd = Get-Command idrive.exe -ErrorAction SilentlyContinue
    if (\$cmd) { \$idrive = \$cmd.Source }
  }
}
if (-not (Test-IrisDriveCli \$idrive)) {
  if (Test-Path -LiteralPath (Join-Path \$repo 'Cargo.toml')) {
    Push-Location \$repo
    cargo build -q @cargoProfileArgs -p idrive --bin idrive
    Pop-Location
    \$idrive = \$repoIdrive
  }
}
if (-not (Test-IrisDriveCli \$idrive)) { throw \"current idrive.exe with app-keys support not found for \$label\" }
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
while IFS= read -r stale_pid; do
  [[ -n \"\$stale_pid\" && \"\$stale_pid\" != \"\$\$\" ]] || continue
  kill \"\$stale_pid\" >/dev/null 2>&1 || true
done < <(
  ps -eo pid=,args= |
    awk -v self=\"\$\$\" '\$1 != self && \$0 ~ /\\/idrive[[:space:]]+--config-dir/ && \$0 ~ /iris-drive-e2e-run-/ && \$0 ~ /[[:space:]]daemon([[:space:]]|\$)/ { print \$1 }'
)
rm -rf \"\$base\"
mkdir -p \"\$base/config\" \"\$base/work\"
supports_app_keys() {
  [[ -x \"\$1\" ]] && \"\$1\" app-keys --help >/dev/null 2>&1
}
repo=\"\$HOME/src/iris-drive\"
profile=$(sh_quote "$E2E_PROFILE")
cargo_profile_arg=()
if [[ \"\$profile\" == \"release\" ]]; then
  cargo_profile_arg=(--release)
fi
idrive=$(sh_quote "$idrive_override")
if [[ -z \"\$idrive\" ]]; then
  for candidate in \\
    \"\$repo/target/\$profile/idrive\" \\
    \"\$HOME/.cache/cargo-target/\$profile/idrive\" \\
    \"\${CARGO_TARGET_DIR:+\$CARGO_TARGET_DIR/\$profile/idrive}\"
  do
    if supports_app_keys \"\$candidate\"; then
      idrive=\"\$candidate\"
      break
    fi
  done
fi
if [[ -z \"\$idrive\" && \"\$profile\" == \"debug\" ]]; then
  for candidate in \"\$HOME/.cargo/bin/idrive\" \"\$(command -v idrive || true)\"; do
    if supports_app_keys \"\$candidate\"; then
      idrive=\"\$candidate\"
      break
    fi
  done
fi
if ! supports_app_keys \"\$idrive\" && [[ -f \"\$repo/Cargo.toml\" ]] && command -v cargo >/dev/null 2>&1; then
  (cd \"\$repo\" && cargo build -q \"\${cargo_profile_arg[@]}\" -p idrive --bin idrive)
  idrive=\"\$repo/target/\$profile/idrive\"
  [[ -x \"\$idrive\" ]] || idrive=\"\$HOME/.cache/cargo-target/\$profile/idrive\"
  [[ -x \"\$idrive\" ]] || idrive=\"\${CARGO_TARGET_DIR:+\$CARGO_TARGET_DIR/\$profile/idrive}\"
fi
if ! supports_app_keys \"\$idrive\"; then
  idrive=\"\${CARGO_TARGET_DIR:+\$CARGO_TARGET_DIR/\$profile/idrive}\"; idrive=\"\${idrive:-\$repo/target/\$profile/idrive}\"
fi
supports_app_keys \"\$idrive\" || idrive=\"\$HOME/.cache/cargo-target/\$profile/idrive\"
if [[ \"\$profile\" == \"debug\" ]]; then
  supports_app_keys \"\$idrive\" || idrive=\"\$HOME/.cargo/bin/idrive\"
  supports_app_keys \"\$idrive\" || idrive=\"\$(command -v idrive || true)\"
fi
if ! supports_app_keys \"\$idrive\"; then
  if [[ -f \"\$repo/Cargo.toml\" ]] && command -v cargo >/dev/null 2>&1; then
    (cd \"\$repo\" && cargo build -q \"\${cargo_profile_arg[@]}\" -p idrive --bin idrive)
    idrive=\"\$repo/target/\$profile/idrive\"
    [[ -x \"\$idrive\" ]] || idrive=\"\$HOME/.cache/cargo-target/\$profile/idrive\"
  fi
fi
if ! supports_app_keys \"\$idrive\"; then
  echo \"current idrive with app-keys support not found for \$label\" >&2
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
  meta="$(remote_exec_with_timeout "$label" "$script" "$SETUP_REMOTE_TIMEOUT_SECS")"
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
if ([string]::IsNullOrWhiteSpace(\$idrive) -or -not (Test-Path -LiteralPath \$idrive)) {
  \$cmd = Get-Command idrive.exe -ErrorAction SilentlyContinue
  if (-not \$cmd) { throw 'idrive.exe not found' }
  \$idrive = \$cmd.Source
}
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

owner_profile_roster_ops_b64() {
  local label="$1"
  local kind
  local config
  local script
  kind="$(host_value "$label" kind)"
  config="$(host_value "$label" config)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$config = $(ps_quote "$config")
\$sidecar = Join-Path \$config 'profile-roster-events.json'
if (Test-Path -LiteralPath \$sidecar) {
  [Convert]::ToBase64String([IO.File]::ReadAllBytes(\$sidecar))
} else {
  \$text = Get-Content -LiteralPath (Join-Path \$config 'config.toml') -Raw
  \$match = [regex]::Match(\$text, '(?s)\\[\\[profile\\.profile_roster_ops\\]\\].*?(?=\\r?\\n\\[\\[drives\\]\\])')
  if (-not \$match.Success) { throw 'owner profile roster ops block not found' }
  [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(\$match.Value))
}
"
  else
    script="
set -Eeuo pipefail
config=$(sh_quote "$config")
sidecar=\"\$config/profile-roster-events.json\"
if [[ -f \"\$sidecar\" ]]; then
  base64 <\"\$sidecar\" | tr -d '\\n'
else
  awk 'BEGIN{copy=0} /^\\[\\[profile\\.profile_roster_ops\\]\\]/{copy=1} /^\\[\\[drives\\]\\]/{copy=0} copy{print}' \"\$config/config.toml\" | base64 | tr -d '\\n'
fi
"
  fi
  remote_exec "$label" "$script" | tr -d '\r\n'
}

sideload_profile_roster_ops() {
  local label="$1"
  local roster_ops_b64="$2"
  local owner_profile_id="$3"
  local kind
  local config
  local script
  kind="$(host_value "$label" kind)"
  config="$(host_value "$label" config)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$config = $(ps_quote "$config")
\$path = Join-Path \$config 'config.toml'
\$ownerProfileId = $(ps_quote "$owner_profile_id")
\$rosterOps = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($(ps_quote "$roster_ops_b64")))
\$text = Get-Content -LiteralPath \$path -Raw
\$text = \$text.Replace('authorization_state = \"awaiting_approval\"', 'authorization_state = \"authorized\"')
\$text = [regex]::Replace(\$text, '(?m)^(profile_id\\s*=\\s*)\"[^\"]+\"', { param(\$m) \$m.Groups[1].Value + [char]34 + \$ownerProfileId + [char]34 }, 1)
\$text = [regex]::Replace(\$text, '(?m)^(root_scope_id\\s*=\\s*)\"[^\"]+\"', { param(\$m) \$m.Groups[1].Value + [char]34 + \$ownerProfileId + [char]34 })
\$text = [regex]::Replace(\$text, '(?ms)^\\[profile\\.outbound_app_key_link_request\\]\\r?\\n.*?(?=^\\[)', '', 1)
\$lf = [string][char]10
\$trimmed = \$rosterOps.TrimStart()
\$utf8 = [Text.UTF8Encoding]::new(\$false)
if (\$trimmed.StartsWith('{') -and \$trimmed.Contains('\"events\"')) {
  [IO.File]::WriteAllText((Join-Path \$config 'profile-roster-events.json'), \$rosterOps, \$utf8)
} else {
  \$pattern = '(?s)\\r?\\n\\[\\[profile\\.profile_roster_ops\\]\\].*?(?=\\r?\\n\\[\\[drives\\]\\])'
  if ([regex]::IsMatch(\$text, \$pattern)) {
    \$text = [regex]::Replace(\$text, \$pattern, (\$lf + \$rosterOps + \$lf), 1)
  } else {
    \$text = \$text.Replace(\$lf + '[[drives]]', \$lf + \$rosterOps + \$lf + '[[drives]]')
  }
}
[IO.File]::WriteAllText(\$path, \$text, \$utf8)
"
  else
    script="
set -Eeuo pipefail
CONFIG_PATH=$(sh_quote "$config") ROSTER_OPS_B64=$(sh_quote "$roster_ops_b64") OWNER_PROFILE_ID=$(sh_quote "$owner_profile_id") python3 - <<'PY'
import base64, json, os, re
from pathlib import Path

path = Path(os.environ['CONFIG_PATH']) / 'config.toml'
roster_ops = base64.b64decode(os.environ['ROSTER_OPS_B64']).decode()
owner_profile_id = os.environ['OWNER_PROFILE_ID']
text = path.read_text()
text = text.replace('authorization_state = \"awaiting_approval\"', 'authorization_state = \"authorized\"')
text = re.sub(
    r'(?m)^(profile_id\\s*=\\s*)\"[^\"]+\"',
    lambda match: f'{match.group(1)}\"{owner_profile_id}\"',
    text,
    count=1,
)
text = re.sub(
    r'(?m)^(root_scope_id\\s*=\\s*)\"[^\"]+\"',
    lambda match: f'{match.group(1)}\"{owner_profile_id}\"',
    text,
)
text = re.sub(r'(?ms)^\\[profile\\.outbound_app_key_link_request\\]\\n.*?(?=^\\[)', '', text, count=1)
try:
    parsed = json.loads(roster_ops)
except json.JSONDecodeError:
    parsed = None
if isinstance(parsed, dict) and isinstance(parsed.get('events'), list):
    (path.parent / 'profile-roster-events.json').write_text(roster_ops)
else:
    text = re.sub(r'\\n\\[\\[profile\\.profile_roster_ops\\]\\].*?(?=\\n\\[\\[drives\\]\\])', '\\n' + roster_ops + '\\n', text, count=1, flags=re.S)
    if '[[profile.profile_roster_ops]]' not in text:
        text = text.replace('\\n[[drives]]', '\\n' + roster_ops + '\\n[[drives]]', 1)
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
  local windows_cloud_root
  local fips_port
  local fips_addr
  local fips_static_peers
  local fips_bootstrap
  local fips_open_discovery
  kind="$(host_value "$label" kind)"
  idrive="$(host_value "$label" idrive)"
  config="$(host_value "$label" config)"
  log="$(host_value "$label" log)"
  err="$(host_value "$label" err)"
  work="$(host_value "$label" work)"
  pidfile="$(host_value "$label" pid)"
  fips_port="$(host_value "$label" fips_port)"
  fips_addr="$(host_value "$label" fips_addr)"
  fips_static_peers="$(host_value "$label" fips_static_peers)"
  fips_bootstrap="$(host_value "$label" fips_bootstrap)"
  fips_open_discovery="$(host_value "$label" fips_open_discovery)"
  if [[ "$kind" == "windows" ]]; then
    windows_cloud_root="${IRIS_DRIVE_E2E_WINDOWS_CLOUD_ROOT:-off}"
    if [[ "${IRIS_DRIVE_E2E_WINDOWS_PROJECTION_MUTATIONS:-0}" == "1" && -z "${IRIS_DRIVE_E2E_WINDOWS_CLOUD_ROOT+x}" ]]; then
      windows_cloud_root=""
    fi
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
\$env:IRIS_DRIVE_WINDOWS_CLOUD_ROOT = $(ps_quote "$windows_cloud_root")
\$env:IRIS_DRIVE_FIPS_UDP_BIND_ADDR = $(ps_quote "${fips_port:+0.0.0.0:$fips_port}")
\$env:IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR = $(ps_quote "${fips_addr:+$fips_addr:$fips_port}")
\$env:IRIS_DRIVE_FIPS_UDP_PUBLIC = 'false'
\$env:IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY = 'true'
\$env:IRIS_DRIVE_FIPS_ENABLE_WEBRTC = 'true'
\$env:IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP = $(ps_quote "$fips_bootstrap")
\$env:IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING = $(ps_quote "$fips_open_discovery")
\$env:IRIS_DRIVE_FIPS_STATIC_PEERS = $(ps_quote "$fips_static_peers")
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
    printf "%s\n" "$script" | ssh "$ssh_host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' >/dev/null 2>&1 &
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
mount_enabled=0; case "$label" in mac*|darwin*|ios*|android*) mount_labels="" ;; esac
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
  nohup env \\
    IRIS_DRIVE_FIPS_UDP_BIND_ADDR=$(sh_quote "${fips_port:+0.0.0.0:$fips_port}") \\
    IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=$(sh_quote "${fips_addr:+$fips_addr:$fips_port}") \\
    IRIS_DRIVE_FIPS_UDP_PUBLIC=false \\
    IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY=true \\
    IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true \\
    IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=$(sh_quote "$fips_bootstrap") \\
    IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=$(sh_quote "$fips_open_discovery") \\
    IRIS_DRIVE_FIPS_STATIC_PEERS=$(sh_quote "$fips_static_peers") \\
    \"\$idrive\" --config-dir \"\$config\" daemon --watch-debounce-ms 100 --gateway-port 0 --mount --mountpoint \"\$work\"$(daemon_relay_args_posix) >\"\$log\" 2>\"\$err\" < /dev/null &
else
  nohup env \\
    IRIS_DRIVE_FIPS_UDP_BIND_ADDR=$(sh_quote "${fips_port:+0.0.0.0:$fips_port}") \\
    IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=$(sh_quote "${fips_addr:+$fips_addr:$fips_port}") \\
    IRIS_DRIVE_FIPS_UDP_PUBLIC=false \\
    IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY=true \\
    IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true \\
    IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=$(sh_quote "$fips_bootstrap") \\
    IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=$(sh_quote "$fips_open_discovery") \\
    IRIS_DRIVE_FIPS_STATIC_PEERS=$(sh_quote "$fips_static_peers") \\
    \"\$idrive\" --config-dir \"\$config\" daemon --watch-debounce-ms 100 --gateway-port 0$(daemon_relay_args_posix) >\"\$log\" 2>\"\$err\" < /dev/null &
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
  local label="$1" prefix="e2e/$RUN_ID/"
  idrive_cmd "$label" list |
    jq -r --arg prefix "$prefix" '.files[] | select(.path | startswith($prefix)) | [.sha256, (.size | tostring), .path] | @tsv' |
    LC_ALL=C sort
}

snapshot_all_once() {
  local out_dir="$1"
  local label pid status
  local -a labels=()
  local -a pids=()
  mkdir -p "$out_dir"
  for label in "${LABELS[@]}"; do
    (
      snapshot "$label" >"$out_dir/$label.snapshot"
    ) &
    labels+=("$label")
    pids+=("$!")
  done
  status=0
  for i in "${!pids[@]}"; do
    if ! wait "${pids[$i]}"; then
      echo "snapshot failed for ${labels[$i]}" >&2
      status=1
    fi
  done
  return "$status"
}

filter_ignored_snapshot_paths() {
  python3 -c '
import sys

ignored_names = {".ds_store", ".hashtree", ".trash", "$recycle.bin", "thumbs.db", "desktop.ini"}

def ignored(name: str) -> bool:
    lower = name.lower()
    return (
        lower in ignored_names
        or name.startswith("._")
        or lower.startswith(".trash-")
        or name.endswith("~")
        or (name.startswith("#") and name.endswith("#"))
        or lower.endswith(".sbak")
    )

for line in sys.stdin:
    path = line.rstrip("\n").split("\t")[-1]
    if any(ignored(part) for part in path.split("/")):
        continue
    sys.stdout.write(line)
'
}

projection_snapshot() {
  local label="$1"
  local kind
  kind="$(host_value "$label" kind)"
  if ! projection_enabled "$label"; then
    return 0
  fi
  local prefix="e2e/$RUN_ID"
  if [[ "$kind" == "windows" ]]; then
    local script="
\$ErrorActionPreference = 'Stop'
\$root = Join-Path \$HOME 'Iris Drive'
\$prefix = $(ps_quote "$prefix")
\$base = Join-Path \$root (\$prefix -replace '/', [IO.Path]::DirectorySeparatorChar)
if (-not (Test-Path -LiteralPath \$base -PathType Container)) { exit 0 }
Get-ChildItem -LiteralPath \$base -File -Recurse -Force | ForEach-Object {
  \$relative = \$_.FullName.Substring(\$root.Length).TrimStart([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar).Replace('\\', '/')
  \$hash = (Get-FileHash -LiteralPath \$_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  Write-Output (\"\$hash\`t\$([string]\$_.Length)\`t\$relative\")
} | Sort-Object
"
    remote_exec "$label" "$script" | tr -d '\r' | filter_ignored_snapshot_paths
    return
  fi
  local root
  root="$(host_value "$label" work)"
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
  remote_exec "$label" "$script" | tr -d '\r' | filter_ignored_snapshot_paths
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
    idrive_cmd "$label" status |
      jq '{
        daemon: {
          running: .daemon.running,
          fresh: .daemon.fresh
        },
        summary: {
          file_count: .summary.file_count,
          sync_status: .summary.sync_status,
          sync_status_label: .summary.sync_status_label,
          online_app_key_count: .summary.online_app_key_count
        },
        fips: {
          state: .network.fips.state,
          state_label: .network.fips.state_label,
          endpoint_npub: .network.fips.endpoint_npub,
          roster_peer_count: .network.fips.roster_peer_count,
          roster_online_peer_count: .network.fips.roster_online_peer_count,
          roster_connected_peer_count: .network.fips.roster_connected_peer_count,
          connected_peer_count: .network.fips.connected_peer_count,
          other_peer_count: .network.fips.other_peer_count,
          error: .network.fips.error
        },
        peers: [
          .peers[]? | {
            label,
            connection_state,
            sync_state,
            root_available,
            root_cid,
            last_block_sync
          }
        ]
      }' >&2 || true
    echo "---- $label snapshot ----" >&2
    snapshot "$label" >&2 || true
    if [[ -n "${EXPECTED_SNAPSHOT:-}" ]]; then
      local current missing extra
      current="$(snapshot "$label" || true)"
      missing="$(
        comm -23 \
          <(printf "%s\n" "$EXPECTED_SNAPSHOT" | sed '/^$/d') \
          <(printf "%s\n" "$current" | sed '/^$/d') || true
      )"
      extra="$(
        comm -13 \
          <(printf "%s\n" "$EXPECTED_SNAPSHOT" | sed '/^$/d') \
          <(printf "%s\n" "$current" | sed '/^$/d') || true
      )"
      if [[ -n "$missing" || -n "$extra" ]]; then
        echo "---- $label snapshot diff ----" >&2
        if [[ -n "$missing" ]]; then
          echo "missing:" >&2
          printf "%s\n" "$missing" | sed -n '1,40p' >&2
        fi
        if [[ -n "$extra" ]]; then
          echo "extra:" >&2
          printf "%s\n" "$extra" | sed -n '1,40p' >&2
        fi
      fi
      if [[ "$PROVIDER_MUTATIONS" != "1" ]] && projection_enabled "$label"; then
        local projection_current projection_missing projection_extra
        projection_current="$(projection_snapshot "$label" || true)"
        projection_missing="$(
          comm -23 \
            <(printf "%s\n" "$EXPECTED_SNAPSHOT" | sed '/^$/d') \
            <(printf "%s\n" "$projection_current" | sed '/^$/d') || true
        )"
        projection_extra="$(
          comm -13 \
            <(printf "%s\n" "$EXPECTED_SNAPSHOT" | sed '/^$/d') \
            <(printf "%s\n" "$projection_current" | sed '/^$/d') || true
        )"
        if [[ -n "$projection_missing" || -n "$projection_extra" ]]; then
          echo "---- $label projection snapshot diff ----" >&2
          if [[ -n "$projection_missing" ]]; then
            echo "missing:" >&2
            printf "%s\n" "$projection_missing" | sed -n '1,40p' >&2
          fi
          if [[ -n "$projection_extra" ]]; then
            echo "extra:" >&2
            printf "%s\n" "$projection_extra" | sed -n '1,40p' >&2
          fi
        fi
      fi
    fi
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
    jq -e '.profile.authorization_state == "authorized"' >/dev/null 2>&1 <<<"$status" || return 1
  done
}

all_fresh() {
  local label status
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e '.daemon.running == true and .daemon.fresh == true' >/dev/null 2>&1 <<<"$status" || return 1
  done
}

all_snapshots_ready() {
  local label status
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e '
      .daemon.running == true and
      .daemon.fresh == true and
      .summary.sync_status == "up to date"
    ' >/dev/null 2>&1 <<<"$status" || return 1
  done
}

all_have_direct_peer() {
  local label status
  local expected_peers
  expected_peers=$((${#LABELS[@]} - 1))
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e --argjson expected "$expected_peers" '
      .network.fips.running == true and
      .network.fips.fresh == true and
      (.network.fips.roster_peer_count // 0) >= $expected and
      (.network.fips.roster_connected_peer_count // 0) >= $expected
    ' >/dev/null 2>&1 <<<"$status" || return 1
  done
}

all_have_roster_peers() {
  local label status
  local expected_peers
  expected_peers=$((${#LABELS[@]} - 1))
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e --argjson expected "$expected_peers" '
      .network.fips.running == true and
      .network.fips.fresh == true and
      (.network.fips.roster_peer_count // 0) >= $expected
    ' >/dev/null 2>&1 <<<"$status" || return 1
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
  local host_label current tmp
  all_snapshots_ready || return 1
  tmp="$(mktemp -d)"
  if ! snapshot_all_once "$tmp"; then
    rm -rf "$tmp"
    return 1
  fi
  for host_label in "${LABELS[@]}"; do
    current="$(cat "$tmp/$host_label.snapshot")"
    if [[ "$current" != "$EXPECTED_SNAPSHOT" ]]; then
      rm -rf "$tmp"
      return 1
    fi
  done
  rm -rf "$tmp"
  return 0
}

snapshots_match_current_union() {
  local expected host_label current tmp
  all_snapshots_ready || return 1
  tmp="$(mktemp -d)"
  if ! snapshot_all_once "$tmp"; then
    rm -rf "$tmp"
    return 1
  fi
  expected="$(cat "$tmp"/*.snapshot | LC_ALL=C sort -u)"
  for host_label in "${LABELS[@]}"; do
    current="$(cat "$tmp/$host_label.snapshot")"
    if [[ "$current" != "$expected" ]]; then
      rm -rf "$tmp"
      return 1
    fi
  done
  rm -rf "$tmp"
  return 0
}

projection_snapshots_match_expected() {
  local host_label current
  if [[ "$PROVIDER_MUTATIONS" == "1" ]]; then
    return 0
  fi
  for host_label in "${LABELS[@]}"; do
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
  EXPECTED_SOURCE_LABEL="$label"
  SOURCE_MATCH_CANDIDATE=""
  SOURCE_MATCH_STABLE_COUNT=0
  wait_until "$step" source_and_snapshots_match_current_source
}

source_visible_snapshot() {
  local label="$1"
  local expected
  if projection_enabled "$label"; then
    projection_snapshot "$label"
  else
    snapshot "$label"
  fi
}

wait_for_source_snapshot_changed() {
  local label="$1"
  local previous="$2"
  local step="$3"
  EXPECTED_SOURCE_LABEL="$label"
  EXPECTED_SOURCE_PREVIOUS_SNAPSHOT="$previous"
  SOURCE_MATCH_CANDIDATE=""
  SOURCE_MATCH_STABLE_COUNT=0
  wait_until "$step" source_and_snapshots_match_current_source_after_change
}

source_and_snapshots_match_current_source() {
  local expected
  expected="$(source_visible_snapshot "$EXPECTED_SOURCE_LABEL")"
  EXPECTED_SNAPSHOT="$expected"
  EXPECTED_SOURCE_FILE_COUNT="$(snapshot_file_count "$expected")"
  source_and_snapshots_match_expected
}

source_and_snapshots_match_current_source_after_change() {
  local expected stable_polls
  expected="$(source_visible_snapshot "$EXPECTED_SOURCE_LABEL")"
  if [[ "$expected" == "$EXPECTED_SOURCE_PREVIOUS_SNAPSHOT" ]]; then
    return 1
  fi
  EXPECTED_SNAPSHOT="$expected"
  EXPECTED_SOURCE_FILE_COUNT="$(snapshot_file_count "$expected")"
  source_and_snapshots_match_expected || return 1
  if [[ "$SOURCE_MATCH_CANDIDATE" == "$expected" ]]; then
    SOURCE_MATCH_STABLE_COUNT=$((SOURCE_MATCH_STABLE_COUNT + 1))
  else
    SOURCE_MATCH_CANDIDATE="$expected"
    SOURCE_MATCH_STABLE_COUNT=1
  fi
  stable_polls="${IRIS_DRIVE_E2E_SOURCE_STABLE_POLLS:-2}"
  [[ "$SOURCE_MATCH_STABLE_COUNT" -ge "$stable_polls" ]]
}

source_root_matches_expected_count() {
  local status
  status="$(idrive_cmd "$EXPECTED_SOURCE_LABEL" status 2>/dev/null || true)"
  jq -e --argjson count "$EXPECTED_SOURCE_FILE_COUNT" \
    '.daemon.running == true and .daemon.fresh == true and .hashtree.file_count >= $count' \
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

run_for_all_labels_parallel() {
  local label status
  local -a labels=()
  local -a pids=()
  local fn="$1"
  for label in "${LABELS[@]}"; do
    (
      "$fn" "$label"
    ) &
    labels+=("$label")
    pids+=("$!")
  done
  status=0
  for i in "${!pids[@]}"; do
    if ! wait "${pids[$i]}"; then
      echo "$fn failed for ${labels[$i]}" >&2
      status=1
    fi
  done
  return "$status"
}

idle_cpu_gate_enabled() {
  bool_true "$IDLE_CPU_GATE"
}

idle_cpu_remote_timeout_secs() {
  if [[ -n "${IRIS_DRIVE_E2E_IDLE_CPU_TIMEOUT_SECS:-}" ]]; then
    printf "%s" "$IRIS_DRIVE_E2E_IDLE_CPU_TIMEOUT_SECS"
    return
  fi
  local warmup="${IRIS_DRIVE_IDLE_CPU_WARMUP_SECS:-90}"
  local duration="${IRIS_DRIVE_IDLE_CPU_DURATION_SECS:-60}"
  printf "%s" $((warmup + duration + 90))
}

idle_cpu_gate_label() {
  local label="$1"
  local kind config timeout script repo_line
  kind="$(host_value "$label" kind)"
  config="$(host_value "$label" config)"
  timeout="$(idle_cpu_remote_timeout_secs)"
  if [[ "$kind" == "windows" ]]; then
    script="
\$ErrorActionPreference = 'Stop'
\$repo = Join-Path \$HOME 'src\iris-drive'
\$env:IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES = 'daemon'
\$env:IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH = $(ps_quote "$config")
\$env:IRIS_DRIVE_IDLE_CPU_WARMUP_SECS = $(ps_quote "${IRIS_DRIVE_IDLE_CPU_WARMUP_SECS:-90}")
\$env:IRIS_DRIVE_IDLE_CPU_DURATION_SECS = $(ps_quote "${IRIS_DRIVE_IDLE_CPU_DURATION_SECS:-60}")
\$env:IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS = $(ps_quote "${IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS:-5}")
\$env:IRIS_DRIVE_IDLE_CPU_DAEMON_MAX = $(ps_quote "${IRIS_DRIVE_IDLE_CPU_DAEMON_MAX:-10}")
& (Join-Path \$repo 'scripts\idle-cpu-gate-windows.ps1')
exit \$LASTEXITCODE
"
  else
    if [[ "$(host_value "$label" ssh)" == "local" ]]; then
      repo_line="repo=$(sh_quote "$ROOT")"
    else
      repo_line='repo="$HOME/src/iris-drive"'
    fi
    script="
set -Eeuo pipefail
${repo_line}
export IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES=daemon
export IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH=$(sh_quote "$config")
export IRIS_DRIVE_IDLE_CPU_WARMUP_SECS=$(sh_quote "${IRIS_DRIVE_IDLE_CPU_WARMUP_SECS:-90}")
export IRIS_DRIVE_IDLE_CPU_DURATION_SECS=$(sh_quote "${IRIS_DRIVE_IDLE_CPU_DURATION_SECS:-60}")
export IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS=$(sh_quote "${IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS:-5}")
export IRIS_DRIVE_IDLE_CPU_DAEMON_MAX=$(sh_quote "${IRIS_DRIVE_IDLE_CPU_DAEMON_MAX:-10}")
\"\$repo/scripts/idle-cpu-gate.sh\" --platform auto
"
  fi
  remote_exec_with_timeout "$label" "$script" "$timeout"
}

write_initial_seed_files() {
  local label="$1"
  write_file "$label" "seed/$label.txt" "seed from $label in $RUN_ID
"
  write_file "$label" "shared/same.txt" "same bytes from all devices
"
}

step_create_edit_rename_delete() {
  local before
  before="$(source_visible_snapshot "$source_label")"
  write_file "$source_label" "ops/create-edit.txt" "version 1 from $source_label"
  wait_for_source_snapshot_changed "$source_label" "$before" "create from source"
  before="$(source_visible_snapshot "$source_label")"
  write_file "$source_label" "ops/create-edit.txt" "version 2 from $source_label"
  wait_for_source_snapshot_changed "$source_label" "$before" "edit from source"
  before="$(source_visible_snapshot "$source_label")"
  rename_remote "$source_label" "ops/create-edit.txt" "ops/renamed.txt"
  wait_for_source_snapshot_changed "$source_label" "$before" "rename from source"
  before="$(source_visible_snapshot "$source_label")"
  remove_remote "$source_label" "ops/renamed.txt"
  wait_for_source_snapshot_changed "$source_label" "$before" "delete from source"
}

step_nested_create_delete() {
  local before
  before="$(source_visible_snapshot "$target_label")"
  write_file "$target_label" "download/dir1/one.txt" "nested from $target_label"
  rename_remote "$target_label" "download/dir1" "download/dir2"
  wait_for_source_snapshot_changed "$target_label" "$before" "nested rename"
  before="$(source_visible_snapshot "$target_label")"
  remove_remote "$target_label" "download/dir2/one.txt"
  remove_remote "$target_label" "download/dir2"
  wait_for_source_snapshot_changed "$target_label" "$before" "nested delete"
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

step_windows_projection_replaces_stale_remote_edit() {
  if [[ -z "$windows_label" || -z "$ubuntu_label" ]]; then
    echo "skip: windows+ubuntu projection stale-edit check needs both labels"
    return 0
  fi
  if ! projection_enabled "$windows_label" || ! projection_enabled "$ubuntu_label"; then
    echo "skip: projection stale-edit check disabled"
    return 0
  fi

  local before
  before="$(source_visible_snapshot "$windows_label")"
  write_file "$windows_label" "projection/projected-edit.txt" "old bytes"
  wait_for_source_snapshot_changed "$windows_label" "$before" "windows projected edit baseline"
  before="$(source_visible_snapshot "$ubuntu_label")"
  write_file "$ubuntu_label" "projection/projected-edit.txt" "new bytes"
  wait_for_source_snapshot_changed "$ubuntu_label" "$before" "ubuntu remote edit over windows projected file"
  windows_projection_not_stale "projection/projected-edit.txt" "old bytes"
}

step_file_type_replacements() {
  local before
  before="$(source_visible_snapshot "$source_label")"
  write_file "$source_label" "types/file-to-dir" "old file"
  write_file "$source_label" "types/dir-to-file/old.txt" "old child"
  wait_for_source_snapshot_changed "$source_label" "$before" "initial file type setup"
  before="$(source_visible_snapshot "$source_label")"
  remove_remote "$source_label" "types/file-to-dir"
  write_file "$source_label" "types/file-to-dir/new.txt" "new child"
  remove_remote "$source_label" "types/dir-to-file"
  write_file "$source_label" "types/dir-to-file" "new file"
  wait_for_source_snapshot_changed "$source_label" "$before" "file type replacements"
}

step_rename_chain() {
  local before
  before="$(source_visible_snapshot "$source_label")"
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
  wait_for_source_snapshot_changed "$source_label" "$before" "rename chain"
}

step_ignored_noise() {
  local before
  before="$(source_visible_snapshot "$source_label")"
  write_file "$source_label" "noise/keep.txt" "keep"
  write_file "$source_label" "noise/.DS_Store" "finder"
  write_file "$source_label" "noise/._keep.txt" "resource fork"
  write_file "$source_label" "noise/Thumbs.db" "thumbs"
  write_file "$source_label" "noise/desktop.ini" "desktop"
  write_file "$source_label" "noise/draft~" "backup"
  write_file "$source_label" "noise/#draft#" "emacs"
  write_file "$source_label" "noise/backup.sbak" "seafile backup"
  write_file "$source_label" ".hashtree/prev" "internal"
  wait_for_source_snapshot_changed "$source_label" "$before" "ignored noise"
}

step_receiver_restart() {
  local i
  local before
  before="$(source_visible_snapshot "$source_label")"
  stop_daemon "$target_label"
  for i in $(seq 1 12); do
    write_file "$source_label" "reconnect/file-$i.txt" "file $i while $target_label stopped"
  done
  start_daemon "$target_label"
  wait_until "target daemon fresh after restart" all_fresh
  wait_for_source_snapshot_changed "$source_label" "$before" "receiver restart"
}

step_source_restart_delete() {
  local before
  before="$(source_visible_snapshot "$source_label")"
  write_file "$source_label" "stopped-source-delete/from-source.txt" "delete while $source_label is stopped"
  wait_for_source_snapshot_changed "$source_label" "$before" "source restart delete baseline"
  before="$(source_visible_snapshot "$source_label")"
  remove_remote "$source_label" "stopped-source-delete/from-source.txt"
  wait_for_source_snapshot_changed "$source_label" "$before" "source restart delete"
  stop_daemon "$source_label"
  start_daemon "$source_label"
  wait_until "source daemon fresh after restart" all_fresh
  wait_for_source_snapshot "$source_label" "source restart delete after restart"
}

step_concurrent_same_path_edits() {
  CONCURRENT_SOURCE_CONTENT="concurrent edit from $source_label in $RUN_ID"
  CONCURRENT_TARGET_CONTENT="concurrent edit from $target_label in $RUN_ID"

  local before
  before="$(source_visible_snapshot "$source_label")"
  write_file "$source_label" "conflicts/concurrent.txt" "concurrent baseline in $RUN_ID"
  wait_for_source_snapshot_changed "$source_label" "$before" "concurrent edit baseline"

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
  local before
  before="$(source_visible_snapshot "$source_label")"
  for i in $(seq 1 "$MANY_FILES"); do
    write_file "$source_label" "many/$(printf "%03d" "$i").txt" "many file $i from $source_label"
  done
  wait_for_source_snapshot_changed "$source_label" "$before" "many small files"
}

step_large_file() {
  local before
  before="$(source_visible_snapshot "$target_label")"
  write_zero_file "$target_label" "large/zero.bin" "$LARGE_BYTES"
  wait_for_source_snapshot_changed "$target_label" "$before" "large file"
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
    if [[ "$(host_value "$label" kind)" == "posix" && ( "$label" == ubuntu* || "$label" == linux* ) ]]; then
      MOUNT_LABELS+="${MOUNT_LABELS:+ }$label"
    fi
  done
fi

echo "run id: $RUN_ID"
echo "hosts: ${LABELS[*]}"
echo "idrive profile: $E2E_PROFILE"
if [[ "$PROVIDER_MUTATIONS" == "1" ]]; then
  echo "mutation surface: provider commands"
else
  echo "mutation surface: provider bridge on Windows; projections on POSIX mounts: ${MOUNT_LABELS:-none}"
fi

for label in "${LABELS[@]}"; do
  echo "setting up $label ($(host_value "$label" ssh))"
  setup_host "$label"
done

echo "initializing owner on $owner_label"
owner_json="$(idrive_cmd "$owner_label" init --label "$owner_label")"
owner_profile_id="$(jq -r '.profile_id' <<<"$owner_json")"
admin_app_key_npub="$(jq -r '.current_app_key_npub' <<<"$owner_json")"
set_host_value "$owner_label" app_key_npub "$admin_app_key_npub"
invite_json="$(idrive_cmd "$owner_label" app-keys invite)"
invite_url="$(jq -r '.url' <<<"$invite_json")"
invite_admin_app_key_npub="$(jq -r '.admin_app_key_npub' <<<"$invite_json")"
if [[ "$invite_url" != https://drive.iris.to/invite/* ]]; then
  echo "owner invite did not use canonical https://drive.iris.to/invite/ URL: $invite_url" >&2
  exit 1
fi
if [[ "$invite_admin_app_key_npub" != "$admin_app_key_npub" ]]; then
  echo "owner invite metadata does not match owner admin AppKey" >&2
  exit 1
fi

for label in "${LABELS[@]}"; do
  if [[ "$label" == "$owner_label" ]]; then
    continue
  fi
  echo "requesting invite-based link for $label"
  link_json="$(idrive_cmd "$label" app-keys request "$invite_url" --label "$label")"
  linked_app_key_npub="$(jq -r '.current_app_key_npub' <<<"$link_json")"
  set_host_value "$label" app_key_npub "$linked_app_key_npub"
  request_url="$(jq -r '.app_key_link_request.url' <<<"$link_json")"
  request_profile_id="$(jq -r '.app_key_link_request.profile_id' <<<"$link_json")"
  request_admin_app_key_npub="$(jq -r '.app_key_link_request.admin_app_key_npub' <<<"$link_json")"
  if [[ "$request_url" != iris-drive://app-key-link\?* ]]; then
    echo "$label did not create an app-key-link request URL: $request_url" >&2
    exit 1
  fi
  if [[ "$request_admin_app_key_npub" != "$admin_app_key_npub" || "$request_profile_id" != "$owner_profile_id" ]]; then
    echo "$label request metadata does not match invite profile/admin AppKey" >&2
    exit 1
  fi
  if [[ "$request_url" != *"app_key="* || -z "$linked_app_key_npub" || "$linked_app_key_npub" == "null" ]]; then
    echo "$label request did not include an app-key URL and structured AppKey metadata: $request_url" >&2
    exit 1
  fi
  if [[ "$request_url" == *"local-owner"* || "$request_url" == *"app_key=device-"* ]]; then
    echo "$label request URL leaked placeholder ids: $request_url" >&2
    exit 1
  fi
  idrive_cmd "$owner_label" app-keys approve "$request_url" --label "$label" >/dev/null
done

if [[ "$SIDELOAD_APPKEYS" == "1" ]]; then
  echo "side-loading approved profile roster ops into peer temp configs"
  roster_ops_b64="$(owner_profile_roster_ops_b64 "$owner_label")"
  for label in "${LABELS[@]}"; do
    if [[ "$label" == "$owner_label" ]]; then
      continue
    fi
    sideload_profile_roster_ops "$label" "$roster_ops_b64" "$owner_profile_id"
  done
fi

configure_fips_static_hints

for label in "${LABELS[@]}"; do
  start_daemon "$label"
done

run_step "authorization" wait_until "all devices authorized" all_authorized
run_step "fresh daemons" wait_until "all daemon statuses fresh" all_fresh
run_step "FIPS roster readiness" wait_until "every device has the full roster" all_have_roster_peers

run_step "initial seed writes" run_for_all_labels_parallel write_initial_seed_files

run_step "initial multi-device merge" wait_for_converged_union "initial merge"

run_step "create edit rename delete from $source_label" step_create_edit_rename_delete
run_step "nested create/delete from $target_label" step_nested_create_delete
run_step "windows projection stale remote edit" step_windows_projection_replaces_stale_remote_edit
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
if idle_cpu_gate_enabled; then
  run_step "idle daemon CPU gate" run_for_all_labels_parallel idle_cpu_gate_label
fi

echo
echo "cross-vm e2e passed for: ${LABELS[*]}"
