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
SYNC_BRANCH="${IRIS_DRIVE_DEV_VM_SYNC_BRANCH:-codex/dev-vm-sync}"
FIPS_SYNC_BRANCH="${IRIS_DRIVE_DEV_VM_FIPS_SYNC_BRANCH:-$SYNC_BRANCH}"
TARGET_BRANCH="${IRIS_DRIVE_DEV_VM_TARGET_BRANCH:-$(git -C "$ROOT" branch --show-current || true)}"
TARGET_BRANCH="${TARGET_BRANCH:-master}"
FIPS_TARGET_BRANCH="${IRIS_DRIVE_DEV_VM_FIPS_TARGET_BRANCH:-$FIPS_SYNC_BRANCH}"
FORCE=0
FAIL_ON_DIRTY=0
SKIP_PUSH=0
NO_RUN=0
LIST_TARGETS=0
ONLY_LABELS=()

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
  IRIS_DRIVE_DEV_VM_REQUIRE_CLEAN=1   Refuse to run when local repos are dirty.
  IRIS_DRIVE_DEV_VM_MIN_FREE_KB       Prune VM incremental build caches below
                                      this free space.
  IRIS_DRIVE_DEV_VM_PRUNE_COMPILED_CACHE=1
                                      Also prune compiled Cargo deps/build
                                      artifacts when below MIN_FREE_KB.
  IRIS_DRIVE_DEV_VM_SKIP_CONNECTIVITY_CHECK=1
                                      Skip the final all-VM FIPS online check.
  IRIS_DRIVE_DEV_VM_CONNECTIVITY_TIMEOUT
                                      Seconds to wait for all selected peers to
                                      report fips_online=true (default: 60).
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

Remote worktree paths are expected to be:
  ~/src/iris-drive
  ~/src/hashtree
  ~/src/fips

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
      } | ssh "$host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -'
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
declare -a IRIS_REMOTES=()
declare -a HASHTREE_REMOTES=()
declare -a FIPS_REMOTES=()
declare -a IRIS_BARES=()
declare -a HASHTREE_BARES=()
declare -a FIPS_BARES=()
declare -a OVERLAY_IPS=()
declare -a STATIC_PEERS_BY_INDEX=()

add_target_from_remotes() {
  local label="$1"
  local kind="$2"
  local iris_remote="$3"
  local hashtree_remote="$4"
  local fips_remote="$5"
  local iris_parts
  local hashtree_parts
  local fips_parts
  local host
  local hashtree_host
  local fips_host
  local iris_bare
  local hashtree_bare
  local fips_bare

  contains_label "$label" || return 0

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

  LABELS+=("$label")
  KINDS+=("$kind")
  HOSTS+=("$host")
  IRIS_REMOTES+=("$iris_remote")
  HASHTREE_REMOTES+=("$hashtree_remote")
  FIPS_REMOTES+=("$fips_remote")
  IRIS_BARES+=("$iris_bare")
  HASHTREE_BARES+=("$hashtree_bare")
  FIPS_BARES+=("$fips_bare")
}

warn_or_fail_local_dirty "$ROOT" "iris-drive"
warn_or_fail_local_dirty "$HASHTREE_ROOT" "hashtree"
warn_or_fail_local_dirty "$FIPS_ROOT" "fips"

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

add_target_from_remotes \
  macos \
  macos \
  "$MACOS_IRIS_REMOTE" \
  "$MACOS_HASHTREE_REMOTE" \
  "$MACOS_FIPS_REMOTE"
add_target_from_remotes \
  ubuntu \
  linux \
  "$UBUNTU_IRIS_REMOTE" \
  "$UBUNTU_HASHTREE_REMOTE" \
  "$UBUNTU_FIPS_REMOTE"
add_target_from_remotes \
  windows \
  windows \
  "$WINDOWS_IRIS_REMOTE" \
  "$WINDOWS_HASHTREE_REMOTE" \
  "$WINDOWS_FIPS_REMOTE"

