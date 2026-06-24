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
HASHTREE_ROOT="${IRIS_DRIVE_HASHTREE_ROOT:-$(cd "$ROOT/../hashtree/rust" && pwd)}"
HASHTREE_ROOT="$(git -C "$HASHTREE_ROOT" rev-parse --show-toplevel)"
FIPS_ROOT="${IRIS_DRIVE_FIPS_ROOT:-$(cd "$ROOT/../fips" && pwd)}"
FIPS_ROOT="$(git -C "$FIPS_ROOT" rev-parse --show-toplevel)"
SOCIAL_GRAPH_ROOT="${IRIS_DRIVE_SOCIAL_GRAPH_ROOT:-$(cd "$ROOT/../nostr-social-graph" && pwd)}"
SOCIAL_GRAPH_ROOT="$(git -C "$SOCIAL_GRAPH_ROOT" rev-parse --show-toplevel)"
CASHU_SERVICE_ROOT="${IRIS_DRIVE_CASHU_SERVICE_ROOT:-$(cd "$ROOT/../cashu-service" && pwd)}"
CASHU_SERVICE_ROOT="$(git -C "$CASHU_SERVICE_ROOT" rev-parse --show-toplevel)"
SYNC_BRANCH="${IRIS_DRIVE_DEV_VM_SYNC_BRANCH:-codex/dev-vm-sync}"
FIPS_SYNC_BRANCH="${IRIS_DRIVE_DEV_VM_FIPS_SYNC_BRANCH:-$SYNC_BRANCH}"
SOCIAL_GRAPH_SYNC_BRANCH="${IRIS_DRIVE_DEV_VM_SOCIAL_GRAPH_SYNC_BRANCH:-$SYNC_BRANCH}"
TARGET_BRANCH="${IRIS_DRIVE_DEV_VM_TARGET_BRANCH:-$(git -C "$ROOT" branch --show-current || true)}"
TARGET_BRANCH="${TARGET_BRANCH:-master}"
FIPS_TARGET_BRANCH="${IRIS_DRIVE_DEV_VM_FIPS_TARGET_BRANCH:-$FIPS_SYNC_BRANCH}"
SOCIAL_GRAPH_TARGET_BRANCH="${IRIS_DRIVE_DEV_VM_SOCIAL_GRAPH_TARGET_BRANCH:-master}"
FORCE=0
FAIL_ON_DIRTY=0
SKIP_PUSH=0
NO_RUN=0
LIST_TARGETS=0
ONLY_LABELS=()
SSH_PROBE_OPTS=(-o BatchMode=yes -o ConnectTimeout="${IRIS_DRIVE_DEV_VM_SSH_PROBE_TIMEOUT:-10}")

usage() {
  cat <<'USAGE'
Usage:
  scripts/dev-vm-update-run.sh [--force|--fail-on-dirty] [--only macos|ubuntu|windows] [--skip-push] [--no-run]
  scripts/dev-vm-update-run.sh --list-targets

Syncs the current committed iris-drive, hashtree, and fips revisions to the
configured VM git remotes, updates the VM worktrees, builds, then restarts the
dev app or daemon with native FIPS UDP over the nvpn overlay while keeping
WebRTC enabled.

Remote worktrees with local changes are auto-stashed before checkout. Use
--fail-on-dirty to stop instead, or --force to discard tracked VM changes.

VM git remotes are read from environment, from
~/.config/iris-drive/dev-lab.env, or from generic local remotes named
macos, ubuntu, and windows. Keep machine-specific SSH names in local config.

Environment:
  IRIS_DRIVE_DEV_VM_MACOS_REMOTE      Git remote name for the macOS VM.
  IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE     Git remote name for the Ubuntu VM.
  IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE    Git remote name for the Windows VM.
  IRIS_DRIVE_DEV_VM_MACOS_SSH_HOST    Optional SSH host override for commands.
  IRIS_DRIVE_DEV_VM_UBUNTU_SSH_HOST   Optional SSH host override for commands.
  IRIS_DRIVE_DEV_VM_WINDOWS_SSH_HOST  Optional SSH host override for commands.
                                      Git remote hostnames still define peer
                                      labels/static-hint keys.
  IRIS_DRIVE_DEV_VM_FIPS_PORT         UDP port advertised over nvpn (default: 22121).
  IRIS_DRIVE_DEV_VM_USE_NVPN_STATIC_HINTS
                                      auto/1/0 for direct nvpn static FIPS
                                      hints. auto only injects peer hints that
                                      are reachable from that VM (default: auto).
  IRIS_DRIVE_DEV_VM_SYNC_BRANCH       Temporary branch pushed to VM bare repos.
  IRIS_DRIVE_DEV_VM_FIPS_SYNC_BRANCH  Temporary branch pushed for fips
                                      (default: same as SYNC_BRANCH).
  IRIS_DRIVE_DEV_VM_TARGET_BRANCH     Branch name checked out in VM worktrees.
  IRIS_DRIVE_DEV_VM_FIPS_TARGET_BRANCH
                                      Branch checked out in VM fips worktrees
                                      (default: FIPS sync branch, to avoid
                                      clobbering local fips feature branches).
  IRIS_DRIVE_DEV_VM_SOCIAL_GRAPH_SYNC_BRANCH
                                      Temporary branch pushed for nostr-social-graph.
  IRIS_DRIVE_DEV_VM_SOCIAL_GRAPH_TARGET_BRANCH
                                      Branch checked out in VM nostr-social-graph
                                      worktrees (default: master).
  IRIS_DRIVE_DEV_VM_SOCIAL_GRAPH_BARE
                                      Remote bare repo path for nostr-social-graph
                                      when no per-target social graph remote is set
                                      (default: ~/git/nostr-social-graph.git).
  IRIS_DRIVE_DEV_VM_MACOS_SOCIAL_GRAPH_REMOTE
  IRIS_DRIVE_DEV_VM_UBUNTU_SOCIAL_GRAPH_REMOTE
  IRIS_DRIVE_DEV_VM_WINDOWS_SOCIAL_GRAPH_REMOTE
                                      Optional git remote names for existing
                                      nostr-social-graph bare repos.
  IRIS_DRIVE_DEV_VM_REQUIRE_CLEAN=1   Refuse to run when local repos are dirty.
  IRIS_DRIVE_DEV_VM_MIN_FREE_KB       Prune VM incremental build caches below
                                      this free space.
  IRIS_DRIVE_DEV_VM_PRUNE_COMPILED_CACHE=1
                                      Also prune compiled Cargo deps/build
                                      artifacts when below MIN_FREE_KB.
  IRIS_DRIVE_DEV_VM_CARGO_INCREMENTAL Override VM Cargo incremental builds
                                      (default: 0).
  IRIS_DRIVE_DEV_VM_CARGO_PROFILE_DEV_DEBUG
                                      Override VM Cargo dev debuginfo
                                      (default: 0).
  IRIS_DRIVE_DEV_VM_SKIP_CONNECTIVITY_CHECK=1
                                      Skip the final all-VM FIPS online check.
  IRIS_DRIVE_DEV_VM_CONNECTIVITY_TIMEOUT
                                      Seconds to wait for all selected peers to
                                      report fips_online=true (default: 60).
  IRIS_DRIVE_DEV_VM_FIPS_READY_TIMEOUT
                                      Seconds to wait for a restarted daemon to
                                      report FIPS running (default: 300).
  IRIS_DRIVE_DEV_VM_FIPS_ENABLE_BOOTSTRAP
                                      Override FIPS bootstrap discovery for VM
                                      daemons. Defaults to false when static
                                      peer hints are present, otherwise true.
  IRIS_DRIVE_DEV_VM_FIPS_OPEN_DISCOVERY_MAX_PENDING
                                      Override FIPS open discovery fanout for VM
                                      daemons. Defaults to 0 when static peer
                                      hints cover every VM edge, otherwise 16
                                      so missing edges can be discovered.
  IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER=1
                                      Fail macOS runs unless the app is signed
                                      with FileProvider-capable entitlements.
  IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM
                                      Apple Developer team id used for macOS
                                      FileProvider signing.
  IRIS_DRIVE_DEV_VM_MACOS_SIGN_IDENTITY
                                      macOS codesign identity name or SHA-1 hash;
                                      defaults to first Apple Development
                                      identity, else ad-hoc.
  IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN    Optional macOS signing keychain to unlock
                                      and pass to codesign/xcodebuild.
  IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN_PASS_FILE
                                      Password file for the signing keychain.
                                      Defaults to
                                      ~/.config/iris-drive/dev-build-keychain.pass.
  IRIS_DRIVE_DEV_VM_MACOS_RESET_FILEPROVIDER=0
                                      Skip FileProvider domain reset on macOS
                                      app start. The default reset is done via
                                      NSFileProviderManager, not by deleting
                                      CloudStorage files.
  IRIS_DRIVE_HASHTREE_ROOT            Local hashtree/rust checkout.
  IRIS_DRIVE_FIPS_ROOT                Local fips checkout.
  IRIS_DRIVE_SOCIAL_GRAPH_ROOT        Local nostr-social-graph checkout.
  IRIS_DRIVE_CASHU_SERVICE_ROOT       Local cashu-service checkout.

Remote worktree paths are expected to be:
  ~/src/iris-drive
  ~/src/hashtree
  ~/src/fips
  ~/src/nostr-social-graph
  ~/src/cashu-service

The script never git-cleans untracked files.
USAGE
}

log() {
  printf '[dev-vms] %s\n' "$*" >&2
}

die() {
  printf '[dev-vms] ERROR: %s\n' "$*" >&2
  exit 1
}

sh_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/'\\\\''/g")"
}

ps_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/''/g")"
}

contains_label() {
  local needle="$1"
  local label
  if [[ ${#ONLY_LABELS[@]} -eq 0 ]]; then
    return 0
  fi
  for label in "${ONLY_LABELS[@]}"; do
    if [[ "$label" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)
      FORCE=1
      shift
      ;;
    --fail-on-dirty)
      FAIL_ON_DIRTY=1
      shift
      ;;
    --skip-push)
      SKIP_PUSH=1
      shift
      ;;
    --no-run|--build-only)
      NO_RUN=1
      shift
      ;;
    --list-targets)
      LIST_TARGETS=1
      shift
      ;;
    --only)
      [[ $# -ge 2 ]] || die "--only needs a label"
      case "$2" in
        macos|ubuntu|windows) ONLY_LABELS+=("$2") ;;
        *) die "unknown --only label: $2" ;;
      esac
      shift 2
      ;;
    -h|--help|help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

warn_or_fail_local_dirty() {
  local repo="$1"
  local name="$2"
  local dirty
  dirty="$(git -C "$repo" status --short)"
  if [[ -z "$dirty" ]]; then
    return 0
  fi
  if [[ "${IRIS_DRIVE_DEV_VM_REQUIRE_CLEAN:-0}" == "1" ]]; then
    printf '%s\n' "$dirty" >&2
    die "$name has local changes; commit/stash or unset IRIS_DRIVE_DEV_VM_REQUIRE_CLEAN"
  fi
  log "warning: $name has local changes; only committed HEAD will be deployed"
}

remote_url_parts() {
  local repo="$1"
  local remote="$2"
  local url
  url="$(git -C "$repo" remote get-url "$remote" 2>/dev/null || true)"
  [[ -n "$url" ]] || return 1
  [[ "$url" == *:* && "$url" != *"://"* ]] || {
    die "remote $remote in $repo must use scp-style ssh syntax, got: $url"
  }
  printf '%s\t%s\n' "${url%%:*}" "${url#*:}"
}

