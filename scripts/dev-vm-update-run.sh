#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HASHTREE_ROOT="${IRIS_DRIVE_HASHTREE_ROOT:-$(cd "$ROOT/../hashtree/rust" && pwd)}"
HASHTREE_ROOT="$(git -C "$HASHTREE_ROOT" rev-parse --show-toplevel)"
SYNC_BRANCH="${IRIS_DRIVE_DEV_VM_SYNC_BRANCH:-codex/dev-vm-sync}"
TARGET_BRANCH="${IRIS_DRIVE_DEV_VM_TARGET_BRANCH:-$(git -C "$ROOT" branch --show-current || true)}"
TARGET_BRANCH="${TARGET_BRANCH:-master}"
FORCE=0
SKIP_PUSH=0
NO_RUN=0
LIST_TARGETS=0
ONLY_LABELS=()

usage() {
  cat <<'USAGE'
Usage:
  scripts/dev-vm-update-run.sh [--force] [--only macos|ubuntu|windows] [--skip-push] [--no-run]
  scripts/dev-vm-update-run.sh --list-targets

Syncs the current committed iris-drive and hashtree revisions to the configured
VM git remotes, updates the VM worktrees, builds, then restarts the dev app or
daemon with native FIPS UDP over the nvpn overlay while keeping WebRTC enabled.

Defaults are derived from local git remotes:
  macos    iris-drive remote macos-utm, hashtree remote macos-utm
  ubuntu   iris-drive remote ubuntu-dev, hashtree remote ubuntu-dev
  windows  iris-drive remote win11-dev, hashtree remote win11-dev

Environment:
  IRIS_DRIVE_DEV_VM_MACOS_REMOTE      Git remote name for the macOS VM.
  IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE     Git remote name for the Ubuntu VM.
  IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE    Git remote name for the Windows VM.
  IRIS_DRIVE_DEV_VM_FIPS_PORT         UDP port advertised over nvpn (default: 22121).
  IRIS_DRIVE_DEV_VM_SYNC_BRANCH       Temporary branch pushed to VM bare repos.
  IRIS_DRIVE_DEV_VM_TARGET_BRANCH     Branch name checked out in VM worktrees.
  IRIS_DRIVE_DEV_VM_REQUIRE_CLEAN=1   Refuse to run when local repos are dirty.
  IRIS_DRIVE_HASHTREE_ROOT            Local hashtree/rust checkout.

Remote worktree paths are expected to be:
  ~/src/iris-drive
  ~/src/hashtree

Use --force when VM worktrees have local scratch edits that should be replaced
by the pushed revision. The script never git-cleans untracked files.
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

declare -a LABELS=()
declare -a KINDS=()
declare -a HOSTS=()
declare -a IRIS_REMOTES=()
declare -a HASHTREE_REMOTES=()
declare -a IRIS_BARES=()
declare -a HASHTREE_BARES=()

add_target_from_remotes() {
  local label="$1"
  local kind="$2"
  local iris_remote="$3"
  local hashtree_remote="$4"
  local iris_parts
  local hashtree_parts
  local host
  local hashtree_host
  local iris_bare
  local hashtree_bare

  contains_label "$label" || return 0

  iris_parts="$(remote_url_parts "$ROOT" "$iris_remote" || true)"
  hashtree_parts="$(remote_url_parts "$HASHTREE_ROOT" "$hashtree_remote" || true)"
  if [[ -z "$iris_parts" || -z "$hashtree_parts" ]]; then
    if [[ ${#ONLY_LABELS[@]} -gt 0 ]]; then
      die "missing git remotes for requested target $label"
    fi
    log "skipping $label; missing git remote $iris_remote or hashtree remote $hashtree_remote"
    return 0
  fi

  host="${iris_parts%%$'\t'*}"
  iris_bare="${iris_parts#*$'\t'}"
  hashtree_host="${hashtree_parts%%$'\t'*}"
  hashtree_bare="${hashtree_parts#*$'\t'}"
  if [[ "$host" != "$hashtree_host" ]]; then
    die "$label iris-drive remote host ($host) differs from hashtree host ($hashtree_host)"
  fi

  LABELS+=("$label")
  KINDS+=("$kind")
  HOSTS+=("$host")
  IRIS_REMOTES+=("$iris_remote")
  HASHTREE_REMOTES+=("$hashtree_remote")
  IRIS_BARES+=("$iris_bare")
  HASHTREE_BARES+=("$hashtree_bare")
}

warn_or_fail_local_dirty "$ROOT" "iris-drive"
warn_or_fail_local_dirty "$HASHTREE_ROOT" "hashtree"

add_target_from_remotes \
  macos \
  macos \
  "${IRIS_DRIVE_DEV_VM_MACOS_REMOTE:-macos-utm}" \
  "${IRIS_DRIVE_DEV_VM_MACOS_HASHTREE_REMOTE:-${IRIS_DRIVE_DEV_VM_MACOS_REMOTE:-macos-utm}}"
add_target_from_remotes \
  ubuntu \
  linux \
  "${IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE:-ubuntu-dev}" \
  "${IRIS_DRIVE_DEV_VM_UBUNTU_HASHTREE_REMOTE:-${IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE:-ubuntu-dev}}"
add_target_from_remotes \
  windows \
  windows \
  "${IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE:-win11-dev}" \
  "${IRIS_DRIVE_DEV_VM_WINDOWS_HASHTREE_REMOTE:-${IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE:-win11-dev}}"

if [[ ${#LABELS[@]} -eq 0 ]]; then
  usage >&2
  die "no VM targets configured"
fi

if [[ "$LIST_TARGETS" == "1" ]]; then
  for i in "${!LABELS[@]}"; do
    printf '%s\t%s\t%s\tiris=%s\thashtree=%s\n' \
      "${LABELS[$i]}" \
      "${KINDS[$i]}" \
      "${HOSTS[$i]}" \
      "${IRIS_BARES[$i]}" \
      "${HASHTREE_BARES[$i]}"
  done
  exit 0
fi

if [[ "$SKIP_PUSH" != "1" ]]; then
  for i in "${!LABELS[@]}"; do
    log "pushing iris-drive HEAD to ${LABELS[$i]} (${IRIS_REMOTES[$i]}:$SYNC_BRANCH)"
    git -C "$ROOT" push "${IRIS_REMOTES[$i]}" "HEAD:refs/heads/$SYNC_BRANCH"
    log "pushing hashtree HEAD to ${LABELS[$i]} (${HASHTREE_REMOTES[$i]}:$SYNC_BRANCH)"
    git -C "$HASHTREE_ROOT" push "${HASHTREE_REMOTES[$i]}" "HEAD:refs/heads/$SYNC_BRANCH"
  done
fi

run_posix_target() {
  local label="$1"
  local kind="$2"
  local host="$3"
  local iris_bare="$4"
  local hashtree_bare="$5"

  {
    printf 'LABEL=%s\n' "$(sh_quote "$label")"
    printf 'KIND=%s\n' "$(sh_quote "$kind")"
    printf 'IRIS_BARE=%s\n' "$(sh_quote "$iris_bare")"
    printf 'HASHTREE_BARE=%s\n' "$(sh_quote "$hashtree_bare")"
    printf 'SYNC_BRANCH=%s\n' "$(sh_quote "$SYNC_BRANCH")"
    printf 'TARGET_BRANCH=%s\n' "$(sh_quote "$TARGET_BRANCH")"
    printf 'FORCE=%s\n' "$(sh_quote "$FORCE")"
    printf 'NO_RUN=%s\n' "$(sh_quote "$NO_RUN")"
    printf 'FIPS_PORT=%s\n' "$(sh_quote "${IRIS_DRIVE_DEV_VM_FIPS_PORT:-22121}")"
    cat <<'REMOTE_SH'
set -Eeuo pipefail

log() {
  printf '[%s] %s\n' "$LABEL" "$*" >&2
}

expand_path() {
  case "$1" in
    "~") printf '%s\n' "$HOME" ;;
    "~/"*) printf '%s/%s\n' "$HOME" "${1#~/}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

require_clean_or_force() {
  local repo="$1"
  local name="$2"
  local dirty
  dirty="$(git -C "$repo" status --short)"
  if [[ -n "$dirty" && "$FORCE" != "1" ]]; then
    printf '[%s] %s has local changes:\n%s\n' "$LABEL" "$name" "$dirty" >&2
    printf '[%s] rerun with --force to replace tracked VM changes\n' "$LABEL" >&2
    exit 3
  fi
}

sync_repo() {
  local repo="$1"
  local name="$2"
  local bare="$3"

  bare="$(expand_path "$bare")"
  [[ -d "$repo/.git" ]] || { log "missing checkout: $repo"; exit 1; }
  require_clean_or_force "$repo" "$name"
  log "fetching $name from $bare"
  git -C "$repo" fetch "$bare" "$SYNC_BRANCH"
  if [[ "$FORCE" == "1" ]]; then
    git -C "$repo" checkout --force -B "$TARGET_BRANCH" FETCH_HEAD
    git -C "$repo" reset --hard FETCH_HEAD
  else
    git -C "$repo" checkout -B "$TARGET_BRANCH" FETCH_HEAD
  fi
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

stop_idrive_daemon() {
  local config_dir="$1"
  local status_file="$config_dir/daemon-status.json"
  local pid=""

  if [[ -f "$status_file" ]]; then
    pid="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("pid", ""))' "$status_file" 2>/dev/null || true)"
  fi
  if [[ -n "$pid" ]]; then
    kill "$pid" >/dev/null 2>&1 || true
  fi
}

run_linux() {
  local iris_repo="$HOME/src/iris-drive"
  local idrive="$iris_repo/target/debug/idrive"
  local config_dir="${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-$HOME/.config/iris-drive}"
  local mountpoint="${IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT:-$HOME/Iris Drive}"
  local overlay_ip=""
  local external_addr=""

  log "building idrive"
  (cd "$iris_repo" && cargo build -p idrive)
  [[ "$NO_RUN" == "1" ]] && return 0

  overlay_ip="$(detect_overlay_ip || true)"
  if [[ -n "$overlay_ip" ]]; then
    external_addr="$overlay_ip:$FIPS_PORT"
  fi

  log "restarting idrive daemon"
  mkdir -p "$config_dir" "$mountpoint"
  stop_idrive_daemon "$config_dir"
  rm -f "$config_dir/daemon.lock"
  nohup env \
    "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=0.0.0.0:$FIPS_PORT" \
    "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=$external_addr" \
    "IRIS_DRIVE_FIPS_UDP_PUBLIC=true" \
    "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true" \
    "$idrive" --config-dir "$config_dir" daemon \
      --watch-interval 2 \
      --watch-debounce-ms 100 \
      --mount \
      --mountpoint "$mountpoint" \
      > /tmp/iris-drive-daemon.log 2>&1 < /dev/null &
  sleep 3
  "$idrive" --config-dir "$config_dir" status \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); f=(d.get("network") or {}).get("fips") or {}; print("connected_peers=", f.get("connected_peers")); print("peers=", [(p.get("label"), p.get("fips_online"), p.get("sync_state")) for p in d.get("peers", [])])'
}

run_macos() {
  local iris_repo="$HOME/src/iris-drive"
  local idrive="$iris_repo/target/debug/idrive"
  local app="$iris_repo/macos/.build/DerivedData/Build/Products/Debug/Iris Drive.app"
  local app_base="${IRIS_DRIVE_DEV_VM_MACOS_APP_BASE_DIR:-$HOME/.local/share/iris-drive-dev-app}"
  local config_dir="$app_base/Config"
  local overlay_ip=""
  local external_addr=""

  log "building idrive helper"
  (cd "$iris_repo" && cargo build -p idrive)
  log "building macOS app"
  (cd "$iris_repo" && xcodebuild \
    -project macos/IrisDriveMac.xcodeproj \
    -scheme IrisDriveMac \
    -configuration Debug \
    -derivedDataPath macos/.build/DerivedData \
    CODE_SIGNING_ALLOWED=NO \
    build)
  cp "$idrive" "$app/Contents/MacOS/idrive"
  chmod +x "$app/Contents/MacOS/idrive"
  codesign --force --sign - "$app/Contents/MacOS/idrive" >/dev/null 2>&1 || true
  [[ "$NO_RUN" == "1" ]] && return 0

  overlay_ip="$(detect_overlay_ip || true)"
  if [[ -n "$overlay_ip" ]]; then
    external_addr="$overlay_ip:$FIPS_PORT"
  fi

  log "restarting macOS app"
  pkill -x "Iris Drive" >/dev/null 2>&1 || true
  mkdir -p "$config_dir"
  stop_idrive_daemon "$config_dir"
  rm -f "$config_dir/daemon.lock"
  sleep 1
  open \
    --env "IRIS_DRIVE_APP_BASE_DIR=$app_base" \
    --env "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=0.0.0.0:$FIPS_PORT" \
    --env "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=$external_addr" \
    --env "IRIS_DRIVE_FIPS_UDP_PUBLIC=true" \
    --env "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true" \
    "$app"
  sleep 8
  "$idrive" --config-dir "$config_dir" status \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); f=(d.get("network") or {}).get("fips") or {}; print("connected_peers=", f.get("connected_peers")); print("peers=", [(p.get("label"), p.get("fips_online"), p.get("sync_state")) for p in d.get("peers", [])])'
}

sync_repo "$HOME/src/hashtree" hashtree "$HASHTREE_BARE"
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

  {
    printf '$Label = %s\n' "$(ps_quote "$label")"
    printf '$IrisBare = %s\n' "$(ps_quote "$iris_bare")"
    printf '$HashtreeBare = %s\n' "$(ps_quote "$hashtree_bare")"
    printf '$SyncBranch = %s\n' "$(ps_quote "$SYNC_BRANCH")"
    printf '$TargetBranch = %s\n' "$(ps_quote "$TARGET_BRANCH")"
    printf '$Force = %s\n' "$(ps_quote "$FORCE")"
    printf '$NoRun = %s\n' "$(ps_quote "$NO_RUN")"
    printf '$FipsPort = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_FIPS_PORT:-22121}")"
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
  return $Path
}

function Require-CleanOrForce([string]$Repo, [string]$Name) {
  $Dirty = git -C $Repo status --short
  if ($Dirty -and $Force -ne "1") {
    [Console]::Error.WriteLine("[$Label] $Name has local changes:")
    [Console]::Error.WriteLine(($Dirty -join [Environment]::NewLine))
    [Console]::Error.WriteLine("[$Label] rerun with --force to replace tracked VM changes")
    exit 3
  }
}

function Sync-Repo([string]$Repo, [string]$Name, [string]$Bare) {
  $Bare = Expand-RemotePath $Bare
  if (-not (Test-Path (Join-Path $Repo ".git"))) {
    throw "missing checkout: $Repo"
  }
  Require-CleanOrForce $Repo $Name
  Write-Log "fetching $Name from $Bare"
  git -C $Repo fetch $Bare $SyncBranch
  if ($LASTEXITCODE -ne 0) { throw "git fetch failed for $Name" }
  if ($Force -eq "1") {
    git -C $Repo checkout --force -B $TargetBranch FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw "git checkout failed for $Name" }
    git -C $Repo reset --hard FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw "git reset failed for $Name" }
  } else {
    git -C $Repo checkout -B $TargetBranch FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw "git checkout failed for $Name" }
  }
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

$IrisRepo = Join-Path $HOME "src\iris-drive"
$HashtreeRepo = Join-Path $HOME "src\hashtree"
Sync-Repo $HashtreeRepo "hashtree" $HashtreeBare
Sync-Repo $IrisRepo "iris-drive" $IrisBare

Write-Log "publishing Windows dev app"
Set-Location $IrisRepo
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\windows-publish.ps1 -Configuration Debug -StopRunningApp
if ($LASTEXITCODE -ne 0) { throw "windows publish failed" }

if ($NoRun -eq "1") {
  exit 0
}

$OverlayIp = Detect-OverlayIp
$ExternalAddr = ""
if ($OverlayIp) {
  $ExternalAddr = "${OverlayIp}:$FipsPort"
}

$env:IRIS_DRIVE_FIPS_UDP_BIND_ADDR = "0.0.0.0:$FipsPort"
$env:IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR = $ExternalAddr
$env:IRIS_DRIVE_FIPS_UDP_PUBLIC = "true"
$env:IRIS_DRIVE_FIPS_ENABLE_WEBRTC = "true"

$PublishDir = Join-Path $IrisRepo "windows\bin\Debug\net8.0-windows\win-x64\publish"
$Exe = Join-Path $PublishDir "IrisDrive.exe"
if (-not (Test-Path $Exe)) {
  throw "missing published Windows app: $Exe"
}
Write-Log "starting Windows app"
Start-Process -FilePath $Exe -WorkingDirectory $PublishDir
Start-Sleep -Seconds 8

$Idrive = Join-Path $PublishDir "idrive.exe"
try {
  $Status = & $Idrive status | ConvertFrom-Json
  $Connected = $Status.network.fips.connected_peers -join ","
  $Peers = @($Status.peers | ForEach-Object { "$($_.label):$($_.fips_online):$($_.sync_state)" }) -join ", "
  Write-Output "connected_peers=[$Connected]"
  Write-Output "peers=[$Peers]"
} catch {
  Write-Log "status read failed after launch: $_"
}
REMOTE_PS
  } | ssh "$host" 'powershell -NoProfile -ExecutionPolicy Bypass -Command "`$script = [Console]::In.ReadToEnd(); Invoke-Expression `$script"'
}

for i in "${!LABELS[@]}"; do
  log "updating/building/running ${LABELS[$i]} on ${HOSTS[$i]}"
  case "${KINDS[$i]}" in
    macos|linux)
      run_posix_target "${LABELS[$i]}" "${KINDS[$i]}" "${HOSTS[$i]}" "${IRIS_BARES[$i]}" "${HASHTREE_BARES[$i]}"
      ;;
    windows)
      run_windows_target "${LABELS[$i]}" "${HOSTS[$i]}" "${IRIS_BARES[$i]}" "${HASHTREE_BARES[$i]}"
      ;;
    *)
      die "unknown target kind: ${KINDS[$i]}"
      ;;
  esac
done

log "done"