if [[ ${#LABELS[@]} -eq 0 ]]; then
  usage >&2
  die "no VM targets configured"
fi

if [[ "$LIST_TARGETS" == "1" ]]; then
  for i in "${!LABELS[@]}"; do
    printf '%s\t%s\t%s\tiris=%s\thashtree=%s\tfips=%s\n' \
      "${LABELS[$i]}" \
      "${KINDS[$i]}" \
      "${HOSTS[$i]}" \
      "${IRIS_BARES[$i]}" \
      "${HASHTREE_BARES[$i]}" \
      "${FIPS_BARES[$i]}"
  done
  exit 0
fi

detect_remote_overlay_ip() {
  local kind="$1"
  local host="$2"
  local ip=""
  if [[ "$kind" == "windows" ]]; then
    ip="$(ssh "$host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' <<'REMOTE_PS' 2>/dev/null || true
$ErrorActionPreference = "SilentlyContinue"
$Nvpn = (Get-Command nvpn -ErrorAction SilentlyContinue).Source
if (-not $Nvpn) {
  $Candidate = Join-Path $HOME "src\nostr-vpn\target\debug\nvpn.exe"
  if (Test-Path $Candidate) { $Nvpn = $Candidate }
}
if ($Nvpn) {
  try {
    $Status = & $Nvpn status --json | ConvertFrom-Json
    $Running = $true
    if ($Status.daemon -and $null -ne $Status.daemon.running) {
      $Running = [bool]$Status.daemon.running
    }
    if ($Running -and $Status.tunnel_ip) {
      (($Status.tunnel_ip -as [string]) -replace "/.*$", "")
    }
  } catch {}
}
REMOTE_PS
)"
  else
    ip="$(ssh "$host" 'bash -se' <<'REMOTE_SH' 2>/dev/null || true
set -Eeuo pipefail
nvpn=""
for candidate in \
  "$(command -v nvpn 2>/dev/null || true)" \
  "$HOME/src/nostr-vpn/target/debug/nvpn" \
  "$HOME/src/nostr-vpn/target/aarch64-apple-darwin/debug/nvpn" \
  "/Library/PrivilegedHelperTools/to.nostrvpn.nvpn"
do
  [[ -n "$candidate" && -x "$candidate" ]] || continue
  if "$candidate" status --json 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); daemon=d.get("daemon") or {}; running=daemon.get("running"); source=d.get("status_source"); ip=(d.get("tunnel_ip") or "").split("/")[0]; invalid=not ip or (running is False and source != "daemon"); print(ip) if not invalid else None; sys.exit(1 if invalid else 0)'; then
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
    ssh "$host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' <<REMOTE_PS >/dev/null 2>&1
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
  ssh "$host" 'bash -se' <<REMOTE_SH >/dev/null 2>&1
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
      done
      log "not using nvpn static FIPS peer hints; disabled by IRIS_DRIVE_DEV_VM_USE_NVPN_STATIC_HINTS=$mode"
      return 0
      ;;
    *)
      die "unknown IRIS_DRIVE_DEV_VM_USE_NVPN_STATIC_HINTS value: $mode"
      ;;
  esac

  for i in "${!LABELS[@]}"; do
    ip="$(detect_remote_overlay_ip "${KINDS[$i]}" "${HOSTS[$i]}" || true)"
    if [[ -z "$ip" ]]; then
      log "warning: could not detect nvpn IP for ${LABELS[$i]} on ${HOSTS[$i]}; native FIPS may need WebRTC or relay fallback"
      continue
    fi
    OVERLAY_IPS[$i]="$ip"
  done

  for i in "${!LABELS[@]}"; do
    pieces=()
    for j in "${!LABELS[@]}"; do
      [[ "$i" == "$j" ]] && continue
      ip="${OVERLAY_IPS[$j]:-}"
      [[ -n "$ip" ]] || continue
      if [[ "$mode" == "auto" ]] && ! can_target_reach_overlay_ip "${KINDS[$i]}" "${HOSTS[$i]}" "$ip"; then
        log "not using nvpn static FIPS hint ${LABELS[$i]} -> ${LABELS[$j]} ($ip); overlay address is not reachable from ${LABELS[$i]}"
        continue
      fi
      key="$(target_peer_hint_key "${HOSTS[$j]}")"
      pieces+=("$key=$ip:$fips_port")
    done

    if [[ ${#pieces[@]} -gt 0 ]]; then
      local IFS=,
      STATIC_PEERS_BY_INDEX[$i]="${pieces[*]}"
      log "using static FIPS peer hints for ${LABELS[$i]} over nvpn: ${STATIC_PEERS_BY_INDEX[$i]}"
    else
      STATIC_PEERS_BY_INDEX[$i]=""
      log "not using nvpn static FIPS peer hints for ${LABELS[$i]}; no reachable overlay peers"
    fi
  done
}

build_static_peer_hints

if [[ "$SKIP_PUSH" != "1" ]]; then
  for i in "${!LABELS[@]}"; do
    log "ensuring VM bare git repos exist for ${LABELS[$i]}"
    ensure_remote_bare_repo "${KINDS[$i]}" "${HOSTS[$i]}" "${IRIS_BARES[$i]}"
    ensure_remote_bare_repo "${KINDS[$i]}" "${HOSTS[$i]}" "${HASHTREE_BARES[$i]}"
    ensure_remote_bare_repo "${KINDS[$i]}" "${HOSTS[$i]}" "${FIPS_BARES[$i]}"

    log "pushing iris-drive HEAD to ${LABELS[$i]} (${IRIS_REMOTES[$i]}:$SYNC_BRANCH)"
    git -C "$ROOT" push "${IRIS_REMOTES[$i]}" "+HEAD:refs/heads/$SYNC_BRANCH"
    log "pushing hashtree HEAD to ${LABELS[$i]} (${HASHTREE_REMOTES[$i]}:$SYNC_BRANCH)"
    git -C "$HASHTREE_ROOT" push "${HASHTREE_REMOTES[$i]}" "+HEAD:refs/heads/$SYNC_BRANCH"
    log "pushing fips HEAD to ${LABELS[$i]} (${FIPS_REMOTES[$i]}:$FIPS_SYNC_BRANCH)"
    git -C "$FIPS_ROOT" push "${FIPS_REMOTES[$i]}" "+HEAD:refs/heads/$FIPS_SYNC_BRANCH"
  done
fi

run_posix_target() {
  local label="$1"
  local kind="$2"
  local host="$3"
  local iris_bare="$4"
  local hashtree_bare="$5"
  local fips_bare="$6"
  local static_peers="$7"

  {
    printf 'LABEL=%s\n' "$(sh_quote "$label")"
    printf 'KIND=%s\n' "$(sh_quote "$kind")"
    printf 'IRIS_BARE=%s\n' "$(sh_quote "$iris_bare")"
    printf 'HASHTREE_BARE=%s\n' "$(sh_quote "$hashtree_bare")"
    printf 'FIPS_BARE=%s\n' "$(sh_quote "$fips_bare")"
    printf 'SYNC_BRANCH=%s\n' "$(sh_quote "$SYNC_BRANCH")"
    printf 'FIPS_SYNC_BRANCH=%s\n' "$(sh_quote "$FIPS_SYNC_BRANCH")"
    printf 'TARGET_BRANCH=%s\n' "$(sh_quote "$TARGET_BRANCH")"
    printf 'FIPS_TARGET_BRANCH=%s\n' "$(sh_quote "$FIPS_TARGET_BRANCH")"
    printf 'FORCE=%s\n' "$(sh_quote "$FORCE")"
    printf 'FAIL_ON_DIRTY=%s\n' "$(sh_quote "$FAIL_ON_DIRTY")"
    printf 'NO_RUN=%s\n' "$(sh_quote "$NO_RUN")"
    printf 'FIPS_PORT=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_FIPS_PORT:-22121}")"
    printf 'STATIC_PEERS=%s\n' "$(sh_quote "$static_peers")"
    printf 'MIN_FREE_KB=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MIN_FREE_KB:-6291456}")"
    printf 'PRUNE_COMPILED_CACHE=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_PRUNE_COMPILED_CACHE:-0}")"
    printf 'IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER:-0}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_WRITE_APP_GROUP_RUNTIME=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_WRITE_APP_GROUP_RUNTIME:-}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_DEVELOPMENT_TEAM:-}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_SIGN_IDENTITY=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_SIGN_IDENTITY:-}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN:-}")"
    printf 'IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN_PASS_FILE=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_MACOS_KEYCHAIN_PASS_FILE:-}")"
    cat <<'REMOTE_SH'
set -Eeuo pipefail

MACOS_CODESIGN_KEYCHAIN=""
MACOS_XCODE_SIGNED_IDENTITY=""

log() {
  printf '[%s] %s\n' "$LABEL" "$*" >&2
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

  bare="$(expand_path "$bare")"
  [[ -d "$repo/.git" ]] || { log "missing checkout: $repo"; exit 1; }
  prepare_worktree "$repo" "$name"
  log "fetching $name from $bare"
  git -C "$repo" fetch "$bare" "$SYNC_BRANCH"
  local fetched
  local current
  local current_branch
  fetched="$(git -C "$repo" rev-parse FETCH_HEAD)"
  current="$(git -C "$repo" rev-parse HEAD 2>/dev/null || true)"
  current_branch="$(git -C "$repo" symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
  if [[ "$FORCE" != "1" && "$current" == "$fetched" && "$current_branch" == "$TARGET_BRANCH" ]]; then
    log "$name already at $TARGET_BRANCH@${fetched:0:12}; leaving worktree untouched"
    return 0
  fi
  if [[ "$FORCE" == "1" ]]; then
    git -C "$repo" checkout --force -B "$TARGET_BRANCH" FETCH_HEAD
    git -C "$repo" reset --hard FETCH_HEAD
  else
    git -C "$repo" checkout -B "$TARGET_BRANCH" FETCH_HEAD
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

build_idrive() {
  local iris_repo="$1"
  local phase="$2"

  ensure_build_space "$iris_repo" "$phase"
  log "building idrive helper"
  (cd "$iris_repo" && cargo build -p idrive)
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
  local target=""
  [[ -n "$mountpoint" ]] || return 0
  [[ -e "$mountpoint" ]] || return 0

  if command -v findmnt >/dev/null 2>&1; then
    target="$(findmnt -rn --target "$mountpoint" --output TARGET 2>/dev/null | head -n 1 || true)"
    [[ "$target" == "$mountpoint" ]] || return 0
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
      && STATUS_JSON="$output" python3 - <<'PY' >/dev/null 2>&1
import json
import os

json.loads(os.environ["STATUS_JSON"])
PY
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
  STATUS_JSON="$status_json" python3 - <<'PY'
import json
import os
import sys

data = json.loads(os.environ["STATUS_JSON"])
fips = (data.get("network") or {}).get("fips") or {}
sys.exit(0 if fips.get("enabled") and fips.get("running") else 1)
PY
}

print_idrive_status_summary() {
  local status_json="$1"
  STATUS_JSON="$status_json" python3 - <<'PY'
import json
import os

data = json.loads(os.environ["STATUS_JSON"])
fips = (data.get("network") or {}).get("fips") or {}
print("connected_peers=", fips.get("connected_peers"))
print(
    "peers=",
    [
        (peer.get("label"), peer.get("fips_online"), peer.get("sync_state"))
        for peer in data.get("peers", [])
    ],
)
PY
}

idrive_provider_list_retry() {
  local idrive="$1"
  local config_dir="$2"
  local output_file="$3"
  local attempts="${4:-8}"
  local delay="${5:-0.5}"
  local attempt

  for ((attempt = 1; attempt <= attempts; attempt++)); do
    if "$idrive" --config-dir "$config_dir" provider list >"$output_file" 2>&1 \
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
  return 1
}

run_linux() {
  local iris_repo="$HOME/src/iris-drive"
  local idrive="$iris_repo/target/debug/idrive"
  local config_dir="${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-$HOME/.config/iris-drive}"
  local mountpoint="${IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT:-$HOME/Iris Drive}"

  build_idrive "$iris_repo" "Linux build"
  [[ "$NO_RUN" == "1" ]] && return 0

  log "restarting idrive daemon"
  mkdir -p "$config_dir" "$mountpoint"
  stop_idrive_daemon "$config_dir" "$mountpoint"
  rm -f "$config_dir/daemon.lock"
  nohup env \
    "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=0.0.0.0:$FIPS_PORT" \
    "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=" \
    "IRIS_DRIVE_FIPS_UDP_PUBLIC=false" \
    "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true" \
    "IRIS_DRIVE_FIPS_STATIC_PEERS=$STATIC_PEERS" \
    "$idrive" --config-dir "$config_dir" daemon \
      --watch-interval 2 \
      --watch-debounce-ms 100 \
      --no-gateway \
      --mount \
      --mountpoint "$mountpoint" \
      > /tmp/iris-drive-daemon.log 2>&1 < /dev/null &
  local daemon_pid="$!"
  local daemon_ready=0
  local status_json=""
  for _ in {1..50}; do
    if ! process_running "$daemon_pid"; then
      tail -120 /tmp/iris-drive-daemon.log >&2 || true
      die "idrive daemon exited during startup"
    fi
    if status_json="$(idrive_status_json_retry "$idrive" "$config_dir" 1 0.1)" \
      && idrive_status_fips_running "$status_json" >/dev/null; then
      daemon_ready=1
      break
    fi
    sleep 0.2
  done
  if [[ "$daemon_ready" != "1" ]]; then
    tail -120 /tmp/iris-drive-daemon.log >&2 || true
    die "idrive daemon did not report running FIPS"
  fi
  if [[ -z "$status_json" ]]; then
    status_json="$(idrive_status_json_retry "$idrive" "$config_dir" 8 0.5)"
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
  local iris_repo="$1"
  local args=(
    xcodebuild
    -project macos/IrisDriveMac.xcodeproj
    -scheme IrisDriveMac
    -configuration Debug
    -derivedDataPath macos/.build/DerivedData
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
  local app="${IRIS_DRIVE_DEV_VM_MACOS_APP_PATH:-$HOME/Applications/Iris Drive.app}"
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
    --env "IRIS_DRIVE_FIPS_STATIC_PEERS=$STATIC_PEERS"
    --env "IRIS_DRIVE_APP_BASE_DIR=$app_base"
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
    "IRIS_DRIVE_FIPS_STATIC_PEERS=$STATIC_PEERS" \
    "$idrive" --config-dir "$config_dir" daemon \
      --watch-interval 2 \
      --watch-debounce-ms 100 \
      > "$daemon_log" 2>&1 < /dev/null &
  daemon_pid="$!"
  local status_json=""
  for _ in {1..40}; do
    if ! process_running "$daemon_pid"; then
      log "macOS idrive daemon exited during startup"
      tail -n 120 "$daemon_log" >&2 2>/dev/null || true
      exit 4
    fi
    if status_json="$(idrive_status_json_retry "$idrive" "$config_dir" 1 0.1)" \
      && idrive_status_fips_running "$status_json" >/dev/null; then
      break
    fi
    sleep 0.5
  done
  if [[ -z "$status_json" ]]; then
    status_json="$(idrive_status_json_retry "$idrive" "$config_dir" 8 0.5)" || true
  fi
  if [[ -z "$status_json" ]] || ! idrive_status_fips_running "$status_json" >/dev/null; then
    log "macOS idrive daemon did not report running FIPS status"
    tail -n 160 "$daemon_log" >&2 2>/dev/null || true
    exit 4
  fi
  if ! idrive_provider_list_retry "$idrive" "$config_dir" /tmp/iris-drive-macos-provider-list.json 8 0.5; then
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

  if macos_fileprovider_required; then
    [[ -f "$xcode_app_entitlements" ]] && app_entitlements="$xcode_app_entitlements"
    [[ -f "$xcode_appex_entitlements" ]] && appex_entitlements="$xcode_appex_entitlements"
  fi

  if [[ -z "$sign_identity" && -n "$MACOS_XCODE_SIGNED_IDENTITY" ]]; then
    sign_identity="$MACOS_XCODE_SIGNED_IDENTITY"
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
  rm -f "$app_dev_entitlements" "$appex_dev_entitlements"
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
sync_repo "$HOME/src/hashtree" hashtree "$HASHTREE_BARE"
SYNC_BRANCH="$FIPS_SYNC_BRANCH" TARGET_BRANCH="$FIPS_TARGET_BRANCH" sync_repo "$HOME/src/fips" fips "$FIPS_BARE"
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
  local static_peers="$6"

  {
    printf '$Label = %s\n' "$(ps_quote "$label")"
    printf '$IrisBare = %s\n' "$(ps_quote "$iris_bare")"
    printf '$HashtreeBare = %s\n' "$(ps_quote "$hashtree_bare")"
    printf '$FipsBare = %s\n' "$(ps_quote "$fips_bare")"
    printf '$SyncBranch = %s\n' "$(ps_quote "$SYNC_BRANCH")"
    printf '$FipsSyncBranch = %s\n' "$(ps_quote "$FIPS_SYNC_BRANCH")"
    printf '$TargetBranch = %s\n' "$(ps_quote "$TARGET_BRANCH")"
    printf '$FipsTargetBranch = %s\n' "$(ps_quote "$FIPS_TARGET_BRANCH")"
    printf '$Force = %s\n' "$(ps_quote "$FORCE")"
    printf '$FailOnDirty = %s\n' "$(ps_quote "$FAIL_ON_DIRTY")"
    printf '$NoRun = %s\n' "$(ps_quote "$NO_RUN")"
    printf '$FipsPort = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_FIPS_PORT:-22121}")"
    printf '$StaticPeers = %s\n' "$(ps_quote "$static_peers")"
    cat <<'REMOTE_PS'
$ErrorActionPreference = "Stop"

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
    throw "missing checkout: $Repo"
  }
  Prepare-Worktree $Repo $Name
  Write-Log "fetching $Name from $Bare"
  git -C $Repo fetch $Bare $Branch
  if ($LASTEXITCODE -ne 0) { throw "git fetch failed for $Name" }
  $Fetched = (git -C $Repo rev-parse FETCH_HEAD).Trim()
  if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed for $Name" }
  $Current = (git -C $Repo rev-parse HEAD 2>$null)
  if ($LASTEXITCODE -ne 0) { $Current = "" }
  $Current = ($Current -as [string]).Trim()
  $CurrentBranch = (git -C $Repo symbolic-ref --quiet --short HEAD 2>$null)
  if ($LASTEXITCODE -ne 0) { $CurrentBranch = "" }
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
  Remove-Item -Force -ErrorAction SilentlyContinue $DaemonOut, $DaemonErr
  $Args = @(
    "--config-dir", $ConfigDir,
    "daemon",
    "--watch-interval", "2",
    "--watch-debounce-ms", "100",
    "--no-gateway"
  )
  $Process = Start-Process `
    -FilePath $Idrive `
    -ArgumentList $Args `
    -WorkingDirectory $PublishDir `
    -RedirectStandardOutput $DaemonOut `
    -RedirectStandardError $DaemonErr `
    -WindowStyle Hidden `
    -PassThru
  for ($i = 0; $i -lt 40; $i++) {
    if ($Process.HasExited) {
      if (Test-Path $DaemonErr) { Get-Content -Tail 120 $DaemonErr | ForEach-Object { [Console]::Error.WriteLine($_) } }
      throw "idrive daemon exited during startup"
    }
    try {
      $Status = & $Idrive --config-dir $ConfigDir status | ConvertFrom-Json
      if ($Status.network.fips.enabled -and $Status.network.fips.running) {
        return $Process
      }
    } catch {}
    Start-Sleep -Milliseconds 500
  }
  if (Test-Path $DaemonErr) { Get-Content -Tail 120 $DaemonErr | ForEach-Object { [Console]::Error.WriteLine($_) } }
  throw "idrive daemon did not report running FIPS status"
}

$IrisRepo = Join-Path $HOME "src\iris-drive"
$HashtreeRepo = Join-Path $HOME "src\hashtree"
$FipsRepo = Join-Path $HOME "src\fips"
$ConfigDir = Join-Path $env:APPDATA "iris-drive"
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
$DaemonProcess = Start-IdriveDaemon $Idrive $ConfigDir $PublishDir
$env:IRIS_DRIVE_CLI = $Idrive
$env:IRIS_DRIVE_EXTERNAL_DAEMON = "true"
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
set IRIS_DRIVE_FIPS_STATIC_PEERS=$StaticPeers
set IRIS_DRIVE_WINDOWS_CLOUD_DEBUG=1
cd /d "$PublishDir"
start "" "$Exe"
"@ | Set-Content -Encoding ASCII $LaunchScript

$TaskName = "IrisDriveDevLaunch"
try {
  $Action = New-ScheduledTaskAction -Execute "cmd.exe" -Argument "/c `"$LaunchScript`"" -WorkingDirectory $PublishDir
  $Trigger = New-ScheduledTaskTrigger -Once -At ((Get-Date).AddMinutes(1))
  $Principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
  Register-ScheduledTask -TaskName $TaskName -Action $Action -Trigger $Trigger -Principal $Principal -Force | Out-Null
  Start-ScheduledTask -TaskName $TaskName
} catch {
  Write-Log "interactive scheduled launch failed, falling back to SSH session launch: $_"
  Start-Process -FilePath $Exe -WorkingDirectory $PublishDir
}
Start-Sleep -Seconds 8
if (-not (Get-Process -Name "IrisDrive" -ErrorAction SilentlyContinue)) {
  Write-Log "scheduled launch did not create an IrisDrive process; starting in SSH session"
  Start-Process -FilePath $Exe -WorkingDirectory $PublishDir
  Start-Sleep -Seconds 5
}
if (-not (Get-Process -Name "IrisDrive" -ErrorAction SilentlyContinue)) {
  throw "Windows app did not start"
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
  } | ssh "$host" 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -'
}

remote_status_json() {
  local kind="$1"
  local host="$2"

  case "$kind" in
    macos)
      ssh "$host" 'bash -se' <<'REMOTE_SH'
set -Eeuo pipefail
idrive="$HOME/src/iris-drive/target/debug/idrive"
app_group="${IRIS_DRIVE_DEV_VM_MACOS_APP_GROUP_IDENTIFIER:-}"
if [[ -z "$app_group" ]]; then
  app="$HOME/Applications/Iris Drive.app"
  app_group="$(codesign -d --entitlements :- "$app" 2>/dev/null \
    | plutil -extract com.apple.security.application-groups.0 raw -o - - 2>/dev/null \
    || true)"
fi
if [[ -z "$app_group" ]]; then
  app_group="group.to.iris.drive"
fi
config_dir="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$HOME/Library/Group Containers/$app_group/Iris Drive Dev}/Config"
"$idrive" --config-dir "$config_dir" status
REMOTE_SH
      ;;
    linux)
      ssh "$host" 'bash -se' <<'REMOTE_SH'
set -Eeuo pipefail
idrive="$HOME/src/iris-drive/target/debug/idrive"
config_dir="${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-$HOME/.config/iris-drive}"
"$idrive" --config-dir "$config_dir" status
REMOTE_SH
      ;;
    windows)
      ssh "$host" 'cmd /d /s /c ""%USERPROFILE%\src\iris-drive\windows\bin\Debug\net8.0-windows\win-x64\publish\idrive.exe" --config-dir "%APPDATA%\iris-drive" status"'
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
    if status="$(remote_status_json_retry "${KINDS[$i]}" "${HOSTS[$i]}" 3 0.5)"; then
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

    remote_transport_diagnostics "${KINDS[$i]}" "${HOSTS[$i]}" \
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

peers = {peer.get("label"): peer for peer in data.get("peers", [])}
missing = []
for label in wanted:
    peer = peers.get(label)
    if peer is None:
        missing.append(f"{label}:missing")
    elif peer.get("fips_online") is not True:
        missing.append(
            f"{label}:online={peer.get('fips_online')} state={peer.get('sync_state')}"
        )

if missing:
    print("; ".join(missing))
    raise SystemExit(1)

fips = (data.get("network") or {}).get("fips") or {}
connected = fips.get("connected_peers") or []
print("connected_peers=[" + ",".join(connected) + "]")
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

  [[ "$NO_RUN" == "1" ]] && return 0
  [[ "${IRIS_DRIVE_DEV_VM_SKIP_CONNECTIVITY_CHECK:-0}" == "1" ]] && return 0
  if [[ ${#LABELS[@]} -lt 2 ]]; then
    return 0
  fi

  log "waiting for selected VMs to see each other online over FIPS"
  start="$(date +%s)"
  while true; do
    failures=()
    for i in "${!LABELS[@]}"; do
      expected=()
      for j in "${!LABELS[@]}"; do
        [[ "$i" == "$j" ]] && continue
        expected+=("$(target_peer_hint_key "${HOSTS[$j]}")")
      done

      if ! status="$(remote_status_json_retry "${KINDS[$i]}" "${HOSTS[$i]}" 3 0.5)"; then
        failures+=("${LABELS[$i]}: status unavailable")
        continue
      fi
      if ! summary="$(status_missing_peers "$status" "${expected[@]}" 2>&1)"; then
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
  log "updating/building/running ${LABELS[$i]} on ${HOSTS[$i]}"
  case "${KINDS[$i]}" in
    macos|linux)
      run_posix_target "${LABELS[$i]}" "${KINDS[$i]}" "${HOSTS[$i]}" "${IRIS_BARES[$i]}" "${HASHTREE_BARES[$i]}" "${FIPS_BARES[$i]}" "${STATIC_PEERS_BY_INDEX[$i]:-}"
      ;;
    windows)
      run_windows_target "${LABELS[$i]}" "${HOSTS[$i]}" "${IRIS_BARES[$i]}" "${HASHTREE_BARES[$i]}" "${FIPS_BARES[$i]}" "${STATIC_PEERS_BY_INDEX[$i]:-}"
      ;;
    *)
      die "unknown target kind: ${KINDS[$i]}"
      ;;
  esac
done

check_dev_vm_connectivity
log "done"