ensure_remote_bare_repo() {
  local kind="$1"
  local host="$2"
  local repo="$3"

  case "$kind" in
    windows)
      {
        printf '$BareRepo = %s\n' "$(ps_quote "$repo")"
        cat <<'REMOTE_PS'
$ErrorActionPreference = "Stop"

function Expand-RemotePath([string]$Path) {
  if ($Path -eq "~") {
    return $HOME
  }
  if ($Path.StartsWith("~/") -or $Path.StartsWith("~\")) {
    return (Join-Path $HOME $Path.Substring(2))
  }
  if (-not [System.IO.Path]::IsPathRooted($Path)) {
    return (Join-Path $HOME $Path)
  }
  return $Path
}

$BareRepo = Expand-RemotePath $BareRepo
if (Test-Path $BareRepo) {
  $IsBare = git --git-dir $BareRepo rev-parse --is-bare-repository
  if ($LASTEXITCODE -ne 0 -or ($IsBare -join "").Trim() -ne "true") {
    throw "sync target exists but is not a bare git repo: $BareRepo"
  }
  exit 0
}

$Parent = Split-Path -Parent $BareRepo
if ($Parent) {
  New-Item -ItemType Directory -Force -Path $Parent | Out-Null
}
git init --bare $BareRepo | Out-Null
if ($LASTEXITCODE -ne 0) {
  throw "failed to create bare git repo: $BareRepo"
}
REMOTE_PS
      } | ssh "$host" 'cmd /d /s /c "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""`$script = [Console]::In.ReadToEnd(); & ([scriptblock]::Create(`$script))"""'
      ;;
    *)
      {
        printf 'BARE_REPO=%s\n' "$(sh_quote "$repo")"
        cat <<'REMOTE_SH'
set -Eeuo pipefail

expand_path() {
  case "$1" in
    "~") printf '%s\n' "$HOME" ;;
    "~/"*) printf '%s/%s\n' "$HOME" "${1:2}" ;;
    /*) printf '%s\n' "$1" ;;
    *) printf '%s/%s\n' "$HOME" "$1" ;;
  esac
}

bare_repo="$(expand_path "$BARE_REPO")"
if [[ -e "$bare_repo" ]]; then
  if [[ "$(git --git-dir "$bare_repo" rev-parse --is-bare-repository 2>/dev/null || true)" != "true" ]]; then
    printf 'sync target exists but is not a bare git repo: %s\n' "$bare_repo" >&2
    exit 6
  fi
  exit 0
fi

mkdir -p "$(dirname "$bare_repo")"
git init --bare "$bare_repo" >/dev/null
REMOTE_SH
      } | ssh "$host" 'bash -se'
      ;;
  esac
}

declare -a LABELS=()
declare -a KINDS=()
declare -a HOSTS=()
declare -a SSH_HOSTS=()
declare -a ALL_LABELS=()
declare -a ALL_KINDS=()
declare -a ALL_HOSTS=()
declare -a ALL_SSH_HOSTS=()
declare -a IRIS_REMOTES=()
declare -a HASHTREE_REMOTES=()
declare -a FIPS_REMOTES=()
declare -a SOCIAL_GRAPH_REMOTES=()
declare -a IRIS_BARES=()
declare -a HASHTREE_BARES=()
declare -a FIPS_BARES=()
declare -a SOCIAL_GRAPH_BARES=()
declare -a CASHU_SERVICE_BARES=()
declare -a ALL_OVERLAY_IPS=()
declare -a STATIC_PEERS_BY_INDEX=()
declare -a STATIC_PEERS_COMPLETE_BY_INDEX=()

add_target_from_remotes() {
  local label="$1"
  local kind="$2"
  local iris_remote="$3"
  local hashtree_remote="$4"
  local fips_remote="$5"
  local social_graph_remote="$6"
  local iris_parts
  local hashtree_parts
  local fips_parts
  local social_graph_parts
  local host
  local hashtree_host
  local fips_host
  local social_graph_host
  local ssh_host
  local iris_bare
  local hashtree_bare
  local fips_bare
  local social_graph_bare
  local cashu_service_bare

  iris_parts="$(remote_url_parts "$ROOT" "$iris_remote" || true)"
  hashtree_parts="$(remote_url_parts "$HASHTREE_ROOT" "$hashtree_remote" || true)"
  fips_parts="$(remote_url_parts "$FIPS_ROOT" "$fips_remote" || true)"
  if [[ -z "$iris_parts" || -z "$hashtree_parts" || -z "$fips_parts" ]]; then
    if [[ ${#ONLY_LABELS[@]} -gt 0 ]]; then
      die "missing git remotes for requested target $label"
    fi
    log "skipping $label; missing git remote $iris_remote, hashtree remote $hashtree_remote, or fips remote $fips_remote"
    return 0
  fi

  host="${iris_parts%%$'\t'*}"
  iris_bare="${iris_parts#*$'\t'}"
  hashtree_host="${hashtree_parts%%$'\t'*}"
  hashtree_bare="${hashtree_parts#*$'\t'}"
  fips_host="${fips_parts%%$'\t'*}"
  fips_bare="${fips_parts#*$'\t'}"
  if [[ "$host" != "$hashtree_host" ]]; then
    die "$label iris-drive remote host ($host) differs from hashtree host ($hashtree_host)"
  fi
  if [[ "$host" != "$fips_host" ]]; then
    die "$label iris-drive remote host ($host) differs from fips host ($fips_host)"
  fi
  if [[ -n "$social_graph_remote" ]]; then
    social_graph_parts="$(remote_url_parts "$SOCIAL_GRAPH_ROOT" "$social_graph_remote" || true)"
    if [[ -z "$social_graph_parts" ]]; then
      if [[ ${#ONLY_LABELS[@]} -gt 0 ]]; then
        die "missing nostr-social-graph git remote $social_graph_remote for requested target $label"
      fi
      log "skipping $label; missing nostr-social-graph git remote $social_graph_remote"
      return 0
    fi
    social_graph_host="${social_graph_parts%%$'\t'*}"
    social_graph_bare="${social_graph_parts#*$'\t'}"
    if [[ "$host" != "$social_graph_host" ]]; then
      die "$label iris-drive remote host ($host) differs from nostr-social-graph host ($social_graph_host)"
    fi
  else
    social_graph_bare="${IRIS_DRIVE_DEV_VM_SOCIAL_GRAPH_BARE:-~/git/nostr-social-graph.git}"
  fi
  cashu_service_bare="${IRIS_DRIVE_DEV_VM_CASHU_SERVICE_BARE:-~/git/cashu-service.git}"
  ssh_host="$(ssh_host_for_label "$label" "$host")"

  ALL_LABELS+=("$label")
  ALL_KINDS+=("$kind")
  ALL_HOSTS+=("$host")
  ALL_SSH_HOSTS+=("$ssh_host")

  contains_label "$label" || return 0

  LABELS+=("$label")
  KINDS+=("$kind")
  HOSTS+=("$host")
  SSH_HOSTS+=("$ssh_host")
  IRIS_REMOTES+=("$iris_remote")
  HASHTREE_REMOTES+=("$hashtree_remote")
  FIPS_REMOTES+=("$fips_remote")
  SOCIAL_GRAPH_REMOTES+=("$social_graph_remote")
  IRIS_BARES+=("$iris_bare")
  HASHTREE_BARES+=("$hashtree_bare")
  FIPS_BARES+=("$fips_bare")
  SOCIAL_GRAPH_BARES+=("$social_graph_bare")
  CASHU_SERVICE_BARES+=("$cashu_service_bare")
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

ssh_git_url() {
  local host="$1"
  local path="$2"
  printf '%s:%s\n' "$host" "$path"
}

warn_or_fail_local_dirty "$ROOT" "iris-drive"
warn_or_fail_local_dirty "$HASHTREE_ROOT" "hashtree"
warn_or_fail_local_dirty "$FIPS_ROOT" "fips"
warn_or_fail_local_dirty "$SOCIAL_GRAPH_ROOT" "nostr-social-graph"
warn_or_fail_local_dirty "$CASHU_SERVICE_ROOT" "cashu-service"

configured_remote_name() {
  local env_var="$1"
  local generic_name="$2"
  local value="${!env_var:-}"
  if [[ -n "$value" ]]; then
    printf '%s\n' "$value"
    return 0
  fi
  if [[ -n "$generic_name" ]] && git -C "$ROOT" remote get-url "$generic_name" >/dev/null 2>&1; then
    printf '%s\n' "$generic_name"
    return 0
  fi
  printf '\n'
}

MACOS_IRIS_REMOTE="$(configured_remote_name IRIS_DRIVE_DEV_VM_MACOS_REMOTE macos)"
UBUNTU_IRIS_REMOTE="$(configured_remote_name IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE ubuntu)"
WINDOWS_IRIS_REMOTE="$(configured_remote_name IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE windows)"
MACOS_HASHTREE_REMOTE="${IRIS_DRIVE_DEV_VM_MACOS_HASHTREE_REMOTE:-$MACOS_IRIS_REMOTE}"
UBUNTU_HASHTREE_REMOTE="${IRIS_DRIVE_DEV_VM_UBUNTU_HASHTREE_REMOTE:-$UBUNTU_IRIS_REMOTE}"
WINDOWS_HASHTREE_REMOTE="${IRIS_DRIVE_DEV_VM_WINDOWS_HASHTREE_REMOTE:-$WINDOWS_IRIS_REMOTE}"
MACOS_FIPS_REMOTE="${IRIS_DRIVE_DEV_VM_MACOS_FIPS_REMOTE:-$MACOS_IRIS_REMOTE}"
UBUNTU_FIPS_REMOTE="${IRIS_DRIVE_DEV_VM_UBUNTU_FIPS_REMOTE:-$UBUNTU_IRIS_REMOTE}"
WINDOWS_FIPS_REMOTE="${IRIS_DRIVE_DEV_VM_WINDOWS_FIPS_REMOTE:-$WINDOWS_IRIS_REMOTE}"
MACOS_SOCIAL_GRAPH_REMOTE="${IRIS_DRIVE_DEV_VM_MACOS_SOCIAL_GRAPH_REMOTE:-}"
UBUNTU_SOCIAL_GRAPH_REMOTE="${IRIS_DRIVE_DEV_VM_UBUNTU_SOCIAL_GRAPH_REMOTE:-}"
WINDOWS_SOCIAL_GRAPH_REMOTE="${IRIS_DRIVE_DEV_VM_WINDOWS_SOCIAL_GRAPH_REMOTE:-}"

add_target_from_remotes \
  macos \
  macos \
  "$MACOS_IRIS_REMOTE" \
  "$MACOS_HASHTREE_REMOTE" \
  "$MACOS_FIPS_REMOTE" \
  "$MACOS_SOCIAL_GRAPH_REMOTE"
add_target_from_remotes \
  ubuntu \
  linux \
  "$UBUNTU_IRIS_REMOTE" \
  "$UBUNTU_HASHTREE_REMOTE" \
  "$UBUNTU_FIPS_REMOTE" \
  "$UBUNTU_SOCIAL_GRAPH_REMOTE"
add_target_from_remotes \
  windows \
  windows \
  "$WINDOWS_IRIS_REMOTE" \
  "$WINDOWS_HASHTREE_REMOTE" \
  "$WINDOWS_FIPS_REMOTE" \
  "$WINDOWS_SOCIAL_GRAPH_REMOTE"

if [[ ${#LABELS[@]} -eq 0 ]]; then
  usage >&2
  die "no VM targets configured"
fi

if [[ "$LIST_TARGETS" == "1" ]]; then
  for i in "${!LABELS[@]}"; do
    printf '%s\t%s\t%s\tssh=%s\tiris=%s\thashtree=%s\tfips=%s\n' \
      "${LABELS[$i]}" \
      "${KINDS[$i]}" \
      "${HOSTS[$i]}" \
      "${SSH_HOSTS[$i]}" \
      "${IRIS_BARES[$i]}" \
      "${HASHTREE_BARES[$i]}" \
      "${FIPS_BARES[$i]}" \
      "social-graph=${SOCIAL_GRAPH_BARES[$i]}" \
      "cashu-service=${CASHU_SERVICE_BARES[$i]}"
  done
  exit 0
fi

detect_remote_overlay_ip() {
  local kind="$1"
  local host="$2"
  local label="${3:-}"
  local override_var=""
  local ip=""
  if [[ -n "$label" ]]; then
    override_var="IRIS_DRIVE_DEV_VM_$(printf '%s' "$label" | tr '[:lower:]-' '[:upper:]_')_NVPN_IP"
    ip="${!override_var:-}"
    if [[ -n "$ip" ]]; then
      printf '%s\n' "${ip%%/*}"
      return 0
    fi
  fi
  if [[ "$kind" == "windows" ]]; then
    ip="$(ssh "${SSH_PROBE_OPTS[@]}" "$host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' <<'REMOTE_PS' 2>/dev/null || true
$TunnelIp = (Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -eq 'nvpn' -and $_.IPAddress -like '10.44.*' } | Select-Object -First 1 -ExpandProperty IPAddress)
$ErrorActionPreference = "SilentlyContinue"
$Nvpn = (Get-Command nvpn -ErrorAction SilentlyContinue).Source
if (-not $Nvpn) {
  $Candidate = Join-Path $HOME "src\nostr-vpn\target\debug\nvpn.exe"
  if (Test-Path $Candidate) { $Nvpn = $Candidate }
}
if (-not $TunnelIp -and $Nvpn) {
  try {
    $Status = & $Nvpn status --json | ConvertFrom-Json
    $Running = $true
    $VpnActive = $true
    if ($Status.daemon -and $null -ne $Status.daemon.running) {
      $Running = [bool]$Status.daemon.running
    }
    if ($Status.daemon -and $Status.daemon.state -and $null -ne $Status.daemon.state.vpn_active) {
      $VpnActive = [bool]$Status.daemon.state.vpn_active
    }
    if ($Running -and $VpnActive -and $Status.tunnel_ip) {
      $TunnelIp = (($Status.tunnel_ip -as [string]) -replace "/.*$", "")
    }
  } catch {}
}
if ($TunnelIp) {
  Write-Output $TunnelIp
}
REMOTE_PS
)"
  else
    ip="$(ssh "${SSH_PROBE_OPTS[@]}" "$host" 'bash -se' <<'REMOTE_SH' 2>/dev/null || true
set -Eeuo pipefail
nvpn=""
for candidate in \
  "$(command -v nvpn 2>/dev/null || true)" \
  "$HOME/src/nostr-vpn/target/debug/nvpn" \
  "$HOME/src/nostr-vpn/target/aarch64-apple-darwin/debug/nvpn" \
  "/Library/PrivilegedHelperTools/to.nostrvpn.nvpn"
do
  [[ -n "$candidate" && -x "$candidate" ]] || continue
  if "$candidate" status --json 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); daemon=d.get("daemon") or {}; state=daemon.get("state") or {}; running=daemon.get("running"); vpn_active=state.get("vpn_active"); ip=(d.get("tunnel_ip") or "").split("/")[0]; invalid=not ip or running is False or vpn_active is False; print(ip) if not invalid else None; sys.exit(1 if invalid else 0)'; then
    exit 0
  fi
done
REMOTE_SH
)"
  fi
  ip="${ip//$'\r'/}"
  ip="$(printf '%s\n' "$ip" | awk 'NF { print $1; exit }')"
  [[ -n "$ip" ]] || return 1
  printf '%s\n' "$ip"
}

target_peer_hint_key() {
  local host="$1"
  host="${host#*@}"
  host="${host%.nvpn}"
  printf '%s\n' "$host"
}

can_target_reach_overlay_ip() {
  local kind="$1"
  local host="$2"
  local ip="$3"

  if [[ "$kind" == "windows" ]]; then
    ssh "${SSH_PROBE_OPTS[@]}" "$host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' <<REMOTE_PS >/dev/null 2>&1
\$ErrorActionPreference = "SilentlyContinue"
\$Ip = "$ip"
if (Test-Connection -ComputerName \$Ip -Count 1 -Quiet) {
  exit 0
}
exit 1
REMOTE_PS
    return $?
  fi

  local wait_arg="1"
  if [[ "$kind" == "macos" ]]; then
    wait_arg="1000"
  fi
  ssh "${SSH_PROBE_OPTS[@]}" "$host" 'bash -se' <<REMOTE_SH >/dev/null 2>&1
ping -c 1 -W "$wait_arg" "$ip" >/dev/null 2>&1
REMOTE_SH
}

build_static_peer_hints() {
  local fips_port="${IRIS_DRIVE_DEV_VM_FIPS_PORT:-22121}"
  local mode="${IRIS_DRIVE_DEV_VM_USE_NVPN_STATIC_HINTS:-auto}"
  local pieces=()
  local ip=""
  local key=""
  local i
  local j
  local -a peer_labels=() peer_kinds=() peer_hosts=() peer_ssh_hosts=() peer_overlay_ips=()

  mode="$(printf '%s' "$mode" | tr '[:upper:]' '[:lower:]')"
  case "$mode" in
    ""|auto)
      mode="auto"
      ;;
    1|true|yes|on|force|forced)
      mode="force"
      ;;
    0|false|no|off|disabled)
      for i in "${!LABELS[@]}"; do
        STATIC_PEERS_BY_INDEX[$i]=""
        STATIC_PEERS_COMPLETE_BY_INDEX[$i]="0"
      done
      log "not using nvpn static FIPS peer hints; disabled by IRIS_DRIVE_DEV_VM_USE_NVPN_STATIC_HINTS=$mode"
      return 0
      ;;
    *)
      die "unknown IRIS_DRIVE_DEV_VM_USE_NVPN_STATIC_HINTS value: $mode"
      ;;
  esac

  if [[ ${#ONLY_LABELS[@]} -gt 0 ]]; then
    peer_labels=("${LABELS[@]}")
    peer_kinds=("${KINDS[@]}")
    peer_hosts=("${HOSTS[@]}")
    peer_ssh_hosts=("${SSH_HOSTS[@]}")
  else
    peer_labels=("${ALL_LABELS[@]}")
    peer_kinds=("${ALL_KINDS[@]}")
    peer_hosts=("${ALL_HOSTS[@]}")
    peer_ssh_hosts=("${ALL_SSH_HOSTS[@]}")
  fi

  for i in "${!peer_labels[@]}"; do
    ip="$(detect_remote_overlay_ip "${peer_kinds[$i]}" "${peer_ssh_hosts[$i]}" "${peer_labels[$i]}" || true)"
    if [[ -z "$ip" ]]; then
      log "warning: could not detect nvpn IP for ${peer_labels[$i]} on ${peer_ssh_hosts[$i]}; native FIPS may need WebRTC or relay transport"
      continue
    fi
    peer_overlay_ips[$i]="$ip"
  done

  for i in "${!LABELS[@]}"; do
    pieces=()
    for j in "${!peer_labels[@]}"; do
      [[ "${LABELS[$i]}" == "${peer_labels[$j]}" ]] && continue
      ip="${peer_overlay_ips[$j]:-}"
      [[ -n "$ip" ]] || continue
      if [[ "$mode" == "auto" ]] && ! can_target_reach_overlay_ip "${KINDS[$i]}" "${SSH_HOSTS[$i]}" "$ip"; then
        log "not using nvpn static FIPS hint ${LABELS[$i]} -> ${peer_labels[$j]} ($ip); overlay address is not reachable from ${LABELS[$i]}"
        continue
      fi
      key="$(target_peer_hint_key "${peer_hosts[$j]}")"
      pieces+=("$key=$ip:$fips_port")
    done

    if [[ ${#pieces[@]} -gt 0 ]]; then
      local IFS=,
      STATIC_PEERS_BY_INDEX[$i]="${pieces[*]}"
      if [[ ${#pieces[@]} -ge $((${#peer_labels[@]} - 1)) ]]; then
        STATIC_PEERS_COMPLETE_BY_INDEX[$i]="1"
      else
        STATIC_PEERS_COMPLETE_BY_INDEX[$i]="0"
      fi
      log "using static FIPS peer hints for ${LABELS[$i]} over nvpn: ${STATIC_PEERS_BY_INDEX[$i]}"
    else
      STATIC_PEERS_BY_INDEX[$i]=""
      STATIC_PEERS_COMPLETE_BY_INDEX[$i]="0"
      log "not using nvpn static FIPS peer hints for ${LABELS[$i]}; no reachable overlay peers"
    fi
  done
}

build_static_peer_hints

if [[ "$SKIP_PUSH" != "1" ]]; then
  for i in "${!LABELS[@]}"; do
    log "ensuring VM bare git repos exist for ${LABELS[$i]}"
    ensure_remote_bare_repo "${KINDS[$i]}" "${SSH_HOSTS[$i]}" "${IRIS_BARES[$i]}"
    ensure_remote_bare_repo "${KINDS[$i]}" "${SSH_HOSTS[$i]}" "${HASHTREE_BARES[$i]}"
    ensure_remote_bare_repo "${KINDS[$i]}" "${SSH_HOSTS[$i]}" "${FIPS_BARES[$i]}"
    ensure_remote_bare_repo "${KINDS[$i]}" "${SSH_HOSTS[$i]}" "${SOCIAL_GRAPH_BARES[$i]}"
    ensure_remote_bare_repo "${KINDS[$i]}" "${SSH_HOSTS[$i]}" "${CASHU_SERVICE_BARES[$i]}"

    log "pushing iris-drive HEAD to ${LABELS[$i]} (${SSH_HOSTS[$i]}:${IRIS_BARES[$i]} $SYNC_BRANCH)"
    git -C "$ROOT" push "$(ssh_git_url "${SSH_HOSTS[$i]}" "${IRIS_BARES[$i]}")" "+HEAD:refs/heads/$SYNC_BRANCH"
    log "pushing hashtree HEAD to ${LABELS[$i]} (${SSH_HOSTS[$i]}:${HASHTREE_BARES[$i]} $SYNC_BRANCH)"
    git -C "$HASHTREE_ROOT" push "$(ssh_git_url "${SSH_HOSTS[$i]}" "${HASHTREE_BARES[$i]}")" "+HEAD:refs/heads/$SYNC_BRANCH"
    log "pushing fips HEAD to ${LABELS[$i]} (${SSH_HOSTS[$i]}:${FIPS_BARES[$i]} $FIPS_SYNC_BRANCH)"
    git -C "$FIPS_ROOT" push "$(ssh_git_url "${SSH_HOSTS[$i]}" "${FIPS_BARES[$i]}")" "+HEAD:refs/heads/$FIPS_SYNC_BRANCH"
    log "pushing nostr-social-graph HEAD to ${LABELS[$i]} (${SSH_HOSTS[$i]}:${SOCIAL_GRAPH_BARES[$i]} $SOCIAL_GRAPH_SYNC_BRANCH)"
    git -C "$SOCIAL_GRAPH_ROOT" push "$(ssh_git_url "${SSH_HOSTS[$i]}" "${SOCIAL_GRAPH_BARES[$i]}")" "+HEAD:refs/heads/$SOCIAL_GRAPH_SYNC_BRANCH"
    log "pushing cashu-service HEAD to ${LABELS[$i]} (${SSH_HOSTS[$i]}:${CASHU_SERVICE_BARES[$i]} $SYNC_BRANCH)"
    git -C "$CASHU_SERVICE_ROOT" push "$(ssh_git_url "${SSH_HOSTS[$i]}" "${CASHU_SERVICE_BARES[$i]}")" "+HEAD:refs/heads/$SYNC_BRANCH"
  done
fi

run_posix_target() {
  local label="$1"
  local kind="$2"
  local host="$3"
  local iris_bare="$4"
  local hashtree_bare="$5"
  local fips_bare="$6"
  local social_graph_bare="$7"
  local cashu_service_bare="$8"
  local static_peers="$9"
  local static_peers_complete="${10}"

  {
    printf 'LABEL=%s\n' "$(sh_quote "$label")"
    printf 'KIND=%s\n' "$(sh_quote "$kind")"
    printf 'IRIS_BARE=%s\n' "$(sh_quote "$iris_bare")"
    printf 'HASHTREE_BARE=%s\n' "$(sh_quote "$hashtree_bare")"
    printf 'FIPS_BARE=%s\n' "$(sh_quote "$fips_bare")"
    printf 'SOCIAL_GRAPH_BARE=%s\n' "$(sh_quote "$social_graph_bare")"
    printf 'CASHU_SERVICE_BARE=%s\n' "$(sh_quote "$cashu_service_bare")"
    printf 'SYNC_BRANCH=%s\n' "$(sh_quote "$SYNC_BRANCH")"
    printf 'FIPS_SYNC_BRANCH=%s\n' "$(sh_quote "$FIPS_SYNC_BRANCH")"
    printf 'SOCIAL_GRAPH_SYNC_BRANCH=%s\n' "$(sh_quote "$SOCIAL_GRAPH_SYNC_BRANCH")"
    printf 'TARGET_BRANCH=%s\n' "$(sh_quote "$TARGET_BRANCH")"
    printf 'FIPS_TARGET_BRANCH=%s\n' "$(sh_quote "$FIPS_TARGET_BRANCH")"
    printf 'SOCIAL_GRAPH_TARGET_BRANCH=%s\n' "$(sh_quote "$SOCIAL_GRAPH_TARGET_BRANCH")"
    printf 'FORCE=%s\n' "$(sh_quote "$FORCE")"
    printf 'FAIL_ON_DIRTY=%s\n' "$(sh_quote "$FAIL_ON_DIRTY")"
    printf 'NO_RUN=%s\n' "$(sh_quote "$NO_RUN")"
    printf 'FIPS_PORT=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_FIPS_PORT:-22121}")"
    printf 'STATIC_PEERS=%s\n' "$(sh_quote "$static_peers")"
    printf 'STATIC_PEERS_COMPLETE=%s\n' "$(sh_quote "$static_peers_complete")"
    printf 'FIPS_ENABLE_BOOTSTRAP=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_FIPS_ENABLE_BOOTSTRAP:-}")"
    printf 'FIPS_OPEN_DISCOVERY_MAX_PENDING=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_FIPS_OPEN_DISCOVERY_MAX_PENDING:-}")"
    printf 'MIN_FREE_KB=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MIN_FREE_KB:-6291456}")"
    printf 'PRUNE_COMPILED_CACHE=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_PRUNE_COMPILED_CACHE:-0}")"
    printf 'CARGO_INCREMENTAL_DEFAULT=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_CARGO_INCREMENTAL:-0}")"
    printf 'CARGO_PROFILE_DEV_DEBUG_DEFAULT=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_CARGO_PROFILE_DEV_DEBUG:-0}")"
    printf 'IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER:-0}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_WRITE_APP_GROUP_RUNTIME=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_WRITE_APP_GROUP_RUNTIME:-}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM:-}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_SIGN_IDENTITY=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_SIGN_IDENTITY:-}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN:-}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN_PASS_FILE=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN_PASS_FILE:-}")"
    printf 'IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-}")"
    printf 'IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT:-}")"
    cat <<'REMOTE_SH'
set -Eeuo pipefail

MACOS_CODESIGN_KEYCHAIN=""
MACOS_XCODE_SIGNED_IDENTITY=""

FIPS_ENABLE_BOOTSTRAP_EFFECTIVE="$FIPS_ENABLE_BOOTSTRAP"
if [[ -z "$FIPS_ENABLE_BOOTSTRAP_EFFECTIVE" ]]; then
  if [[ "$STATIC_PEERS_COMPLETE" == "1" ]]; then
    FIPS_ENABLE_BOOTSTRAP_EFFECTIVE="false"
  else
    FIPS_ENABLE_BOOTSTRAP_EFFECTIVE="true"
  fi
fi

FIPS_OPEN_DISCOVERY_MAX_PENDING_EFFECTIVE="$FIPS_OPEN_DISCOVERY_MAX_PENDING"
if [[ -z "$FIPS_OPEN_DISCOVERY_MAX_PENDING_EFFECTIVE" ]]; then
  if [[ "$STATIC_PEERS_COMPLETE" == "1" ]]; then
    FIPS_OPEN_DISCOVERY_MAX_PENDING_EFFECTIVE="0"
  else
    FIPS_OPEN_DISCOVERY_MAX_PENDING_EFFECTIVE="16"
  fi
fi

log() {
  printf '[%s] %s\n' "$LABEL" "$*" >&2
}

die() {
  log "$*"
  exit 1
}

expand_path() {
  case "$1" in
    "~") printf '%s\n' "$HOME" ;;
    "~/"*) printf '%s/%s\n' "$HOME" "${1:2}" ;;
    /*) printf '%s\n' "$1" ;;
    *) printf '%s/%s\n' "$HOME" "$1" ;;
  esac
}

prepare_worktree() {
  local repo="$1"
  local name="$2"
  local dirty
  dirty="$(git -C "$repo" status --short)"
  if [[ -z "$dirty" ]]; then
    return 0
  fi
  if [[ "$FORCE" == "1" ]]; then
    return 0
  fi
  if [[ "$FAIL_ON_DIRTY" == "1" ]]; then
    printf '[%s] %s has local changes:\n%s\n' "$LABEL" "$name" "$dirty" >&2
    printf '[%s] rerun without --fail-on-dirty to stash, or with --force to discard tracked VM changes\n' "$LABEL" >&2
    exit 3
  fi
  log "stashing local $name changes before deploy"
  git -C "$repo" stash push --include-untracked -m "iris-drive dev-vm autosave $(date -u +%Y%m%dT%H%M%SZ)"
}

sync_repo() {
  local repo="$1"
  local name="$2"
  local bare="$3"
  local branch="${4:-$SYNC_BRANCH}"
  local checkout_branch="${5:-$TARGET_BRANCH}"

  bare="$(expand_path "$bare")"
  if [[ ! -d "$repo/.git" ]]; then
    if [[ -e "$repo" ]]; then
      log "sync path exists but is not a git checkout: $repo"
      exit 1
    fi
    log "creating checkout for $name at $repo"
    mkdir -p "$(dirname "$repo")"
    git clone "$bare" "$repo"
  fi
  prepare_worktree "$repo" "$name"
  log "fetching $name from $bare"
  git -C "$repo" fetch "$bare" "$branch"
  local fetched
  local current
  local current_branch
  fetched="$(git -C "$repo" rev-parse FETCH_HEAD)"
  current="$(git -C "$repo" rev-parse HEAD 2>/dev/null || true)"
  current_branch="$(git -C "$repo" symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
  if [[ "$FORCE" != "1" && "$current" == "$fetched" && "$current_branch" == "$checkout_branch" ]]; then
    log "$name already at $checkout_branch@${fetched:0:12}; leaving worktree untouched"
    return 0
  fi
  if [[ "$FORCE" == "1" ]]; then
    git -C "$repo" checkout --force -B "$checkout_branch" FETCH_HEAD
    git -C "$repo" reset --hard FETCH_HEAD
  else
    git -C "$repo" checkout -B "$checkout_branch" FETCH_HEAD
  fi
}

free_kb() {
  df -Pk "$1" | awk 'NR == 2 { print $4 }'
}

prune_rust_incremental_caches() {
  local target_dir="$1"
  [[ -d "$target_dir" ]] || return 0
  rm -rf "$target_dir/debug/incremental"
  for debug_dir in "$target_dir"/*/debug; do
    [[ -d "$debug_dir" ]] || continue
    rm -rf "$debug_dir/incremental"
  done
}

