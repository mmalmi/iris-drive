#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/desktop-gui-smoke.sh linux [ssh-host|local]
  scripts/desktop-gui-smoke.sh windows <ssh-host>

Runs a native desktop GUI smoke against the selected platform shell. Linux uses
the real GTK window on the user's X11 session, or a disposable Xvfb display
when the VM has no active desktop session. Windows uses WPF UI Automation
against the real IrisDrive.exe window.
USAGE
}

target="${1:-}"
host="${2:-local}"

if [[ "$target" == "-h" || "$target" == "--help" || -z "$target" ]]; then
  usage
  exit 0
fi

run_linux_remote() {
  local remote="$1"
  local runner=(ssh "$remote" 'bash -se')
  if [[ "$remote" == "local" ]]; then
    runner=(bash -se)
  fi

  "${runner[@]}" <<'REMOTE_SH'
set -Eeuo pipefail

log() {
  printf '[linux-gui-smoke] %s\n' "$*" >&2
}

die() {
  printf '[linux-gui-smoke] ERROR: %s\n' "$*" >&2
  if [[ -f /tmp/iris-drive-linux-app.err.log ]]; then
    log "recent app stderr:"
    tail -80 /tmp/iris-drive-linux-app.err.log >&2 || true
  fi
  exit 1
}

repo="${IRIS_DRIVE_REPO:-$HOME/src/iris-drive}"
idrive="$repo/target/debug/idrive"
app="$repo/linux/target/debug/iris-drive"
config_dir="${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-$HOME/.config/iris-drive}"
xvfb_pid=""
wm_pid=""
app_pid=""
use_xvfb=0

cleanup() {
  if [[ -n "$app_pid" ]] && (( use_xvfb )); then
    pkill -P "$app_pid" >/dev/null 2>&1 || true
    kill "$app_pid" >/dev/null 2>&1 || true
  fi
  if [[ -n "$wm_pid" ]]; then
    kill "$wm_pid" >/dev/null 2>&1 || true
  fi
  if [[ -n "$xvfb_pid" ]]; then
    kill "$xvfb_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

[[ -x "$idrive" ]] || die "missing idrive helper at $idrive"
if [[ ! -x "$app" ]]; then
  log "building Linux GTK shell"
  (cd "$repo/linux" && cargo build)
fi
[[ -x "$app" ]] || die "missing Linux GTK app at $app"
command -v xdotool >/dev/null 2>&1 || die "xdotool is required for Linux GUI smoke"
command -v xwininfo >/dev/null 2>&1 || die "xwininfo is required for Linux GUI smoke"
command -v xdpyinfo >/dev/null 2>&1 || die "xdpyinfo is required for Linux GUI smoke"

display_ready() {
  xdpyinfo -display "$DISPLAY" >/dev/null 2>&1
}

display="${IRIS_DRIVE_DEV_VM_LINUX_DISPLAY:-${DISPLAY:-}}"
if [[ -n "$display" ]]; then
  export DISPLAY="$display"
fi
if [[ -z "${DISPLAY:-}" || ! display_ready ]]; then
  command -v Xvfb >/dev/null 2>&1 || die "no usable X display and Xvfb is not installed"
  export DISPLAY="${IRIS_DRIVE_DEV_VM_LINUX_XVFB_DISPLAY:-:98}"
  use_xvfb=1
  if ! display_ready; then
    log "starting disposable Xvfb display on DISPLAY=$DISPLAY"
    Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp \
      > /tmp/iris-drive-linux-xvfb.log \
      2>&1 &
    xvfb_pid="$!"
    for _ in $(seq 1 20); do
      display_ready && break
      sleep 0.2
    done
  fi
  display_ready || die "Xvfb display did not become ready"
  if command -v openbox >/dev/null 2>&1; then
    openbox > /tmp/iris-drive-linux-openbox.log 2>&1 &
    wm_pid="$!"
    sleep 0.3
  fi
fi

find_window() {
  xdotool search --onlyvisible --name '^Iris Drive$' 2>/dev/null | head -n 1
}

window_id="$(find_window || true)"
if [[ -z "$window_id" ]]; then
  log "launching Linux GTK shell on DISPLAY=$DISPLAY"
  mkdir -p "$config_dir"
  if (( use_xvfb )); then
    command -v dbus-run-session >/dev/null 2>&1 \
      || die "dbus-run-session is required for Linux GUI smoke with Xvfb"
    nohup dbus-run-session -- env \
      -u WAYLAND_DISPLAY \
      "DISPLAY=$DISPLAY" \
      "GDK_BACKEND=x11" \
      "GSK_RENDERER=cairo" \
      "LIBGL_ALWAYS_SOFTWARE=1" \
      "NO_AT_BRIDGE=1" \
      "GIO_USE_PORTALS=0" \
      "GTK_USE_PORTAL=0" \
      "IRIS_DRIVE_DISABLE_TRAY=1" \
      "IRIS_DRIVE_CLI=$idrive" \
      "IRIS_DRIVE_CONFIG_DIR=$config_dir" \
      "$app" \
      > /tmp/iris-drive-linux-app.out.log \
      2> /tmp/iris-drive-linux-app.err.log \
      < /dev/null &
  else
    nohup env \
      "DISPLAY=$DISPLAY" \
      "IRIS_DRIVE_DISABLE_TRAY=1" \
      "IRIS_DRIVE_CLI=$idrive" \
      "IRIS_DRIVE_CONFIG_DIR=$config_dir" \
      "$app" \
      > /tmp/iris-drive-linux-app.out.log \
      2> /tmp/iris-drive-linux-app.err.log \
      < /dev/null &
  fi
  app_pid="$!"
  disown "$app_pid" >/dev/null 2>&1 || true
fi

for _ in $(seq 1 40); do
  window_id="$(find_window || true)"
  if [[ -n "$window_id" ]]; then
    info="$(xwininfo -id "$window_id" 2>/dev/null || true)"
    width="$(awk '/Width:/ { print $2; exit }' <<<"$info")"
    height="$(awk '/Height:/ { print $2; exit }' <<<"$info")"
    if grep -F "Map State: IsViewable" <<<"$info" >/dev/null \
      && [[ "$width" =~ ^[0-9]+$ && "$height" =~ ^[0-9]+$ ]] \
      && (( width >= 640 && height >= 400 )); then
      xdotool windowactivate --sync "$window_id" >/dev/null 2>&1 || true
      xdotool windowfocus "$window_id" >/dev/null 2>&1 || true
      status="$("$idrive" --config-dir "$config_dir" status)"
      STATUS_JSON="$status" python3 - <<'PY'
import json
import os

status = json.loads(os.environ["STATUS_JSON"])
if not status.get("initialized"):
    raise SystemExit("Linux GUI smoke expected an initialized app profile")
summary = status.get("summary") or {}
network = status.get("network") or {}
if summary:
    if int(summary.get("authorized_device_count") or 0) < 1:
        raise SystemExit("Linux GUI summary has no authorized devices")
else:
    if int(network.get("authorized_device_count") or 0) < 1:
        raise SystemExit("Linux status has no authorized devices")
PY
      screenshot="/tmp/iris-drive-linux-gui-smoke.png"
      rm -f "$screenshot"
      if command -v gnome-screenshot >/dev/null 2>&1; then
        gnome-screenshot -w -f "$screenshot" >/dev/null 2>&1 || true
      elif command -v import >/dev/null 2>&1; then
        import -window "$window_id" "$screenshot" >/dev/null 2>&1 || true
      fi
      if [[ -f "$screenshot" && ! -s "$screenshot" ]]; then
        die "Linux GUI screenshot was empty"
      fi
      echo "LINUX_GUI_SMOKE_OK"
      exit 0
    fi
  fi
  sleep 0.5
done

die "Linux GTK shell did not expose a visible Iris Drive window"
REMOTE_SH
}

run_windows_remote() {
  local remote="$1"
  [[ "$remote" != "local" ]] || {
    echo "windows GUI smoke requires a Windows SSH host" >&2
    exit 2
  }

  ssh "$remote" 'cmd /d /s /c "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""`$script = [Console]::In.ReadToEnd(); & ([scriptblock]::Create(`$script))"""' <<'REMOTE_PS'
$ErrorActionPreference = "Stop"

function Write-SmokeLog([string]$Message) {
  [Console]::Error.WriteLine("[windows-gui-smoke] $Message")
}

function Fail([string]$Message) {
  [Console]::Error.WriteLine("[windows-gui-smoke] ERROR: $Message")
  exit 1
}

$IrisRepo = Join-Path $HOME "src\iris-drive"
$PublishDir = Join-Path $IrisRepo "windows\bin\Debug\net8.0-windows\win-x64\publish"
$Exe = Join-Path $PublishDir "IrisDrive.exe"
$Idrive = Join-Path $PublishDir "idrive.exe"
$ConfigDir = Join-Path $env:APPDATA "iris-drive"

if (-not (Test-Path $Exe)) {
  Fail "missing published Windows app at $Exe"
}
if (-not (Test-Path $Idrive)) {
  $Idrive = Join-Path $IrisRepo "target\debug\idrive.exe"
}
if (-not (Test-Path $Idrive)) {
  Fail "missing idrive helper"
}

Add-Type -Namespace IrisDriveSmoke -Name NativeMethods -MemberDefinition @"
  [System.Runtime.InteropServices.DllImport("user32.dll")]
  public static extern bool IsWindowVisible(System.IntPtr hWnd);
  [System.Runtime.InteropServices.DllImport("user32.dll")]
  public static extern bool ShowWindow(System.IntPtr hWnd, int nCmdShow);
  [System.Runtime.InteropServices.DllImport("user32.dll")]
  public static extern bool SetForegroundWindow(System.IntPtr hWnd);
"@

function Current-IrisWindowProcess {
  Get-Process -Name "IrisDrive" -ErrorAction SilentlyContinue |
    Where-Object {
      $_.MainWindowHandle -ne [IntPtr]::Zero -and
      [IrisDriveSmoke.NativeMethods]::IsWindowVisible($_.MainWindowHandle)
    } |
    Sort-Object StartTime -Descending |
    Select-Object -First 1
}

function Stop-HiddenIrisWindowProcesses {
  Get-Process -Name "IrisDrive" -ErrorAction SilentlyContinue |
    Where-Object {
      $_.MainWindowHandle -eq [IntPtr]::Zero -or
      -not [IrisDriveSmoke.NativeMethods]::IsWindowVisible($_.MainWindowHandle)
    } |
    ForEach-Object {
      Write-SmokeLog "stopping hidden Windows WPF shell process $($_.Id)"
      Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
}

function Test-InteractiveDesktop {
  $Computer = Get-CimInstance Win32_ComputerSystem
  if (-not [string]::IsNullOrWhiteSpace($Computer.UserName)) {
    return $true
  }
  return [bool](Get-Process -Name "explorer" -ErrorAction SilentlyContinue)
}

function Require-InteractiveDesktop {
  if (-not (Test-InteractiveDesktop)) {
    Fail "Windows GUI smoke requires an unlocked interactive desktop session; unlock the Windows VM console and rerun"
  }
}

function Start-IrisWindowProcess {
  Require-InteractiveDesktop
  $LaunchScript = Join-Path $PublishDir "launch-iris-drive-gui-smoke.cmd"
@"
@echo off
set IRIS_DRIVE_CLI=$Idrive
set IRIS_DRIVE_EXTERNAL_DAEMON=true
cd /d "$PublishDir"
start "" "$Exe"
"@ | Set-Content -Encoding ASCII $LaunchScript

  $TaskName = "IrisDriveGuiSmokeLaunch"
  Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
  $Action = New-ScheduledTaskAction -Execute "cmd.exe" -Argument "/c `"$LaunchScript`"" -WorkingDirectory $PublishDir
  $Trigger = New-ScheduledTaskTrigger -Once -At ((Get-Date).AddMinutes(1))
  $Principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
  Register-ScheduledTask -TaskName $TaskName -Action $Action -Trigger $Trigger -Principal $Principal -Force | Out-Null
  Start-ScheduledTask -TaskName $TaskName
}

$Process = Current-IrisWindowProcess
if (-not $Process) {
  Stop-HiddenIrisWindowProcesses
  Start-Sleep -Seconds 1
  Write-SmokeLog "launching Windows WPF shell"
  Start-IrisWindowProcess
}

for ($i = 0; $i -lt 50; $i++) {
  $Process = Current-IrisWindowProcess
  if ($Process) { break }
  Start-Sleep -Milliseconds 500
}
if (-not $Process) {
  Fail "Windows WPF shell did not expose a visible Iris Drive window"
}

[void][IrisDriveSmoke.NativeMethods]::ShowWindow($Process.MainWindowHandle, 9)
[void][IrisDriveSmoke.NativeMethods]::SetForegroundWindow($Process.MainWindowHandle)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$Window = [System.Windows.Automation.AutomationElement]::FromHandle($Process.MainWindowHandle)
if (-not $Window) {
  Fail "UI Automation could not attach to Iris Drive window"
}

function Find-ElementByName([string]$Name) {
  $Condition = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::NameProperty,
    $Name
  )
  return $Window.FindFirst(
    [System.Windows.Automation.TreeScope]::Descendants,
    $Condition
  )
}

function Find-ButtonByName([string]$Name) {
  $Condition = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::NameProperty,
    $Name
  )
  $Matches = $Window.FindAll(
    [System.Windows.Automation.TreeScope]::Descendants,
    $Condition
  )
  for ($i = 0; $i -lt $Matches.Count; $i++) {
    $Element = $Matches.Item($i)
    if ($Element.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button) {
      return $Element
    }
  }
  return $null
}

function Require-Element([string]$Name) {
  $Element = Find-ElementByName $Name
  if (-not $Element) {
    Fail "missing UI Automation element named '$Name'"
  }
  return $Element
}

function Invoke-Button([string]$Name) {
  $Button = Find-ButtonByName $Name
  if (-not $Button) {
    Fail "missing button named '$Name'"
  }
  $Pattern = $null
  if (-not $Button.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$Pattern)) {
    Fail "button '$Name' does not expose InvokePattern"
  }
  $Pattern.Invoke()
  Start-Sleep -Milliseconds 500
}

[void](Require-Element "My Drive")
[void](Require-Element "Open Drive Folder")
[void](Require-Element "Files")
[void](Require-Element "Storage")
[void](Require-Element "Devices")
Invoke-Button "Devices"
[void](Require-Element "Linked devices")
Invoke-Button "Network"
[void](Require-Element "FIPS")
[void](Require-Element "Relays")
Invoke-Button "My Drive"

$Status = & $Idrive --config-dir $ConfigDir status | ConvertFrom-Json
if (-not $Status.initialized) {
  Fail "Windows GUI smoke expected an initialized app profile"
}
$Authorized = 0
if ($Status.summary -and $Status.summary.authorized_device_count) {
  $Authorized = [int]$Status.summary.authorized_device_count
} elseif ($Status.network -and $Status.network.authorized_device_count) {
  $Authorized = [int]$Status.network.authorized_device_count
}
if ($Authorized -lt 1) {
  Fail "Windows status has no authorized devices"
}

"WINDOWS_GUI_SMOKE_OK"
REMOTE_PS
}

case "$target" in
  linux)
    run_linux_remote "$host"
    ;;
  windows)
    run_windows_remote "$host"
    ;;
  *)
    echo "unknown desktop GUI smoke target: $target" >&2
    usage >&2
    exit 2
    ;;
esac