prune_rust_compiled_caches() {
  local target_dir="$1"
  [[ -d "$target_dir" ]] || return 0
  rm -rf \
    "$target_dir/debug/build" \
    "$target_dir/debug/deps"
  for debug_dir in "$target_dir"/*/debug; do
    [[ -d "$debug_dir" ]] || continue
    rm -rf \
      "$debug_dir/build" \
      "$debug_dir/deps"
  done
}

ensure_build_space() {
  local repo="$1"
  local phase="$2"
  local available=""
  local companion_target="$HOME/src/nostr-vpn/target"

  available="$(free_kb "$repo" 2>/dev/null || true)"
  [[ -n "$available" ]] || return 0
  if (( available >= MIN_FREE_KB )); then
    return 0
  fi

  log "low disk before $phase ($((available / 1024)) MiB free); pruning incremental build caches"
  prune_rust_incremental_caches "$repo/target"
  rm -rf \
    "$repo/macos/.build/DerivedData/Build/Intermediates.noindex" \
    "$repo/macos/.build/DerivedData/Index.noindex"

  available="$(free_kb "$repo" 2>/dev/null || true)"
  if [[ -n "$available" && "$available" -lt "$MIN_FREE_KB" && "$PRUNE_COMPILED_CACHE" == "1" ]]; then
    log "still below disk target; pruning compiled Cargo caches because IRIS_DRIVE_DEV_VM_PRUNE_COMPILED_CACHE=1"
    prune_rust_compiled_caches "$repo/target"
  fi

  available="$(free_kb "$repo" 2>/dev/null || true)"
  if [[ -n "$available" && "$available" -lt "$MIN_FREE_KB" && -d "$companion_target" ]]; then
    log "still below disk target; pruning nostr-vpn incremental caches"
    prune_rust_incremental_caches "$companion_target"
    if [[ "$PRUNE_COMPILED_CACHE" == "1" ]]; then
      prune_rust_compiled_caches "$companion_target"
    fi
  fi

  available="$(free_kb "$repo" 2>/dev/null || true)"
  if [[ -n "$available" && "$available" -lt "$MIN_FREE_KB" ]]; then
    log "warning: only $((available / 1024)) MiB free after pruning; build may still fail"
  fi
}

cargo_dev_build() {
  CARGO_INCREMENTAL="${CARGO_INCREMENTAL_DEFAULT}" \
    CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG_DEFAULT}" \
    cargo build "$@"
}

build_idrive() {
  local iris_repo="$1"
  local phase="$2"

  ensure_build_space "$iris_repo" "$phase"
  log "building idrive helper"
  (cd "$iris_repo" && cargo_dev_build -p idrive)
}

detect_overlay_ip() {
  local nvpn=""
  local ip=""
  if command -v nvpn >/dev/null 2>&1; then
    nvpn="$(command -v nvpn)"
  elif [[ -x "$HOME/src/nostr-vpn/target/debug/nvpn" ]]; then
    nvpn="$HOME/src/nostr-vpn/target/debug/nvpn"
  fi
  [[ -n "$nvpn" ]] || return 1
  ip="$("$nvpn" status --json 2>/dev/null | python3 -c 'import json,sys; print((json.load(sys.stdin).get("tunnel_ip") or "").split("/")[0])' 2>/dev/null || true)"
  [[ -n "$ip" ]] || return 1
  printf '%s\n' "$ip"
}

process_running() {
  local pid="$1"
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  kill -0 "$pid" >/dev/null 2>&1
}

terminate_pid() {
  local pid="$1"
  local i
  process_running "$pid" || return 0
  kill "$pid" >/dev/null 2>&1 || true
  for i in {1..15}; do
    process_running "$pid" || return 0
    sleep 0.1
  done
  kill -KILL "$pid" >/dev/null 2>&1 || true
}

detach_stale_mountpoint() {
  local mountpoint="$1"
  local mounted=0
  [[ -n "$mountpoint" ]] || return 0

  if command -v mountpoint >/dev/null 2>&1 && mountpoint -q "$mountpoint"; then
    mounted=1
  elif command -v findmnt >/dev/null 2>&1 &&
    findmnt -rn --mountpoint "$mountpoint" >/dev/null 2>&1; then
    mounted=1
  fi
  if (( ! mounted )); then
    return 0
  fi

  if command -v fusermount3 >/dev/null 2>&1; then
    fusermount3 -uz "$mountpoint" >/dev/null 2>&1 && return 0
  fi
  if command -v fusermount >/dev/null 2>&1; then
    fusermount -uz "$mountpoint" >/dev/null 2>&1 && return 0
  fi
  umount -l "$mountpoint" >/dev/null 2>&1 || true
}

stop_idrive_daemon() {
  local config_dir="$1"
  local mountpoint="${2:-}"
  local status_file="$config_dir/daemon-status.json"
  local lock_file="$config_dir/daemon.lock"
  local status_pid=""
  local lock_pid=""

  if [[ -f "$status_file" ]]; then
    status_pid="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("pid", ""))' "$status_file" 2>/dev/null || true)"
  fi
  if [[ -f "$lock_file" ]]; then
    lock_pid="$(tr -d '[:space:]' < "$lock_file" 2>/dev/null || true)"
  fi

  terminate_pid "$status_pid"
  if [[ "$lock_pid" != "$status_pid" ]]; then
    terminate_pid "$lock_pid"
  fi

  # The Linux dev app can leave behind an idrive daemon that owns the FUSE
  # mount but was not started with the lab config dir. Kill stale daemons by
  # mountpoint too so a lab deploy really replaces the running VM app.
  if [[ -n "$mountpoint" ]] && command -v pgrep >/dev/null 2>&1; then
    while IFS= read -r pid; do
      [[ -n "$pid" ]] || continue
      [[ "$pid" != "$status_pid" && "$pid" != "$lock_pid" ]] || continue
      local cmdline=""
      if [[ -r "/proc/$pid/cmdline" ]]; then
        cmdline="$(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null || true)"
      else
        cmdline="$(ps -p "$pid" -o command= 2>/dev/null || true)"
      fi
      [[ "$cmdline" == *"idrive"*" daemon "* ]] || continue
      if [[ "$cmdline" == *"$mountpoint"* || "$cmdline" == *"$config_dir"* ]]; then
        terminate_pid "$pid"
      fi
    done < <(pgrep -u "$(id -u)" -f "idrive.* daemon " 2>/dev/null || true)
    detach_stale_mountpoint "$mountpoint"
  fi
}

idrive_status_json_retry() {
  local idrive="$1"
  local config_dir="$2"
  local attempts="${3:-8}"
  local delay="${4:-0.5}"
  local attempt
  local err
  local output=""

  err="$(mktemp -t iris-drive-status.XXXXXX 2>/dev/null || printf '/tmp/iris-drive-status.%s' "$$")"
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    : > "$err"
    if output="$("$idrive" --config-dir "$config_dir" status 2>"$err")" \
      && python3 -c 'import json, sys; json.load(sys.stdin)' <<< "$output" >/dev/null 2>&1
    then
      rm -f "$err"
      printf '%s\n' "$output"
      return 0
    fi
    sleep "$delay"
  done

  if [[ -s "$err" ]]; then
    cat "$err" >&2
  fi
  rm -f "$err"
  return 1
}

idrive_status_fips_running() {
  local status_json="$1"
  python3 -c '
import json
import sys

data = json.load(sys.stdin)
fips = (data.get("network") or {}).get("fips") or {}
if fips:
    sys.exit(0 if fips.get("enabled") and fips.get("running") else 1)

daemon_fips = data.get("fips_block_sync")
daemon_error = data.get("fips_block_sync_error")
running = data.get("running") and data.get("fresh") is not False
has_fips = isinstance(daemon_fips, dict) and daemon_fips.get("endpoint_npub")
sys.exit(0 if running and has_fips and not daemon_error else 1)
' <<< "$status_json"
}

daemon_status_json_for_pid() {
  local config_dir="$1"
  local daemon_pid="$2"
  local status_file="$config_dir/daemon-status.json"
  [[ -f "$status_file" ]] || return 1
  STATUS_FILE="$status_file" DAEMON_PID="$daemon_pid" python3 - <<'PY'
import json
import os
import sys

with open(os.environ["STATUS_FILE"], "r", encoding="utf-8") as handle:
    data = json.load(handle)
if str(data.get("pid", "")) != os.environ["DAEMON_PID"]:
    sys.exit(1)
json.dump(data, sys.stdout, separators=(",", ":"))
sys.stdout.write("\n")
PY
}

wait_for_idrive_fips_status() {
  local idrive="$1"
  local config_dir="$2"
  local daemon_pid="$3"
  local timeout="${IRIS_DRIVE_DEV_VM_FIPS_READY_TIMEOUT:-300}"
  local started="$SECONDS"
  local status_json=""

  while (( SECONDS - started < timeout )); do
    if ! process_running "$daemon_pid"; then
      return 2
    fi
    if status_json="$(daemon_status_json_for_pid "$config_dir" "$daemon_pid")" \
      && idrive_status_fips_running "$status_json" >/dev/null; then
      printf '%s\n' "$status_json"
      return 0
    fi
    sleep 0.5
  done

  if status_json="$(daemon_status_json_for_pid "$config_dir" "$daemon_pid")" \
    && idrive_status_fips_running "$status_json" >/dev/null; then
    printf '%s\n' "$status_json"
    return 0
  fi
  if status_json="$(idrive_status_json_retry "$idrive" "$config_dir" 1 0.1)" \
    && idrive_status_fips_running "$status_json" >/dev/null; then
    printf '%s\n' "$status_json"
    return 0
  fi
  return 1
}

print_idrive_status_summary() {
  local status_json="$1"
  python3 -c '
import json
import sys

data = json.load(sys.stdin)
fips = (data.get("network") or {}).get("fips") or data.get("fips_block_sync") or {}
print("connected_peers=", fips.get("connected_peers"))
print(
    "peers=",
    [
        (peer.get("label"), peer.get("fips_online"), peer.get("sync_state"))
        for peer in data.get("peers", [])
    ],
)
' <<< "$status_json"
}

idrive_provider_list_retry() {
  local idrive="$1"
  local config_dir="$2"
  local output_file="$3"
  local attempts="${4:-8}"
  local delay="${5:-0.5}"
  local stderr_file="${output_file}.stderr"
  local attempt

  for ((attempt = 1; attempt <= attempts; attempt++)); do
    if "$idrive" --config-dir "$config_dir" provider list >"$output_file" 2>"$stderr_file" \
      && python3 - "$output_file" <<'PY' >/dev/null 2>&1
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as fh:
    json.load(fh)
PY
    then
      return 0
    fi
    sleep "$delay"
  done
  if [[ -s "$stderr_file" ]]; then
    cat "$stderr_file" >&2
  fi
  return 1
}

run_linux() {
  local iris_repo="$HOME/src/iris-drive"
  local idrive="$iris_repo/target/debug/idrive"
  local config_dir="${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-$HOME/.config/iris-drive}"
  local mountpoint="${IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT:-$HOME/Iris Drive}"

  build_idrive "$iris_repo" "Linux build"
  log "building Linux GTK app"; (cd "$iris_repo/linux" && cargo_dev_build)
  [[ "$NO_RUN" == "1" ]] && return 0

  log "restarting idrive daemon"
  mkdir -p "$config_dir"
  pkill -u "$(id -u)" -f "$iris_repo/linux/target/debug/iris-drive" >/dev/null 2>&1 || true; rm -f "$config_dir/app.lock"
  stop_idrive_daemon "$config_dir" "$mountpoint"
  mkdir -p "$mountpoint"
  rm -f "$config_dir/daemon.lock"
  nohup env \
    "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=0.0.0.0:$FIPS_PORT" \
    "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=" \
    "IRIS_DRIVE_FIPS_UDP_PUBLIC=false" \
    "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true" \
    "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=$FIPS_ENABLE_BOOTSTRAP_EFFECTIVE" \
    "IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=$FIPS_OPEN_DISCOVERY_MAX_PENDING_EFFECTIVE" \
    "IRIS_DRIVE_FIPS_STATIC_PEERS=$STATIC_PEERS" \
    "$idrive" --config-dir "$config_dir" daemon \
      --watch-debounce-ms 100 \
      --mount \
      --mountpoint "$mountpoint" \
      > /tmp/iris-drive-daemon.log 2>&1 < /dev/null &
  local daemon_pid="$!"
  disown "$daemon_pid" >/dev/null 2>&1 || true
  local status_json=""
  local wait_status=0
  status_json="$(wait_for_idrive_fips_status "$idrive" "$config_dir" "$daemon_pid")" || wait_status=$?
  if [[ "$wait_status" == "2" ]]; then
    tail -120 /tmp/iris-drive-daemon.log >&2 || true
    die "idrive daemon exited during startup"
  fi
  if [[ "$wait_status" != "0" ]]; then
    tail -120 /tmp/iris-drive-daemon.log >&2 || true
    die "idrive daemon did not report running FIPS"
  fi
  print_idrive_status_summary "$status_json"
}

write_macos_fileprovider_runtime() {
  local app_base="$1"
  local config_dir="$2"
  local idrive_path="$3"
  local app_group="$4"
  local runtime_dirs=(
    "$app_base"
    "$HOME/Library/Group Containers/$app_group/Iris Drive"
    "$HOME/Library/Application Support/Iris Drive"
  )

  python3 - "$config_dir" "$idrive_path" "${runtime_dirs[@]}" <<'PY'
import json
import os
import sys

config_dir, idrive_path, *directories = sys.argv[1:]
payload = {
    "config_dir": config_dir,
    "idrive_executable": idrive_path,
}
seen = set()
for directory in directories:
    directory = os.path.abspath(os.path.expanduser(directory))
    if directory in seen:
        continue
    seen.add(directory)
    os.makedirs(directory, exist_ok=True)
    path = os.path.join(directory, "fileprovider-runtime.json")
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, separators=(",", ":"))
        handle.write("\n")
PY
}

macos_app_group_identifier() {
  local explicit="${IRIS_DRIVE_DEV_VM_MACOS_APP_GROUP_IDENTIFIER:-}"
  local team="${IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM:-}"
  if [[ -n "$explicit" ]]; then
    printf '%s\n' "$explicit"
    return 0
  fi
  team="${team%.}"
  if [[ -n "$team" ]]; then
    printf '%s.to.iris.drive\n' "$team"
    return 0
  fi
  printf '%s\n' "group.to.iris.drive"
}

macos_embedded_profile_team() {
  local app="$1"
  local profile="$app/Contents/embedded.provisionprofile"
  local decoded

  [[ -f "$profile" ]] || return 0
  decoded="$(mktemp -t iris-drive-profile.XXXXXX.plist)"
  if security cms -D -i "$profile" > "$decoded" 2>/dev/null; then
    python3 - "$decoded" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "rb") as handle:
    profile = plistlib.load(handle)
teams = profile.get("TeamIdentifier") or []
if teams:
    print(teams[0])
PY
  fi
  rm -f "$decoded"
}

macos_codesign_team_identifier() {
  local bundle="$1"
  codesign -dvv "$bundle" 2>&1 \
    | sed -n 's/^TeamIdentifier=//p' \
    | head -n 1 || true
}

macos_entitlement_team_identifier() {
  local app="$1"
  local team="${IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM:-}"
  team="${team%.}"
  if [[ -n "$team" ]]; then
    printf '%s\n' "$team"
    return 0
  fi

  team="$(macos_embedded_profile_team "$app")"
  team="${team%.}"
  if [[ -n "$team" ]]; then
    printf '%s\n' "$team"
    return 0
  fi

  team="$(macos_codesign_team_identifier "$app")"
  team="${team%.}"
  if [[ -n "$team" ]]; then
    printf '%s\n' "$team"
  fi
}

macos_prepare_entitlements_for_signing() {
  local entitlements="$1"
  local team="$2"
  local output

  [[ -n "$entitlements" && -f "$entitlements" ]] || return 0
  if grep -q '\$(TeamIdentifierPrefix)' "$entitlements" && [[ -z "$team" ]]; then
    return 0
  fi

  output="$(mktemp -t iris-drive-entitlements.XXXXXX.plist)"
  IRIS_DRIVE_MACOS_ENTITLEMENT_TEAM="$team" \
  IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER="${IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER:-}" \
  IRIS_DRIVE_DEV_VM_MACOS_KEEP_PROVISIONED_DEBUG_ENTITLEMENTS="${IRIS_DRIVE_DEV_VM_MACOS_KEEP_PROVISIONED_DEBUG_ENTITLEMENTS:-}" \
  IRIS_DRIVE_DEV_VM_MACOS_KEEP_FILEPROVIDER_TESTING_MODE="${IRIS_DRIVE_DEV_VM_MACOS_KEEP_FILEPROVIDER_TESTING_MODE:-}" \
    python3 - "$entitlements" "$output" <<'PY'
import os
import plistlib
import sys

source, destination = sys.argv[1], sys.argv[2]
team = os.environ.get("IRIS_DRIVE_MACOS_ENTITLEMENT_TEAM", "").rstrip(".")


def truthy(name, default=False):
    value = os.environ.get(name)
    if value is None or value == "":
        return default
    return value in {"1", "true", "TRUE", "True", "yes", "YES", "Yes", "on", "ON", "On"}


def expand(value):
    if isinstance(value, str) and team:
        return value.replace("$(TeamIdentifierPrefix)", f"{team}.")
    if isinstance(value, list):
        return [expand(item) for item in value]
    if isinstance(value, dict):
        return {key: expand(item) for key, item in value.items()}
    return value


with open(source, "rb") as handle:
    entitlements = expand(plistlib.load(handle))

keep_provisioned = truthy("IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER") or truthy(
    "IRIS_DRIVE_DEV_VM_MACOS_KEEP_PROVISIONED_DEBUG_ENTITLEMENTS"
)
keep_fileprovider_testing = truthy(
    "IRIS_DRIVE_DEV_VM_MACOS_KEEP_FILEPROVIDER_TESTING_MODE",
    keep_provisioned,
)

if not keep_provisioned:
    entitlements.pop("com.apple.developer.associated-domains", None)
    entitlements.pop("com.apple.security.application-groups", None)
if not keep_fileprovider_testing:
    entitlements.pop("com.apple.developer.fileprovider.testing-mode", None)

with open(destination, "wb") as handle:
    plistlib.dump(entitlements, handle, sort_keys=False)
PY
  printf '%s\n' "$output"
}

macos_embedded_profile_codesign_identity() {
  local app="$1"
  local profile="$app/Contents/embedded.provisionprofile"
  local decoded
  local identities

  [[ -f "$profile" ]] || return 0
  decoded="$(mktemp -t iris-drive-profile.XXXXXX.plist)"
  security cms -D -i "$profile" > "$decoded" 2>/dev/null || {
    rm -f "$decoded"
    return 0
  }
  identities="$(security find-identity -v -p codesigning 2>/dev/null || true)"
  python3 - "$decoded" "$identities" <<'PY'
import hashlib
import plistlib
import sys

with open(sys.argv[1], "rb") as handle:
    profile = plistlib.load(handle)
identities = sys.argv[2].upper()
for cert in profile.get("DeveloperCertificates", []):
    fingerprint = hashlib.sha1(cert).hexdigest().upper()
    if fingerprint in identities:
        print(fingerprint)
        break
PY
  rm -f "$decoded"
}

copy_macos_dev_tree_best_effort() {
  local source="$1"
  local destination="$2"

  [[ -d "$source" ]] || return 0
  mkdir -p "$destination"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a --ignore-errors "$source"/ "$destination"/ >/dev/null 2>&1 \
      || log "warning: some files could not be migrated from $source"
  else
    ditto "$source" "$destination" >/dev/null 2>&1 \
      || log "warning: some files could not be migrated from $source"
  fi
}

macos_fileprovider_required() {
  case "${IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

unlock_macos_build_keychain() {
  local keychain="${IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN:-}"
  local pass_file="${IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN_PASS_FILE:-}"

  if [[ -z "$keychain" ]]; then
    keychain="$HOME/Library/Keychains/iris-drive-build.keychain-db"
    [[ -f "$keychain" ]] || return 0
  else
    keychain="$(expand_path "$keychain")"
  fi

  if [[ -z "$pass_file" ]]; then
    pass_file="$HOME/.config/iris-drive/dev-build-keychain.pass"
  else
    pass_file="$(expand_path "$pass_file")"
  fi

  [[ -f "$keychain" && -f "$pass_file" ]] || return 0
  log "unlocking macOS signing keychain"
  security unlock-keychain -p "$(cat "$pass_file")" "$keychain" >/dev/null
  MACOS_CODESIGN_KEYCHAIN="$keychain"
}

ensure_macos_codesign_chain() {
  [[ -n "$MACOS_CODESIGN_KEYCHAIN" ]] || return 0

  local certs
  local keychain
  certs="$(mktemp -t iris-drive-apple-certs.XXXXXX)"
  for keychain in \
    "$HOME/Library/Keychains/login.keychain-db" \
    /Library/Keychains/System.keychain \
    /System/Library/Keychains/SystemRootCertificates.keychain
  do
    security find-certificate -a -p -c "Apple Worldwide Developer Relations" "$keychain" 2>/dev/null || true
    security find-certificate -a -p -c "Apple Root CA" "$keychain" 2>/dev/null || true
  done > "$certs"
  if [[ -s "$certs" ]]; then
    security import "$certs" -k "$MACOS_CODESIGN_KEYCHAIN" -A >/dev/null 2>&1 || true
  fi
  rm -f "$certs"
}

with_macos_keychain_only() {
  local keychain="$1"
  shift
  local current_file
  local status
  local restored=()
  local line

  current_file="$(mktemp -t iris-drive-keychains.XXXXXX)"
  security list-keychains -d user > "$current_file"
  security list-keychains -d user -s "$keychain"

  set +e
  "$@"
  status=$?
  set -e

  while IFS= read -r line; do
    line="${line//\"/}"
    line="$(printf '%s' "$line" | xargs)"
    [[ -n "$line" ]] && restored+=("$line")
  done < "$current_file"
  if [[ ${#restored[@]} -gt 0 ]]; then
    security list-keychains -d user -s "${restored[@]}" >/dev/null
  fi
  rm -f "$current_file"
  return "$status"
}

xcodebuild_macos_app() {
  local iris_repo="$1" rust_lib_dir
  ensure_build_space "$iris_repo" "macOS app-core build"; log "building macOS app-core library"
  (cd "$iris_repo" && cargo_dev_build -p iris-drive-app-core)
  rust_lib_dir="$(cd "$iris_repo" && cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug"
  local args=(xcodebuild -project macos/IrisDriveMac.xcodeproj -scheme IrisDriveMac -configuration Debug -derivedDataPath macos/.build/DerivedData
    "LIBRARY_SEARCH_PATHS=$rust_lib_dir"
    "OTHER_LDFLAGS=$rust_lib_dir/libiris_drive_app_core.a"
  )

  if macos_fileprovider_required; then
    args+=(
      -allowProvisioningUpdates
      -allowProvisioningDeviceRegistration
      CODE_SIGN_STYLE=Automatic
      CODE_SIGNING_ALLOWED=YES
      "CODE_SIGN_IDENTITY=Apple Development"
    )
    if [[ -n "${IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM:-}" ]]; then
      args+=("DEVELOPMENT_TEAM=$IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM")
    fi
    if [[ -n "$MACOS_CODESIGN_KEYCHAIN" ]]; then
      args+=("OTHER_CODE_SIGN_FLAGS=--keychain $MACOS_CODESIGN_KEYCHAIN")
      (cd "$iris_repo" && with_macos_keychain_only "$MACOS_CODESIGN_KEYCHAIN" "${args[@]}" build)
    else
      (cd "$iris_repo" && "${args[@]}" build)
    fi
  else
    args+=(CODE_SIGNING_ALLOWED=NO)
    (cd "$iris_repo" && "${args[@]}" build)
  fi
}

run_macos() {
  local iris_repo="$HOME/src/iris-drive"
  local idrive="$iris_repo/target/debug/idrive"
  local built_app="$iris_repo/macos/.build/DerivedData/Build/Products/Debug/Iris Drive.app"
  local app="${IRIS_DRIVE_DEV_VM_MACOS_APP_PATH:-$iris_repo/macos/.build/Applications/Iris Drive.app}"
  local daemon_idrive="$app/Contents/MacOS/idrive"
  local appex="$app/Contents/PlugIns/IrisDriveFileProvider.appex"
  local app_group
  app_group="$(macos_app_group_identifier)"
  local group_app_base="$HOME/Library/Group Containers/$app_group/Iris Drive Dev"
  local legacy_group_app_base="$HOME/Library/Group Containers/group.to.iris.drive/Iris Drive Dev"
  local sandbox_app_base="$HOME/Library/Containers/to.iris.drive.macos/Data/Library/Application Support/Iris Drive Dev"
  local app_base="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$group_app_base}"
  local old_dev_app_base="$HOME/Library/Application Support/Iris Drive Dev"
  local legacy_app_base="$HOME/.local/share/iris-drive-dev-app"
  local config_dir="$app_base/Config"
  local app_stdout="/tmp/iris-drive-macos-app.out"
  local app_stderr="/tmp/iris-drive-macos-app.err"
  local daemon_log="/tmp/iris-drive-macos-daemon.log"
  local daemon_pid=""

  build_idrive "$iris_repo" "idrive helper build"
  ensure_build_space "$iris_repo" "macOS app build"
  unlock_macos_build_keychain
  ensure_macos_codesign_chain
  log "building macOS app"
  xcodebuild_macos_app "$iris_repo"
  if macos_fileprovider_required; then
    MACOS_XCODE_SIGNED_IDENTITY="$(codesign -dv --verbose=4 "$built_app" 2>&1 \
      | sed -n 's/^Authority=\(Apple Development.*\)$/\1/p' \
      | head -n 1 || true)"
    if [[ -n "$MACOS_XCODE_SIGNED_IDENTITY" && -n "$MACOS_CODESIGN_KEYCHAIN" ]]; then
      MACOS_XCODE_SIGNED_IDENTITY="$(security find-certificate \
        -Z \
        -c "$MACOS_XCODE_SIGNED_IDENTITY" \
        "$MACOS_CODESIGN_KEYCHAIN" 2>/dev/null \
        | sed -n 's/^SHA-1 hash: //p' \
      | head -n 1 || true)"
    fi
  fi
  install_macos_dev_app "$built_app" "$app"
  if [[ -z "${IRIS_DRIVE_DEV_VM_MACOS_APP_GROUP_IDENTIFIER:-}" && -z "${IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM:-}" ]]; then
    local profile_team
    profile_team="$(macos_embedded_profile_team "$app")"
    if [[ -n "$profile_team" ]]; then
      app_group="$profile_team.to.iris.drive"
      group_app_base="$HOME/Library/Group Containers/$app_group/Iris Drive Dev"
      app_base="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$group_app_base}"
      config_dir="$app_base/Config"
    fi
  fi
  cp "$idrive" "$app/Contents/MacOS/idrive"
  cp "$idrive" "$appex/Contents/MacOS/idrive"
  chmod +x "$app/Contents/MacOS/idrive"
  chmod +x "$appex/Contents/MacOS/idrive"
  sign_macos_app "$iris_repo" "$app" "$appex"
  check_macos_fileprovider_signing "$app" "$appex"
  register_macos_app_bundle "$app" "$built_app"
  register_fileprovider_plugin "$app" "$appex"
  [[ "$NO_RUN" == "1" ]] && return 0

  log "restarting macOS app"
  pkill -x "Iris Drive" >/dev/null 2>&1 || true
  pkill -f IrisDriveFileProvider >/dev/null 2>&1 || true
  pkill -x fileproviderd >/dev/null 2>&1 || true
  pkill -x idrive >/dev/null 2>&1 || true
  mkdir -p "$config_dir"
  if [[ ! -f "$config_dir/key" ]]; then
    for migration_source in "$legacy_group_app_base" "$sandbox_app_base" "$old_dev_app_base" "$legacy_app_base"; do
      [[ "$migration_source" != "$app_base" ]] || continue
      [[ -f "$migration_source/Config/key" ]] || continue
      log "migrating macOS dev app data from $migration_source"
      mkdir -p "$app_base"
      copy_macos_dev_tree_best_effort "$migration_source/Config" "$config_dir"
      if [[ -d "$migration_source/Hashtree" ]]; then
        copy_macos_dev_tree_best_effort "$migration_source/Hashtree" "$app_base/Hashtree"
      fi
      break
    done
  fi
  write_macos_fileprovider_runtime \
    "$app_base" \
    "$config_dir" \
    "$app/Contents/MacOS/idrive" \
    "$app_group"
  pregrant_macos_dev_app_tcc
  stop_idrive_daemon "$config_dir"
  rm -f "$config_dir/daemon.lock"
  rm -f "$app_stdout" "$app_stderr" "$daemon_log"
  local reset_fileprovider_domain="true"
  case "${IRIS_DRIVE_DEV_VM_MACOS_RESET_FILEPROVIDER:-1}" in
    0|false|FALSE|no|NO|off|OFF) reset_fileprovider_domain="false" ;;
  esac
  sleep 1
  local open_args=(
    --stdout "$app_stdout"
    --stderr "$app_stderr"
    --env "IRIS_DRIVE_EXTERNAL_DAEMON=true"
    --env "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=0.0.0.0:$FIPS_PORT"
    --env "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR="
    --env "IRIS_DRIVE_FIPS_UDP_PUBLIC=false"
    --env "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true"
    --env "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=$FIPS_ENABLE_BOOTSTRAP_EFFECTIVE"
    --env "IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=$FIPS_OPEN_DISCOVERY_MAX_PENDING_EFFECTIVE"
    --env "IRIS_DRIVE_FIPS_STATIC_PEERS=$STATIC_PEERS"
    --env "IRIS_DRIVE_APP_BASE_DIR=$app_base"
    --env "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC=true"
    --env "IRIS_DRIVE_FILEPROVIDER_RUNTIME_EXTERNAL=true"
  )
  if [[ "$reset_fileprovider_domain" == "true" ]]; then
    open_args+=(--env "IRIS_DRIVE_FILEPROVIDER_RESET_ON_START=true")
  fi
  open \
    "${open_args[@]}" \
    "$app"
  for _ in {1..30}; do
    if pgrep -x "Iris Drive" >/dev/null 2>&1; then
      break
    fi
    sleep 0.5
  done
  if ! pgrep -x "Iris Drive" >/dev/null 2>&1; then
    log "macOS app did not stay running"
    tail -n 80 "$app_stderr" >&2 2>/dev/null || true
    exit 4
  fi

  log "starting macOS idrive daemon outside LaunchServices"
  nohup env \
    "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=0.0.0.0:$FIPS_PORT" \
    "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=" \
    "IRIS_DRIVE_FIPS_UDP_PUBLIC=false" \
    "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true" \
    "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=$FIPS_ENABLE_BOOTSTRAP_EFFECTIVE" \
    "IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=$FIPS_OPEN_DISCOVERY_MAX_PENDING_EFFECTIVE" \
    "IRIS_DRIVE_FIPS_STATIC_PEERS=$STATIC_PEERS" \
    "$daemon_idrive" --config-dir "$config_dir" daemon \
      --watch-debounce-ms 100 \
      > "$daemon_log" 2>&1 < /dev/null &
  daemon_pid="$!"
  disown "$daemon_pid" >/dev/null 2>&1 || true
  local status_json=""
  local wait_status=0
  status_json="$(wait_for_idrive_fips_status "$daemon_idrive" "$config_dir" "$daemon_pid")" || wait_status=$?
  if [[ "$wait_status" == "2" ]]; then
    log "macOS idrive daemon exited during startup"
    tail -n 120 "$daemon_log" >&2 2>/dev/null || true
    exit 4
  fi
  if [[ "$wait_status" != "0" ]]; then
    log "macOS idrive daemon did not report running FIPS status"
    tail -n 160 "$daemon_log" >&2 2>/dev/null || true
    exit 4
  fi
  if ! idrive_provider_list_retry "$daemon_idrive" "$config_dir" /tmp/iris-drive-macos-provider-list.json 120 1; then
    log "macOS virtual provider list failed"
    cat /tmp/iris-drive-macos-provider-list.json >&2 2>/dev/null || true
    tail -n 120 "$app_stderr" >&2 2>/dev/null || true
    exit 4
  fi
  print_idrive_status_summary "$status_json"
}

sign_macos_app() {
  local iris_repo="$1"
  local app="$2"
  local appex="$3"
  local sign_identity="${IRIS_DRIVE_DEV_VM_MACOS_SIGN_IDENTITY:-}"
  local app_entitlements="$iris_repo/macos/IrisDriveMac.entitlements"
  local appex_entitlements="$iris_repo/macos/FileProvider/FileProvider.entitlements"
  local helper_entitlements="$iris_repo/macos/idrive-helper.entitlements"
  local xcode_app_entitlements="$iris_repo/macos/.build/DerivedData/Build/Intermediates.noindex/IrisDriveMac.build/Debug/IrisDriveMac.build/Iris Drive.app.xcent"
  local xcode_appex_entitlements="$iris_repo/macos/.build/DerivedData/Build/Intermediates.noindex/IrisDriveMac.build/Debug/IrisDriveFileProvider.build/IrisDriveFileProvider.appex.xcent"
  local app_dev_entitlements=""
  local appex_dev_entitlements=""
  local generated_app_entitlements=""
  local generated_appex_entitlements=""
  local entitlement_team=""
  local source_app_entitlements=""
  local source_appex_entitlements=""

  if macos_fileprovider_required; then
    [[ -f "$xcode_app_entitlements" ]] && app_entitlements="$xcode_app_entitlements"
    [[ -f "$xcode_appex_entitlements" ]] && appex_entitlements="$xcode_appex_entitlements"
    entitlement_team="$(macos_entitlement_team_identifier "$app")"
  fi
  source_app_entitlements="$app_entitlements"
  source_appex_entitlements="$appex_entitlements"

  if [[ -z "$sign_identity" && -n "$MACOS_XCODE_SIGNED_IDENTITY" ]]; then
    sign_identity="$MACOS_XCODE_SIGNED_IDENTITY"
  fi

  if [[ -z "$sign_identity" ]]; then
    sign_identity="$(macos_embedded_profile_codesign_identity "$app")"
  fi

  if [[ -z "$sign_identity" ]]; then
    sign_identity="$(security find-identity -v -p codesigning 2>/dev/null \
      | sed -n 's/.*"\(Apple Development[^"]*\)".*/\1/p' \
      | head -n 1 || true)"
  fi

  if [[ -z "$sign_identity" ]]; then
    sign_identity="-"
    app_dev_entitlements="$(mktemp -t iris-drive-dev-app-entitlements.XXXXXX.plist)"
    appex_dev_entitlements="$(mktemp -t iris-drive-dev-appex-entitlements.XXXXXX.plist)"
    cat > "$app_dev_entitlements" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.app-sandbox</key>
  <true/>
  <key>com.apple.security.network.client</key>
  <true/>
  <key>com.apple.security.network.server</key>
  <true/>
</dict>
</plist>
EOF
    cat > "$appex_dev_entitlements" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.app-sandbox</key>
  <true/>
  <key>com.apple.security.network.client</key>
  <true/>
</dict>
</plist>
EOF
    app_entitlements="$app_dev_entitlements"
    appex_entitlements="$appex_dev_entitlements"
    log "codesigning macOS app ad-hoc with dev entitlements"
  else
    log "codesigning macOS app with identity: $sign_identity"
  fi

  if [[ "$sign_identity" != "-" ]]; then
    generated_app_entitlements="$(
      macos_prepare_entitlements_for_signing "$app_entitlements" "$entitlement_team"
    )"
    generated_appex_entitlements="$(
      macos_prepare_entitlements_for_signing "$appex_entitlements" "$entitlement_team"
    )"
    app_entitlements="${generated_app_entitlements:-$app_entitlements}"
    appex_entitlements="${generated_appex_entitlements:-$appex_entitlements}"
  fi

  local codesign_base=(codesign --force --sign "$sign_identity")
  if [[ -n "$MACOS_CODESIGN_KEYCHAIN" && "$sign_identity" != "-" ]]; then
    codesign_base+=(--keychain "$MACOS_CODESIGN_KEYCHAIN")
  fi

  "${codesign_base[@]}" \
    --entitlements "$helper_entitlements" \
    "$app/Contents/MacOS/idrive" >/dev/null
  if [[ -f "$appex/Contents/MacOS/idrive" ]]; then
    "${codesign_base[@]}" \
      --entitlements "$helper_entitlements" \
      "$appex/Contents/MacOS/idrive" >/dev/null
  fi
  if [[ -n "$appex_entitlements" ]]; then
    "${codesign_base[@]}" \
      --entitlements "$appex_entitlements" \
      "$appex" >/dev/null
  else
    "${codesign_base[@]}" "$appex" >/dev/null
  fi
  if [[ -n "$app_entitlements" ]]; then
    "${codesign_base[@]}" \
      --entitlements "$app_entitlements" \
      "$app" >/dev/null
  else
    "${codesign_base[@]}" "$app" >/dev/null
  fi

  if [[ "$sign_identity" != "-" ]] &&
    { grep -q '\$(TeamIdentifierPrefix)' "$source_app_entitlements" 2>/dev/null ||
      grep -q '\$(TeamIdentifierPrefix)' "$source_appex_entitlements" 2>/dev/null; }; then
    if [[ -z "$entitlement_team" ]]; then
      entitlement_team="$(macos_codesign_team_identifier "$app")"
      entitlement_team="${entitlement_team%.}"
    fi
    [[ -n "$entitlement_team" ]] \
      || die "cannot resolve TeamIdentifierPrefix from signed macOS app"

    if [[ -z "$generated_appex_entitlements" ]]; then
      generated_appex_entitlements="$(
        macos_prepare_entitlements_for_signing "$source_appex_entitlements" "$entitlement_team"
      )"
    fi
    if [[ -n "$generated_appex_entitlements" ]]; then
      "${codesign_base[@]}" \
        --entitlements "$generated_appex_entitlements" \
        "$appex" >/dev/null
    fi

    if [[ -z "$generated_app_entitlements" ]]; then
      generated_app_entitlements="$(
        macos_prepare_entitlements_for_signing "$source_app_entitlements" "$entitlement_team"
      )"
    fi
    if [[ -n "$generated_app_entitlements" ]]; then
      "${codesign_base[@]}" \
        --entitlements "$generated_app_entitlements" \
        "$app" >/dev/null
    fi
  fi

  rm -f \
    "$app_dev_entitlements" \
    "$appex_dev_entitlements" \
    "$generated_app_entitlements" \
    "$generated_appex_entitlements"
  codesign --verify --deep --strict --verbose=2 "$app" >/dev/null
}

install_macos_dev_app() {
  local built_app="$1"
  local app="$2"

  [[ -d "$built_app" ]] || die "built macOS app not found at $built_app"
  [[ "$app" == *.app && "$app" != "/" ]] || die "unsafe macOS app install path: $app"
  mkdir -p "$(dirname "$app")"
  rm -rf "$app"
  ditto "$built_app" "$app"
}

register_macos_app_bundle() {
  local app="$1"
  local built_app="$2"
  local lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
  local candidate
  local stale_root

  [[ -x "$lsregister" ]] || return 0
  "$lsregister" -u "$built_app" >/dev/null 2>&1 || true
  if command -v mdfind >/dev/null 2>&1; then
    mdfind "kMDItemCFBundleIdentifier == 'to.iris.drive.macos'" 2>/dev/null \
      | while IFS= read -r candidate; do
          [[ -n "$candidate" && "$candidate" != "$app" ]] || continue
          "$lsregister" -u "$candidate" >/dev/null 2>&1 || true
        done
  fi
  if [[ -d "$HOME/Library/Developer/Xcode/DerivedData" ]]; then
    find "$HOME/Library/Developer/Xcode/DerivedData" \
      -path "*/Build/Products/Debug/Iris Drive.app" \
      -type d -prune -print 2>/dev/null \
      | while IFS= read -r candidate; do
          [[ -n "$candidate" && "$candidate" != "$app" ]] || continue
          "$lsregister" -u "$candidate" >/dev/null 2>&1 || true
          rm -rf "$candidate"
        done
  fi
  for stale_root in /private/tmp/iris-drive-sign-tests /tmp/iris-drive-sign-tests; do
    [[ -d "$stale_root" ]] || continue
    find "$stale_root" \
      -name "*.app" \
      -type d -prune -print 2>/dev/null \
      | while IFS= read -r candidate; do
          "$lsregister" -u "$candidate" >/dev/null 2>&1 || true
        done
    rm -rf "$stale_root"
  done
  "$lsregister" -f -R -trusted "$app" >/dev/null 2>&1 || true
}

register_fileprovider_plugin() {
  local app="$1"
  local appex="$2"
  local plugin_id="to.iris.drive.macos.FileProvider"
  local plugin
  command -v pluginkit >/dev/null 2>&1 || return 0

  pluginkit -m -i "$plugin_id" -ADv 2>/dev/null \
    | awk -F '\t' 'NF >= 4 { print $4 }' \
    | while IFS= read -r plugin; do
        if [[ -n "$plugin" && "$plugin" != "$appex" ]]; then
          pluginkit -r "$plugin" >/dev/null 2>&1 || true
        fi
      done
  register_macos_app_bundle "$app" "$app"
  pluginkit -a "$appex" >/dev/null 2>&1 || true
  pluginkit -e use -i "$plugin_id" >/dev/null 2>&1 || true
}

pregrant_macos_dev_app_tcc() {
  case "${IRIS_DRIVE_DEV_VM_MACOS_PREGRANT_TCC:-1}" in
    0|false|FALSE|no|NO|off|OFF) return 0 ;;
  esac
  command -v sqlite3 >/dev/null 2>&1 || return 0

  local tcc_db="$HOME/Library/Application Support/com.apple.TCC/TCC.db"
  [[ -f "$tcc_db" && -w "$tcc_db" ]] || return 0

  sqlite3 "$tcc_db" <<'SQL' >/dev/null 2>&1 || true
INSERT OR REPLACE INTO access(
  service,
  client,
  client_type,
  auth_value,
  auth_reason,
  auth_version,
  csreq,
  policy_id,
  indirect_object_identifier_type,
  indirect_object_identifier,
  indirect_object_code_identity,
  flags,
  last_modified,
  pid,
  pid_version,
  boot_uuid,
  last_reminded
) VALUES (
  'kTCCServiceSystemPolicyAppData',
  'to.iris.drive.macos',
  0,
  2,
  4,
  2,
  NULL,
  NULL,
  0,
  'UNUSED',
  NULL,
  0,
  CAST(strftime('%s','now') AS INTEGER),
  NULL,
  NULL,
  'UNUSED',
  CAST(strftime('%s','now') AS INTEGER)
);
SQL
  killall tccd >/dev/null 2>&1 || true
}

check_macos_fileprovider_signing() {
  local app="$1"
  local appex="$2"
  local require="${IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER:-0}"

  if codesign -d --entitlements :- "$app" 2>/dev/null \
      | grep -q 'com.apple.developer.fileprovider.testing-mode' \
    && codesign -d --entitlements :- "$appex" 2>/dev/null \
      | grep -q 'com.apple.developer.fileprovider.testing-mode'; then
    log "macOS app signed with FileProvider testing entitlement"
    return 0
  fi

  case "$require" in
    1|true|TRUE|yes|YES|on|ON)
      die "macOS FileProvider requires Apple Development signing; no FileProvider-capable entitlements are present"
      ;;
  esac

  log "warning: macOS app is not FileProvider-capable in this signing mode; install an Apple Development identity or set IRIS_DRIVE_DEV_VM_MACOS_SIGN_IDENTITY to test the real drive domain"
}

ensure_build_space "$HOME/src/iris-drive" "repository sync"
sync_repo "$HOME/src/nostr-social-graph" nostr-social-graph "$SOCIAL_GRAPH_BARE" "$SOCIAL_GRAPH_SYNC_BRANCH" "$SOCIAL_GRAPH_TARGET_BRANCH"
sync_repo "$HOME/src/cashu-service" cashu-service "$CASHU_SERVICE_BARE"
sync_repo "$HOME/src/hashtree" hashtree "$HASHTREE_BARE"
sync_repo "$HOME/src/fips" fips "$FIPS_BARE" "$FIPS_SYNC_BRANCH" "$FIPS_TARGET_BRANCH"
sync_repo "$HOME/src/iris-drive" iris-drive "$IRIS_BARE"
case "$KIND" in
  macos) run_macos ;;
  linux) run_linux ;;
  *) log "unknown POSIX kind: $KIND"; exit 2 ;;
esac
REMOTE_SH
  } | ssh "$host" 'bash -se'
}

run_windows_target() {
  local label="$1"
  local host="$2"
  local iris_bare="$3"
  local hashtree_bare="$4"
  local fips_bare="$5"
  local social_graph_bare="$6"
  local cashu_service_bare="$7"
  local static_peers="$8"
  local static_peers_complete="$9"

  {
    printf '$Label = %s\n' "$(ps_quote "$label")"
    printf '$IrisBare = %s\n' "$(ps_quote "$iris_bare")"
    printf '$HashtreeBare = %s\n' "$(ps_quote "$hashtree_bare")"
    printf '$FipsBare = %s\n' "$(ps_quote "$fips_bare")"
    printf '$SocialGraphBare = %s\n' "$(ps_quote "$social_graph_bare")"
    printf '$CashuServiceBare = %s\n' "$(ps_quote "$cashu_service_bare")"
    printf '$SyncBranch = %s\n' "$(ps_quote "$SYNC_BRANCH")"
    printf '$FipsSyncBranch = %s\n' "$(ps_quote "$FIPS_SYNC_BRANCH")"
    printf '$SocialGraphSyncBranch = %s\n' "$(ps_quote "$SOCIAL_GRAPH_SYNC_BRANCH")"
    printf '$TargetBranch = %s\n' "$(ps_quote "$TARGET_BRANCH")"
    printf '$FipsTargetBranch = %s\n' "$(ps_quote "$FIPS_TARGET_BRANCH")"
    printf '$SocialGraphTargetBranch = %s\n' "$(ps_quote "$SOCIAL_GRAPH_TARGET_BRANCH")"
    printf '$Force = %s\n' "$(ps_quote "$FORCE")"
    printf '$FailOnDirty = %s\n' "$(ps_quote "$FAIL_ON_DIRTY")"
    printf '$NoRun = %s\n' "$(ps_quote "$NO_RUN")"
    printf '$FipsPort = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_FIPS_PORT:-22121}")"
    printf '$StaticPeers = %s\n' "$(ps_quote "$static_peers")"
    printf '$StaticPeersComplete = %s\n' "$(ps_quote "$static_peers_complete")"
    printf '$FipsEnableBootstrap = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_FIPS_ENABLE_BOOTSTRAP:-}")"
    printf '$FipsOpenDiscoveryMaxPending = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_FIPS_OPEN_DISCOVERY_MAX_PENDING:-}")"
    printf '$CargoIncrementalDefault = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_CARGO_INCREMENTAL:-0}")"
    printf '$CargoProfileDevDebugDefault = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_CARGO_PROFILE_DEV_DEBUG:-0}")"
    printf '$WindowsConfigDirOverride = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_WINDOWS_CONFIG_DIR:-}")"
    cat <<'REMOTE_PS'
$ErrorActionPreference = "Stop"

$FipsEnableBootstrapEffective = $FipsEnableBootstrap
if ([string]::IsNullOrWhiteSpace($FipsEnableBootstrapEffective)) {
  if ($StaticPeersComplete -eq "1") {
    $FipsEnableBootstrapEffective = "false"
  } else {
    $FipsEnableBootstrapEffective = "true"
  }
}

$FipsOpenDiscoveryMaxPendingEffective = $FipsOpenDiscoveryMaxPending
if ([string]::IsNullOrWhiteSpace($FipsOpenDiscoveryMaxPendingEffective)) {
  if ($StaticPeersComplete -eq "1") {
    $FipsOpenDiscoveryMaxPendingEffective = "0"
  } else {
    $FipsOpenDiscoveryMaxPendingEffective = "16"
  }
}

function Write-Log([string]$Message) {
  [Console]::Error.WriteLine("[$Label] $Message")
}

function Expand-RemotePath([string]$Path) {
  if ($Path -eq "~") {
    return $HOME
  }
  if ($Path.StartsWith("~/") -or $Path.StartsWith("~\")) {
    return (Join-Path $HOME $Path.Substring(2))
  }
  if (-not [System.IO.Path]::IsPathRooted($Path)) {
    return (Join-Path $HOME $Path)
  }
  return $Path
}

function Prepare-Worktree([string]$Repo, [string]$Name) {
  $Dirty = git -C $Repo status --short
  if (-not $Dirty) {
    return
  }
  if ($Force -eq "1") {
    return
  }
  if ($FailOnDirty -eq "1") {
    [Console]::Error.WriteLine("[$Label] $Name has local changes:")
    [Console]::Error.WriteLine(($Dirty -join [Environment]::NewLine))
    [Console]::Error.WriteLine("[$Label] rerun without --fail-on-dirty to stash, or with --force to discard tracked VM changes")
    exit 3
  }
  Write-Log "stashing local $Name changes before deploy"
  $Stamp = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
  git -C $Repo stash push --include-untracked -m "iris-drive dev-vm autosave $Stamp"
  if ($LASTEXITCODE -ne 0) { throw "git stash failed for $Name" }
}

function Sync-Repo([string]$Repo, [string]$Name, [string]$Bare, [string]$Branch = $SyncBranch, [string]$CheckoutBranch = $TargetBranch) {
  $Bare = Expand-RemotePath $Bare
  if (-not (Test-Path (Join-Path $Repo ".git"))) {
    if (Test-Path $Repo) {
      throw "sync path exists but is not a git checkout: $Repo"
    }
    Write-Log "creating checkout for $Name at $Repo"
    $Parent = Split-Path -Parent $Repo
    if ($Parent) {
      New-Item -ItemType Directory -Force -Path $Parent | Out-Null
    }
    git clone $Bare $Repo
    if ($LASTEXITCODE -ne 0) { throw "git clone failed for $Name" }
  }
  $HasHead = $true
  try {
    git -C $Repo rev-parse --verify HEAD 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) { $HasHead = $false }
  } catch {
    $HasHead = $false
  }
  if ($HasHead) {
    Prepare-Worktree $Repo $Name
  } else {
    Write-Log "$Name checkout has no HEAD yet; skipping dirty check before first checkout"
  }
  Write-Log "fetching $Name from $Bare"
  git -C $Repo fetch $Bare $Branch
  if ($LASTEXITCODE -ne 0) { throw "git fetch failed for $Name" }
  $Fetched = (git -C $Repo rev-parse FETCH_HEAD).Trim()
  if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed for $Name" }
  try {
    $Current = (git -C $Repo rev-parse --verify HEAD 2>$null)
    if ($LASTEXITCODE -ne 0) { $Current = "" }
  } catch {
    $Current = ""
  }
  $Current = ($Current -as [string]).Trim()
  try {
    $CurrentBranch = (git -C $Repo symbolic-ref --quiet --short HEAD 2>$null)
    if ($LASTEXITCODE -ne 0) { $CurrentBranch = "" }
  } catch {
    $CurrentBranch = ""
  }
  $CurrentBranch = ($CurrentBranch -as [string]).Trim()
  if (($Force -ne "1") -and ($Current -eq $Fetched) -and ($CurrentBranch -eq $CheckoutBranch)) {
    Write-Log "$Name already at $CheckoutBranch@$($Fetched.Substring(0, [Math]::Min(12, $Fetched.Length))); leaving worktree untouched"
    return
  }
  if ($Force -eq "1") {
    git -C $Repo checkout --force -B $CheckoutBranch FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw "git checkout failed for $Name" }
    git -C $Repo reset --hard FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw "git reset failed for $Name" }
  } else {
    git -C $Repo checkout -B $CheckoutBranch FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw "git checkout failed for $Name" }
  }
}

function Build-Idrive([string]$IrisRepo) {
  Write-Log "building idrive helper"
  $env:CARGO_INCREMENTAL = $CargoIncrementalDefault
  $env:CARGO_PROFILE_DEV_DEBUG = $CargoProfileDevDebugDefault
  cargo build -p idrive --locked
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

function Detect-OverlayIp {
  $Nvpn = (Get-Command nvpn -ErrorAction SilentlyContinue).Source
  if (-not $Nvpn) {
    $Candidate = Join-Path $HOME "src\nostr-vpn\target\debug\nvpn.exe"
    if (Test-Path $Candidate) { $Nvpn = $Candidate }
  }
  if (-not $Nvpn) { return "" }
  try {
    $Status = & $Nvpn status --json | ConvertFrom-Json
    return (($Status.tunnel_ip -as [string]) -replace "/.*$", "")
  } catch {
    return ""
  }
}

function Stop-IdriveDaemon([string]$ConfigDir) {
  $Pids = @()
  $StatusFile = Join-Path $ConfigDir "daemon-status.json"
  $LockFile = Join-Path $ConfigDir "daemon.lock"
  if (Test-Path $StatusFile) {
    try {
      $Status = Get-Content -Raw $StatusFile | ConvertFrom-Json
      if ($Status.pid) { $Pids += [int]$Status.pid }
    } catch {}
  }
  if (Test-Path $LockFile) {
    try {
      $LockPid = (Get-Content -Raw $LockFile).Trim()
      if ($LockPid) { $Pids += [int]$LockPid }
    } catch {}
  }
  foreach ($PidValue in ($Pids | Select-Object -Unique)) {
    try {
      $Process = Get-Process -Id $PidValue -ErrorAction Stop
      if ($Process.ProcessName -eq "idrive") {
        Stop-Process -InputObject $Process -Force -ErrorAction Stop
      }
    } catch {}
  }
  Remove-Item -Force -ErrorAction SilentlyContinue $LockFile
}

function Stop-IrisDriveDevProcesses([string]$IrisRepo, [string]$ConfigDir) {
  foreach ($TaskName in @("IrisDriveDevDaemon", "IrisDriveDevLaunch")) {
    try {
      Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
      Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    } catch {}
  }

  $PublishDir = Join-Path $IrisRepo "windows\bin\Debug\net8.0-windows\win-x64\publish"
  $Prefixes = @(
    (Join-Path $IrisRepo ""),
    (Join-Path $PublishDir ""),
    (Join-Path $ConfigDir "")
  ) | ForEach-Object { $_.TrimEnd("\") }

  $Processes = Get-CimInstance Win32_Process |
    Where-Object { $_.Name -in @("IrisDrive.exe", "idrive.exe") }

  foreach ($ProcessInfo in $Processes) {
    $CommandLine = [string]$ProcessInfo.CommandLine
    $ExecutablePath = [string]$ProcessInfo.ExecutablePath
    $MatchesDevTree = $false
    foreach ($Prefix in $Prefixes) {
      if (-not $Prefix) { continue }
      if ($CommandLine.Contains($Prefix) -or $ExecutablePath.Contains($Prefix)) {
        $MatchesDevTree = $true
        break
      }
    }
    if (-not $MatchesDevTree) { continue }
    try {
      Stop-Process -Id $ProcessInfo.ProcessId -Force -ErrorAction Stop
    } catch {}
  }

  for ($i = 0; $i -lt 20; $i++) {
    $StillRunning = Get-CimInstance Win32_Process |
      Where-Object {
        $_.Name -in @("IrisDrive.exe", "idrive.exe") -and
        (([string]$_.CommandLine).Contains($IrisRepo) -or
         ([string]$_.ExecutablePath).Contains($IrisRepo))
      }
    if (-not $StillRunning) { return }
    Start-Sleep -Milliseconds 100
  }
}

function Start-IdriveDaemon([string]$Idrive, [string]$ConfigDir, [string]$PublishDir) {
  New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null
  Stop-IdriveDaemon $ConfigDir
  $DaemonOut = Join-Path $env:TEMP "iris-drive-windows-daemon.out.log"
  $DaemonErr = Join-Path $env:TEMP "iris-drive-windows-daemon.err.log"
  $CloudFilesLog = Join-Path $ConfigDir "windows-cloud-files.log"
  Remove-Item -Force -ErrorAction SilentlyContinue $DaemonOut, $DaemonErr, $CloudFilesLog

  $DaemonScript = Join-Path $PublishDir "launch-idrive-daemon-dev.cmd"
@"
@echo off
set IRIS_DRIVE_FIPS_UDP_BIND_ADDR=$env:IRIS_DRIVE_FIPS_UDP_BIND_ADDR
set IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=$env:IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR
set IRIS_DRIVE_FIPS_UDP_PUBLIC=$env:IRIS_DRIVE_FIPS_UDP_PUBLIC
set IRIS_DRIVE_FIPS_ENABLE_WEBRTC=$env:IRIS_DRIVE_FIPS_ENABLE_WEBRTC
set IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=$env:IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP
set IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=$env:IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING
set IRIS_DRIVE_FIPS_STATIC_PEERS=$env:IRIS_DRIVE_FIPS_STATIC_PEERS
set IRIS_DRIVE_WINDOWS_CLOUD_DEBUG=$env:IRIS_DRIVE_WINDOWS_CLOUD_DEBUG
cd /d "$PublishDir"
"$Idrive" --config-dir "$ConfigDir" daemon --watch-debounce-ms 100 > "$DaemonOut" 2> "$DaemonErr"
"@ | Set-Content -Encoding ASCII $DaemonScript

  $TaskName = "IrisDriveDevDaemon"
  try {
    Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
  } catch {}
  $Action = New-ScheduledTaskAction -Execute "cmd.exe" -Argument "/c `"$DaemonScript`"" -WorkingDirectory $PublishDir
  $Trigger = New-ScheduledTaskTrigger -Once -At ((Get-Date).AddMinutes(1))
  $Principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType S4U -RunLevel Limited
  Register-ScheduledTask -TaskName $TaskName -Action $Action -Trigger $Trigger -Principal $Principal -Force | Out-Null
  Start-ScheduledTask -TaskName $TaskName

  $DirectProcess = $null
  for ($i = 0; $i -lt 40; $i++) {
    try {
      $Status = & $Idrive --config-dir $ConfigDir status | ConvertFrom-Json
      if ($Status.network.fips.enabled -and $Status.network.fips.running) {
        return
      }
    } catch {}
    $Task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($Task -and $Task.State -eq "Ready" -and $i -gt 2 -and -not $DirectProcess) {
      $RunningDaemon = Get-CimInstance Win32_Process |
        Where-Object {
          $_.Name -eq "idrive.exe" -and
          ((([string]$_.CommandLine).Contains($ConfigDir)) -or
           (([string]$_.ExecutablePath) -eq $Idrive))
        } |
        Select-Object -First 1
      if (-not $RunningDaemon) {
        Write-Log "scheduled daemon task did not start idrive; starting in SSH session"
        $DirectProcess = Start-Process -FilePath $Idrive -ArgumentList @("--config-dir", $ConfigDir, "daemon", "--watch-debounce-ms", "100") -WorkingDirectory $PublishDir -RedirectStandardOutput $DaemonOut -RedirectStandardError $DaemonErr -PassThru
      }
    }
    if ($DirectProcess -and $DirectProcess.HasExited) {
      if (Test-Path $DaemonErr) { Get-Content -Tail 120 $DaemonErr | ForEach-Object { [Console]::Error.WriteLine($_) } }
      throw "idrive daemon exited during startup"
    }
    Start-Sleep -Milliseconds 500
  }
  if (Test-Path $DaemonErr) { Get-Content -Tail 120 $DaemonErr | ForEach-Object { [Console]::Error.WriteLine($_) } }
  throw "idrive daemon did not report running FIPS status"
}

$IrisRepo = Join-Path $HOME "src\iris-drive"
$HashtreeRepo = Join-Path $HOME "src\hashtree"
$FipsRepo = Join-Path $HOME "src\fips"
$SocialGraphRepo = Join-Path $HOME "src\nostr-social-graph"
$CashuServiceRepo = Join-Path $HOME "src\cashu-service"
if ([string]::IsNullOrWhiteSpace($WindowsConfigDirOverride)) {
  $ConfigDir = Join-Path $env:APPDATA "iris-drive"
} else {
  $ConfigDir = Expand-RemotePath $WindowsConfigDirOverride
}
Sync-Repo $SocialGraphRepo "nostr-social-graph" $SocialGraphBare $SocialGraphSyncBranch $SocialGraphTargetBranch
Sync-Repo $CashuServiceRepo "cashu-service" $CashuServiceBare
Sync-Repo $HashtreeRepo "hashtree" $HashtreeBare
Sync-Repo $FipsRepo "fips" $FipsBare $FipsSyncBranch $FipsTargetBranch
Sync-Repo $IrisRepo "iris-drive" $IrisBare

Set-Location $IrisRepo
if ($NoRun -eq "1") {
  Write-Log "building Windows dev app"
  Build-Idrive $IrisRepo
  dotnet build .\windows\IrisDrive.Windows.csproj -c Debug -r win-x64 --self-contained true -p:WindowsPackageType=None
  if ($LASTEXITCODE -ne 0) { throw "windows build failed" }
  exit 0
}

Write-Log "publishing Windows dev app"
Stop-IrisDriveDevProcesses $IrisRepo $ConfigDir
Build-Idrive $IrisRepo
$PublishArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ".\scripts\windows-publish.ps1", "-Configuration", "Debug", "-StopRunningApp", "-SkipCliBuild")
powershell @PublishArgs
if ($LASTEXITCODE -ne 0) { throw "windows publish failed" }

$env:IRIS_DRIVE_FIPS_UDP_BIND_ADDR = "0.0.0.0:$FipsPort"
$env:IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR = ""
$env:IRIS_DRIVE_FIPS_UDP_PUBLIC = "false"
$env:IRIS_DRIVE_FIPS_ENABLE_WEBRTC = "true"
$env:IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP = $FipsEnableBootstrapEffective
$env:IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING = $FipsOpenDiscoveryMaxPendingEffective
$env:IRIS_DRIVE_FIPS_STATIC_PEERS = $StaticPeers
$env:IRIS_DRIVE_WINDOWS_CLOUD_DEBUG = "1"

$PublishDir = Join-Path $IrisRepo "windows\bin\Debug\net8.0-windows\win-x64\publish"
$Exe = Join-Path $PublishDir "IrisDrive.exe"
if (-not (Test-Path $Exe)) {
  throw "missing published Windows app: $Exe"
}
$Idrive = Join-Path $PublishDir "idrive.exe"
if (-not (Test-Path $Idrive)) {
  throw "missing published idrive helper: $Idrive"
}
Write-Log "starting Windows idrive daemon"
Start-IdriveDaemon $Idrive $ConfigDir $PublishDir
$env:IRIS_DRIVE_CLI = $Idrive
$env:IRIS_DRIVE_EXTERNAL_DAEMON = "true"
function Test-InteractiveDesktop {
  $Computer = Get-CimInstance Win32_ComputerSystem
  if (-not [string]::IsNullOrWhiteSpace($Computer.UserName)) { return $true }
  return [bool](Get-Process -Name "explorer" -ErrorAction SilentlyContinue)
}

$HasInteractiveDesktop = Test-InteractiveDesktop
if ($HasInteractiveDesktop) {
  Write-Log "starting Windows app"
  $LaunchScript = Join-Path $PublishDir "launch-iris-drive-dev.cmd"
@"
@echo off
set IRIS_DRIVE_CLI=$Idrive
set IRIS_DRIVE_EXTERNAL_DAEMON=true
set IRIS_DRIVE_FIPS_UDP_BIND_ADDR=0.0.0.0:$FipsPort
set IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=
set IRIS_DRIVE_FIPS_UDP_PUBLIC=false
set IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true
set IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=$FipsEnableBootstrapEffective
set IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=$FipsOpenDiscoveryMaxPendingEffective
set IRIS_DRIVE_FIPS_STATIC_PEERS=$StaticPeers
set IRIS_DRIVE_WINDOWS_CLOUD_DEBUG=1
cd /d "$PublishDir"
start "" "$Exe"
"@ | Set-Content -Encoding ASCII $LaunchScript

  $TaskName = "IrisDriveDevLaunch"
  $Action = New-ScheduledTaskAction -Execute "cmd.exe" -Argument "/c `"$LaunchScript`"" -WorkingDirectory $PublishDir
  $Trigger = New-ScheduledTaskTrigger -Once -At ((Get-Date).AddMinutes(1))
  $Principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
  Register-ScheduledTask -TaskName $TaskName -Action $Action -Trigger $Trigger -Principal $Principal -Force | Out-Null
  Start-ScheduledTask -TaskName $TaskName
  Start-Sleep -Seconds 5
  if (-not (Get-Process -Name "IrisDrive" -ErrorAction SilentlyContinue)) {
    Write-Log "interactive scheduled launch did not create an IrisDrive process"
  }
} else {
  Write-Log "no unlocked interactive Windows desktop session; skipping Windows app GUI launch"
}

try {
  $Status = & $Idrive --config-dir $ConfigDir status | ConvertFrom-Json
  $Connected = $Status.network.fips.connected_peers -join ","
  $Peers = @($Status.peers | ForEach-Object { "$($_.label):$($_.fips_online):$($_.sync_state)" }) -join ", "
  Write-Output "connected_peers=[$Connected]"
  Write-Output "peers=[$Peers]"
} catch {
  Write-Log "status read failed after launch: $_"
}
REMOTE_PS
  } | ssh "$host" 'cmd /d /s /c "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""`$script = [Console]::In.ReadToEnd(); & ([scriptblock]::Create(`$script))"""'
}

remote_status_json() {
  local kind="$1"
  local host="$2"

  case "$kind" in
    macos)
      local app_group="${IRIS_DRIVE_DEV_VM_MACOS_APP_GROUP_IDENTIFIER:-}"
      local team="${IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM:-}"
      local app_base="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-}"
      team="${team%.}"
      if [[ -z "$app_group" && -n "$team" ]]; then
        app_group="$team.to.iris.drive"
      fi
      ssh "$host" "IRIS_DRIVE_DEV_VM_MACOS_APP_GROUP_IDENTIFIER=$(sh_quote "$app_group") IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR=$(sh_quote "$app_base") bash -se" <<'REMOTE_SH'
set -Eeuo pipefail
idrive="$HOME/src/iris-drive/target/debug/idrive"
app_group="${IRIS_DRIVE_DEV_VM_MACOS_APP_GROUP_IDENTIFIER:-}"
if [[ -z "$app_group" ]]; then
  app="$HOME/src/iris-drive/macos/.build/Applications/Iris Drive.app"
  app_group="$(codesign -d --entitlements :- "$app" 2>/dev/null \
    | python3 -c 'import plistlib, sys; data=sys.stdin.buffer.read(); plist=plistlib.loads(data) if data.strip() else {}; groups=plist.get("com.apple.security.application-groups") or []; print(groups[0] if groups else "")' 2>/dev/null \
    || true)"
fi
if [[ -z "$app_group" ]]; then
  app_group="group.to.iris.drive"
fi
if [[ -n "${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-}" ]]; then
  config_dir="$IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR/Config"
else
  config_dir="$HOME/Library/Group Containers/$app_group/Iris Drive Dev/Config"
fi
"$idrive" --config-dir "$config_dir" status
REMOTE_SH
      ;;
    linux)
      ssh "$host" "IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR=$(sh_quote "${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-}") IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT=$(sh_quote "${IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT:-}") bash -se" <<'REMOTE_SH'
set -Eeuo pipefail
idrive="$HOME/src/iris-drive/target/debug/idrive"
config_dir="${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-$HOME/.config/iris-drive}"
"$idrive" --config-dir "$config_dir" status
REMOTE_SH
      ;;
    windows)
      {
        printf '$ConfigDirOverride = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_WINDOWS_CONFIG_DIR:-}")"
        cat <<'REMOTE_PS'
$ErrorActionPreference = "Stop"

function Expand-RemotePath([string]$Path) {
  if ($Path -eq "~") {
    return $HOME
  }
  if ($Path.StartsWith("~/") -or $Path.StartsWith("~\")) {
    return (Join-Path $HOME $Path.Substring(2))
  }
  if (-not [System.IO.Path]::IsPathRooted($Path)) {
    return (Join-Path $HOME $Path)
  }
  return $Path
}

$Idrive = Join-Path $HOME "src\iris-drive\windows\bin\Debug\net8.0-windows\win-x64\publish\idrive.exe"
if ([string]::IsNullOrWhiteSpace($ConfigDirOverride)) {
  $ConfigDir = Join-Path $env:APPDATA "iris-drive"
} else {
  $ConfigDir = Expand-RemotePath $ConfigDirOverride
}
& $Idrive --config-dir $ConfigDir status
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
REMOTE_PS
      } | ssh "$host" 'cmd /d /s /c "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""`$script = [Console]::In.ReadToEnd(); & ([scriptblock]::Create(`$script))"""'
      ;;
    *)
      return 1
      ;;
  esac
}

remote_status_json_retry() {
  local kind="$1"
  local host="$2"
  local attempts="${3:-5}"
  local delay="${4:-0.5}"
  local attempt
  local output=""

  for ((attempt = 1; attempt <= attempts; attempt++)); do
    output="$(remote_status_json "$kind" "$host" 2>/dev/null || true)"
    if [[ -n "$output" ]] \
      && STATUS_JSON="$output" python3 - <<'PY' >/dev/null 2>&1
import json
import os

json.loads(os.environ["STATUS_JSON"])
PY
    then
      printf '%s\n' "$output"
      return 0
    fi
    sleep "$delay"
  done

  return 1
}

remote_transport_diagnostics() {
  local kind="$1"
  local host="$2"

  case "$kind" in
    macos|linux)
      ssh "$host" 'bash -se' <<'REMOTE_SH'
set +e

check_https() {
  local label="$1"
  local family="$2"
  if ! command -v curl >/dev/null 2>&1; then
    printf '%s=unknown(no curl)\n' "$label"
    return
  fi
  if curl "$family" -I --max-time 8 https://example.com >/dev/null 2>&1; then
    printf '%s=ok\n' "$label"
  else
    printf '%s=fail\n' "$label"
  fi
}

check_tcp() {
  local label="$1"
  local host="$2"
  local port="$3"
  if ! command -v nc >/dev/null 2>&1; then
    printf '%s=unknown(no nc)\n' "$label"
    return
  fi
  if nc -vz -G 5 "$host" "$port" >/dev/null 2>&1 \
    || nc -vz -w 5 "$host" "$port" >/dev/null 2>&1; then
    printf '%s=ok\n' "$label"
  else
    printf '%s=fail\n' "$label"
  fi
}

check_https ipv4_https -4
check_https ipv6_https -6
check_tcp fips_bootstrap_tcp_54_183_70_180_443 54.183.70.180 443
REMOTE_SH
      ;;
    windows)
      ssh "$host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' <<'REMOTE_PS'
$ProgressPreference = "SilentlyContinue"
function Check-Https {
  try {
    Invoke-WebRequest -UseBasicParsing -TimeoutSec 8 -Uri "https://example.com" | Out-Null
    "https=ok"
  } catch {
    "https=fail"
  }
}
function Check-Tcp {
  param([string]$Label, [string]$HostName, [int]$Port)
  $client = New-Object System.Net.Sockets.TcpClient
  try {
    $result = $client.BeginConnect($HostName, $Port, $null, $null)
    if ($result.AsyncWaitHandle.WaitOne(5000)) {
      $client.EndConnect($result)
      "$Label=ok"
    } else {
      "$Label=fail"
    }
  } catch {
    "$Label=fail"
  } finally {
    $client.Close()
  }
}
Check-Https
Check-Tcp "fips_bootstrap_tcp_54_183_70_180_443" "54.183.70.180" 443
REMOTE_PS
      ;;
    *)
      return 1
      ;;
  esac
}

print_host_macos_vm_nat_diagnostics() {
  local has_macos=0
  local i

  [[ "$(uname -s)" == "Darwin" ]] || return 0
  for i in "${!LABELS[@]}"; do
    if [[ "${KINDS[$i]}" == "macos" ]]; then
      has_macos=1
      break
    fi
  done
  [[ "$has_macos" == "1" ]] || return 0
  if ! ifconfig bridge100 >/dev/null 2>&1; then
    return 0
  fi

  local bridge_ipv4
  local forwarding
  local default_iface
  bridge_ipv4="$(ifconfig bridge100 2>/dev/null | awk '/inet / { print $2; exit }')"
  forwarding="$(sysctl -n net.inet.ip.forwarding 2>/dev/null || true)"
  default_iface="$(route -n get default 2>/dev/null | awk '/interface:/ { print $2; exit }')"

  printf '[dev-vms] host macOS VM NAT diagnostics:\n' >&2
  printf '[dev-vms]   bridge100_ipv4=%s\n' "${bridge_ipv4:-unknown}" >&2
  printf '[dev-vms]   host_default_iface=%s\n' "${default_iface:-unknown}" >&2
  printf '[dev-vms]   net.inet.ip.forwarding=%s\n' "${forwarding:-unknown}" >&2
  if [[ "$forwarding" != "1" ]]; then
    printf '[dev-vms]   warning=host IPv4 forwarding is disabled; macOS VM IPv4-only FIPS bootstrap peers will be unreachable\n' >&2
  fi
}

print_connectivity_diagnostics() {
  local i
  local status=""

  print_host_macos_vm_nat_diagnostics

  for i in "${!LABELS[@]}"; do
    printf '[dev-vms] %s diagnostics:\n' "${LABELS[$i]}" >&2
    if status="$(remote_status_json_retry "${KINDS[$i]}" "${SSH_HOSTS[$i]}" 3 0.5)"; then
      STATUS_JSON="$status" python3 <<'PY' | sed 's/^/[dev-vms]   /' >&2
import json
import os

data = json.loads(os.environ["STATUS_JSON"])
network = data.get("network") or {}
fips = network.get("fips") or {}
relays = network.get("relay_statuses") or []
fips_relays = fips.get("relay_statuses") or []
peers = data.get("peers") or []

relay_summary = ", ".join(
    f"{relay.get('url')}:{relay.get('status')}" for relay in relays
)
fips_relay_summary = ", ".join(
    f"{relay.get('url')}:{relay.get('status')}" for relay in fips_relays
)
peer_summary = ", ".join(
    f"{peer.get('label')}:{peer.get('fips_online')}:{peer.get('sync_state')}"
    for peer in peers
)

print(f"nostr_discovery_app={fips.get('nostr_discovery_app')}")
print(f"connected_peers={fips.get('connected_peers') or []}")
print(f"mesh_peers={fips.get('mesh_peers') or []}")
print(f"relay_statuses={relay_summary}")
print(f"fips_relay_statuses={fips_relay_summary}")
print(f"peers={peer_summary}")
PY
    else
      printf '[dev-vms]   status=unavailable\n' >&2
    fi

    remote_transport_diagnostics "${KINDS[$i]}" "${SSH_HOSTS[$i]}" \
      | sed 's/^/[dev-vms]   /' >&2 || true
  done
}

status_missing_peers() {
  local status="$1"
  shift
  STATUS_JSON="$status" python3 - "$@" <<'PY'
import json
import os
import sys

wanted = sys.argv[1:]
try:
    data = json.loads(os.environ["STATUS_JSON"])
except Exception as exc:
    print(f"invalid status json: {exc}")
    raise SystemExit(1)
peers = {}
for peer in data.get("peers", []):
    for key in ("label", "display_label", "app_key_npub", "app_key_pubkey"):
        value = peer.get(key)
        if value:
            peers[value] = peer
fips = (data.get("network") or {}).get("fips") or {}
online_fips = set(
    (fips.get("online_peers") or [])
    + (fips.get("online_devices") or [])
    + (fips.get("direct_peers") or [])
    + (fips.get("direct_devices") or [])
    + (fips.get("connected_peers") or [])
    + (fips.get("mesh_peers") or [])
    + (fips.get("mesh_devices") or [])
)
missing = []
for label in wanted:
    peer = peers.get(label)
    if peer is None and label in online_fips:
        continue
    if peer is None:
        missing.append(f"{label}:missing")
    elif peer.get("fips_online") is not True:
        missing.append(
            f"{label}:online={peer.get('fips_online')} state={peer.get('sync_state')}"
        )

if missing:
    print("; ".join(missing))
    raise SystemExit(1)

connected = fips.get("connected_peers") or []
print("connected_peers=[" + ",".join(connected) + "]")
PY
}

status_current_app_key_npub() {
  local status="$1"
  STATUS_JSON="$status" python3 <<'PY'
import json
import os

data = json.loads(os.environ["STATUS_JSON"])
npub = data.get("current_app_key_npub")
if not npub:
    npub = ((data.get("profile") or {}).get("profile") or {}).get("current_app_key_npub")
if not npub:
    npub = (data.get("profile") or {}).get("current_app_key_npub")
if npub:
    print(npub)
PY
}

check_dev_vm_connectivity() {
  local timeout="${IRIS_DRIVE_DEV_VM_CONNECTIVITY_TIMEOUT:-60}"
  local start
  local now
  local i
  local j
  local expected=()
  local status=""
  local summary=""
  local failures=()
  local statuses=()
  local npubs=()

  [[ "$NO_RUN" == "1" ]] && return 0
  [[ "${IRIS_DRIVE_DEV_VM_SKIP_CONNECTIVITY_CHECK:-0}" == "1" ]] && return 0
  if [[ ${#LABELS[@]} -lt 2 ]]; then
    return 0
  fi

  log "waiting for selected VMs to see each other online over FIPS"
  start="$(date +%s)"
  while true; do
    failures=()
    statuses=()
    npubs=()
    for i in "${!LABELS[@]}"; do
      if ! status="$(remote_status_json_retry "${KINDS[$i]}" "${SSH_HOSTS[$i]}" 3 0.5)"; then
        failures+=("${LABELS[$i]}: status unavailable")
        statuses[$i]=""
        npubs[$i]=""
        continue
      fi
      statuses[$i]="$status"
      npubs[$i]="$(status_current_app_key_npub "$status")"
      if [[ -z "${npubs[$i]}" ]]; then
        failures+=("${LABELS[$i]}: current app-key npub unavailable")
      fi
    done

    for i in "${!LABELS[@]}"; do
      [[ -n "${statuses[$i]:-}" ]] || continue
      expected=()
      for j in "${!LABELS[@]}"; do
        [[ "$i" == "$j" ]] && continue
        if [[ -n "${npubs[$j]:-}" ]]; then
          expected+=("${npubs[$j]}")
        fi
      done

      if ! summary="$(status_missing_peers "${statuses[$i]}" "${expected[@]}" 2>&1)"; then
        failures+=("${LABELS[$i]}: $summary")
      else
        log "${LABELS[$i]} FIPS online: ${summary}"
      fi
    done

    if [[ ${#failures[@]} -eq 0 ]]; then
      log "all selected VMs report each other online over FIPS"
      return 0
    fi

    now="$(date +%s)"
    if (( now - start >= timeout )); then
      printf '[dev-vms] FIPS connectivity check failed after %ss:\n' "$timeout" >&2
      printf '[dev-vms]   %s\n' "${failures[@]}" >&2
      print_connectivity_diagnostics
      return 5
    fi

    sleep 5
  done
}

for i in "${!LABELS[@]}"; do
  log "updating/building/running ${LABELS[$i]} on ${HOSTS[$i]} via ${SSH_HOSTS[$i]}"
  case "${KINDS[$i]}" in
    macos|linux)
      run_posix_target "${LABELS[$i]}" "${KINDS[$i]}" "${SSH_HOSTS[$i]}" "${IRIS_BARES[$i]}" "${HASHTREE_BARES[$i]}" "${FIPS_BARES[$i]}" "${SOCIAL_GRAPH_BARES[$i]}" "${CASHU_SERVICE_BARES[$i]}" "${STATIC_PEERS_BY_INDEX[$i]:-}" "${STATIC_PEERS_COMPLETE_BY_INDEX[$i]:-0}"
      ;;
    windows)
      run_windows_target "${LABELS[$i]}" "${SSH_HOSTS[$i]}" "${IRIS_BARES[$i]}" "${HASHTREE_BARES[$i]}" "${FIPS_BARES[$i]}" "${SOCIAL_GRAPH_BARES[$i]}" "${CASHU_SERVICE_BARES[$i]}" "${STATIC_PEERS_BY_INDEX[$i]:-}" "${STATIC_PEERS_COMPLETE_BY_INDEX[$i]:-0}"
      ;;
    *)
      die "unknown target kind: ${KINDS[$i]}"
      ;;
  esac
done

check_dev_vm_connectivity
log "done"
