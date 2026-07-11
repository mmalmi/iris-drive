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

When the Windows VM is reachable only from a Linux jump host, set
IRIS_DRIVE_E2E_WINDOWS_GUEST_HOST to the Windows SSH alias. The managed lab
also treats ssh host "vader" as a jump host to "win11-dev".
USAGE
}

sh_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/'\\\\''/g")"
}

ps_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/''/g")"
}

windows_guest_host_for() {
  local remote="$1"
  if [[ -n "${IRIS_DRIVE_E2E_WINDOWS_GUEST_HOST:-}" ]]; then
    printf "%s" "$IRIS_DRIVE_E2E_WINDOWS_GUEST_HOST"
  elif [[ "$remote" == "vader" ]]; then
    printf "win11-dev"
  fi
}

windows_powershell_command_for() {
  local remote="$1"
  local guest
  guest="$(windows_guest_host_for "$remote")"
  if [[ -n "$guest" ]]; then
    printf 'ssh %s powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -' "$(sh_quote "$guest")"
  else
    printf 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command -'
  fi
}

linux_remote_shell() {
  local assignments=()
  if [[ -n "${IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR:-}" ]]; then
    assignments+=("IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR=$(sh_quote "$IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR")")
  fi
  if [[ -n "${IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT:-}" ]]; then
    assignments+=("IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT=$(sh_quote "$IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT")")
  fi
  if [[ ${#assignments[@]} -eq 0 ]]; then
    printf 'bash -se'
  else
    printf '%s bash -se' "${assignments[*]}"
  fi
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
  else
    runner=(ssh "$remote" "$(linux_remote_shell)")
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
  if pgrep -f -- "$app" >/dev/null 2>&1; then
    log "stopping stale Linux GTK shell process without a visible window"
    pkill -f -- "$app" >/dev/null 2>&1 || true
    sleep 0.5
  fi
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
    if int(summary.get("authorized_app_key_count") or summary.get("authorized_device_count") or 0) < 1:
        raise SystemExit("Linux GUI summary has no authorized app keys")
else:
    if int(network.get("authorized_app_key_count") or network.get("authorized_device_count") or 0) < 1:
        raise SystemExit("Linux status has no authorized app keys")
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

  {
    printf '$ConfigDirOverride = %s\n' "$(ps_quote "${IRIS_DRIVE_DEV_VM_WINDOWS_CONFIG_DIR:-}")"
    cat <<'REMOTE_PS'
$ErrorActionPreference = "Stop"

function Write-SmokeLog([string]$Message) {
  [Console]::Error.WriteLine("[windows-gui-smoke] $Message")
}

function Fail([string]$Message) {
  if (Get-Command Write-FailureScreenshot -ErrorAction SilentlyContinue) {
    Write-FailureScreenshot
  }
  if (Get-Command Write-ShellTrace -ErrorAction SilentlyContinue) {
    Write-ShellTrace
  }
  [Console]::Error.WriteLine("[windows-gui-smoke] ERROR: $Message")
  exit 1
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

$IrisRepo = Join-Path $HOME "src\iris-drive"
$PublishDir = Join-Path $IrisRepo "windows\bin\Debug\net8.0-windows\win-x64\publish"
$Exe = Join-Path $PublishDir "IrisDrive.exe"
$Idrive = Join-Path $PublishDir "idrive.exe"
$ShellTrace = Join-Path $PublishDir "windows-shell-smoke.log"
if ([string]::IsNullOrWhiteSpace($ConfigDirOverride)) {
  $ConfigDir = Join-Path $env:APPDATA "iris-drive"
} else {
  $ConfigDir = Expand-RemotePath $ConfigDirOverride
}

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

function Write-FailureScreenshot {
  if ($script:LastWindowsGuiPreflightScreenshot -and (Test-Path $script:LastWindowsGuiPreflightScreenshot)) {
    $Bytes = (Get-Item $script:LastWindowsGuiPreflightScreenshot).Length
    Write-SmokeLog "failure screenshot saved to $script:LastWindowsGuiPreflightScreenshot bytes=$Bytes"
    return
  }
  try {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    $Screenshot = Join-Path $PublishDir "gui-smoke-failure.png"
    $Bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $Bitmap = [System.Drawing.Bitmap]::new($Bounds.Width, $Bounds.Height)
    $Graphics = [System.Drawing.Graphics]::FromImage($Bitmap)
    $Graphics.CopyFromScreen($Bounds.Location, [System.Drawing.Point]::Empty, $Bounds.Size)
    $Bitmap.Save($Screenshot, [System.Drawing.Imaging.ImageFormat]::Png)
    $Graphics.Dispose()
    $Bitmap.Dispose()
    $Bytes = (Get-Item $Screenshot).Length
    Write-SmokeLog "failure screenshot saved to $Screenshot bytes=$Bytes"
  } catch {
    Write-SmokeLog "failure screenshot unavailable: $($_.Exception.Message)"
  }
}

function Write-ShellTrace {
  if ($ShellTrace -and (Test-Path $ShellTrace)) {
    Write-SmokeLog "Windows shell startup trace:"
    Get-Content $ShellTrace -Tail 120 | ForEach-Object {
      [Console]::Error.WriteLine("[windows-gui-smoke] trace $_")
    }
  }
}

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

function Test-VisibleWindowLaunch {
  $TaskName = "IrisDriveGuiSmokeWindowPreflight"
  $ProbeDir = Join-Path $PublishDir "gui-smoke-preflight"
  $ProbeScript = Join-Path $ProbeDir "capture-desktop.ps1"
  $ProbeImage = Join-Path $ProbeDir "desktop.png"
  $PersistedProbeImage = Join-Path $PublishDir "gui-smoke-preflight.png"
  $ProbeResult = Join-Path $ProbeDir "result.txt"
  $ProbeError = Join-Path $ProbeDir "error.txt"
  New-Item -ItemType Directory -Force -Path $ProbeDir | Out-Null
  Remove-Item -Force -ErrorAction SilentlyContinue $ProbeImage, $PersistedProbeImage, $ProbeResult, $ProbeError
@"
`$ErrorActionPreference = "Stop"
try {
  Add-Type -AssemblyName System.Windows.Forms
  Add-Type -AssemblyName System.Drawing
  `$Bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  `$Bitmap = [System.Drawing.Bitmap]::new(`$Bounds.Width, `$Bounds.Height)
  `$Graphics = [System.Drawing.Graphics]::FromImage(`$Bitmap)
  `$Graphics.CopyFromScreen(`$Bounds.Location, [System.Drawing.Point]::Empty, `$Bounds.Size)
  `$Bitmap.Save("$ProbeImage", [System.Drawing.Imaging.ImageFormat]::Png)
  `$Graphics.Dispose()
  `$Bitmap.Dispose()
  "captured width=`$(`$Bounds.Width) height=`$(`$Bounds.Height)" | Set-Content -Encoding ASCII "$ProbeResult"
} catch {
  (`$_ | Out-String) | Set-Content -Encoding ASCII "$ProbeError"
}
"@ | Set-Content -Encoding ASCII $ProbeScript

  try {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    $Action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$ProbeScript`"" -WorkingDirectory $ProbeDir
    $Trigger = New-ScheduledTaskTrigger -Once -At ((Get-Date).AddMinutes(1))
    $Principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
    Register-ScheduledTask -TaskName $TaskName -Action $Action -Trigger $Trigger -Principal $Principal -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName

    for ($i = 0; $i -lt 40; $i++) {
      if ((Test-Path $ProbeResult) -or (Test-Path $ProbeError)) {
        break
      }
      Start-Sleep -Milliseconds 250
    }
    $Task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    $TaskInfo = Get-ScheduledTaskInfo -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($Task -or $TaskInfo) {
      Write-SmokeLog "Windows GUI preflight task state=$($Task.State) last_result=$($TaskInfo.LastTaskResult)"
    }
    if (Test-Path $ProbeError) {
      Write-SmokeLog "Windows GUI preflight screenshot error: $((Get-Content $ProbeError -Raw).Trim())"
      return $false
    }
    if (-not (Test-Path $ProbeImage)) {
      Write-SmokeLog "Windows GUI preflight did not produce a desktop screenshot"
      return $false
    }
    $ProbeImageBytes = (Get-Item $ProbeImage).Length
    $ProbeSummary = if (Test-Path $ProbeResult) { (Get-Content $ProbeResult -Raw).Trim() } else { "captured" }
    Copy-Item -Force $ProbeImage $PersistedProbeImage
    $script:LastWindowsGuiPreflightScreenshot = $PersistedProbeImage
    Write-SmokeLog "Windows GUI preflight desktop screenshot $ProbeSummary bytes=$ProbeImageBytes file=$PersistedProbeImage"
    if ($ProbeImageBytes -gt 1024) {
      return $true
    }
    return $false
  } finally {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $ProbeDir
  }
}

function Require-InteractiveDesktop {
  if (-not (Test-InteractiveDesktop)) {
    Fail "Windows GUI smoke requires an unlocked interactive desktop session; unlock the Windows VM console and rerun"
  }
  if (-not (Test-VisibleWindowLaunch)) {
    Fail "Windows GUI smoke requires a desktop session that exposes visible windows; the active session did not show a disposable Notepad preflight window. Reattach or unlock the VM console/RDP desktop and rerun"
  }
}

function Invoke-InteractiveGuiSmoke {
  Require-InteractiveDesktop
  $TaskName = "IrisDriveGuiSmokeInteractive"
  $RunDir = Join-Path $PublishDir "gui-smoke-interactive"
  $WorkerScript = Join-Path $RunDir "run-gui-smoke.ps1"
  $ResultFile = Join-Path $RunDir "result.txt"
  $ErrorFile = Join-Path $RunDir "error.txt"
  $WorkerLog = Join-Path $RunDir "worker.log"
  $InteractiveScreenshot = Join-Path $PublishDir "gui-smoke-interactive.png"
  New-Item -ItemType Directory -Force -Path $RunDir | Out-Null
  Remove-Item -Force -ErrorAction SilentlyContinue $ResultFile, $ErrorFile, $WorkerLog, $InteractiveScreenshot, $ShellTrace

@'
param(
  [string]$PublishDir,
  [string]$Exe,
  [string]$Idrive,
  [string]$ConfigDir,
  [string]$ShellTrace,
  [string]$ResultFile,
  [string]$ErrorFile,
  [string]$WorkerLog,
  [string]$Screenshot
)

$ErrorActionPreference = "Stop"

function Log([string]$Message) {
  Add-Content -Encoding ASCII -Path $WorkerLog -Value "$(Get-Date -Format o) $Message"
}

function Capture-Screenshot([string]$Path) {
  try {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    $Bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $Bitmap = [System.Drawing.Bitmap]::new($Bounds.Width, $Bounds.Height)
    $Graphics = [System.Drawing.Graphics]::FromImage($Bitmap)
    $Graphics.CopyFromScreen($Bounds.Location, [System.Drawing.Point]::Empty, $Bounds.Size)
    $Bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $Graphics.Dispose()
    $Bitmap.Dispose()
    Log "screenshot captured width=$($Bounds.Width) height=$($Bounds.Height) file=$Path bytes=$((Get-Item $Path).Length)"
  } catch {
    Log "screenshot unavailable: $($_.Exception.Message)"
  }
}

function Fail([string]$Message) {
  Log "ERROR: $Message"
  Capture-Screenshot $Screenshot
  $Message | Set-Content -Encoding ASCII $ErrorFile
  exit 1
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

function Find-ElementByName([System.Windows.Automation.AutomationElement]$Window, [string]$Name) {
  $Condition = [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::NameProperty,
    $Name
  )
  return $Window.FindFirst(
    [System.Windows.Automation.TreeScope]::Descendants,
    $Condition
  )
}

function Find-ButtonByName([System.Windows.Automation.AutomationElement]$Window, [string]$Name) {
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

function Require-Element([System.Windows.Automation.AutomationElement]$Window, [string]$Name) {
  $Element = Find-ElementByName $Window $Name
  if (-not $Element) {
    Fail "missing UI Automation element named '$Name'"
  }
  return $Element
}

function Invoke-Button([System.Windows.Automation.AutomationElement]$Window, [string]$Name) {
  $Button = Find-ButtonByName $Window $Name
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

function Wait-ShellReady {
  for ($i = 0; $i -lt 40; $i++) {
    if ((Test-Path $ShellTrace) -and
        (Select-String -Path $ShellTrace -Pattern "initial RefreshAsync completed" -Quiet)) {
      Log "shell initial refresh completed"
      return
    }
    Start-Sleep -Milliseconds 250
  }
  Fail "Windows shell did not finish initial refresh before UI assertions"
}

try {
  Log "interactive GUI smoke worker started user=$env:USERNAME session=$([System.Diagnostics.Process]::GetCurrentProcess().SessionId)"
  Get-Process -Name "IrisDrive" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500

  $env:IRIS_DRIVE_CLI = $Idrive
  $env:IRIS_DRIVE_CONFIG_DIR = $ConfigDir
  $env:IRIS_DRIVE_EXTERNAL_DAEMON = "true"
  $env:IRIS_DRIVE_WINDOWS_SHELL_TRACE = $ShellTrace
  $Started = Start-Process -FilePath $Exe -WorkingDirectory $PublishDir -PassThru
  Log "launched IrisDrive pid=$($Started.Id)"

  $Process = $null
  for ($i = 0; $i -lt 50; $i++) {
    $Process = Current-IrisWindowProcess
    if ($Process) { break }
    Start-Sleep -Milliseconds 500
  }
  if (-not $Process) {
    Fail "Windows WPF shell did not expose a visible Iris Drive window from the interactive worker"
  }

  Log "visible IrisDrive window pid=$($Process.Id) handle=$($Process.MainWindowHandle)"
  [void][IrisDriveSmoke.NativeMethods]::ShowWindow($Process.MainWindowHandle, 9)
  [void][IrisDriveSmoke.NativeMethods]::SetForegroundWindow($Process.MainWindowHandle)
  Wait-ShellReady

  Add-Type -AssemblyName UIAutomationClient
  Add-Type -AssemblyName UIAutomationTypes
  $Window = [System.Windows.Automation.AutomationElement]::FromHandle($Process.MainWindowHandle)
  if (-not $Window) {
    Fail "UI Automation could not attach to Iris Drive window"
  }

  [void](Require-Element $Window "My Drive")
  [void](Require-Element $Window "Open Drive Folder")
  [void](Require-Element $Window "Files")
  [void](Require-Element $Window "Storage")
  [void](Require-Element $Window "Devices")
  Invoke-Button $Window "Devices"
  [void](Require-Element $Window "Linked Devices")
  Invoke-Button $Window "Network"
  [void](Require-Element $Window "FIPS")
  [void](Require-Element $Window "Relays")
  Invoke-Button $Window "My Drive"

  $Status = & $Idrive --config-dir $ConfigDir status | ConvertFrom-Json
  if (-not $Status.initialized) {
    Fail "Windows GUI smoke expected an initialized app profile"
  }
  $Authorized = 0
  if ($Status.summary -and $Status.summary.authorized_app_key_count) {
    $Authorized = [int]$Status.summary.authorized_app_key_count
  } elseif ($Status.summary -and $Status.summary.authorized_device_count) {
    $Authorized = [int]$Status.summary.authorized_device_count
  } elseif ($Status.network -and $Status.network.authorized_app_key_count) {
    $Authorized = [int]$Status.network.authorized_app_key_count
  } elseif ($Status.network -and $Status.network.authorized_device_count) {
    $Authorized = [int]$Status.network.authorized_device_count
  }
  if ($Authorized -lt 1) {
    Fail "Windows status has no authorized app keys"
  }

  Capture-Screenshot $Screenshot
  "WINDOWS_GUI_SMOKE_OK" | Set-Content -Encoding ASCII $ResultFile
  Log "WINDOWS_GUI_SMOKE_OK"
} catch {
  Fail (($_ | Out-String).Trim())
} finally {
  if ($Process -and -not $Process.HasExited) {
    Log "stopping IrisDrive pid=$($Process.Id)"
    Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
  } elseif ($Started -and -not $Started.HasExited) {
    Log "stopping IrisDrive pid=$($Started.Id)"
    Stop-Process -Id $Started.Id -Force -ErrorAction SilentlyContinue
  }
}
'@ | Set-Content -Encoding ASCII $WorkerScript

  try {
    $ActionArgs = "-NoProfile -ExecutionPolicy Bypass -File `"$WorkerScript`" -PublishDir `"$PublishDir`" -Exe `"$Exe`" -Idrive `"$Idrive`" -ConfigDir `"$ConfigDir`" -ShellTrace `"$ShellTrace`" -ResultFile `"$ResultFile`" -ErrorFile `"$ErrorFile`" -WorkerLog `"$WorkerLog`" -Screenshot `"$InteractiveScreenshot`""
    Write-SmokeLog "launching Windows WPF shell in interactive task"
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    $Action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument $ActionArgs -WorkingDirectory $PublishDir
    $Trigger = New-ScheduledTaskTrigger -Once -At ((Get-Date).AddMinutes(1))
    $Principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
    Register-ScheduledTask -TaskName $TaskName -Action $Action -Trigger $Trigger -Principal $Principal -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName

    for ($i = 0; $i -lt 90; $i++) {
      if ((Test-Path $ResultFile) -or (Test-Path $ErrorFile)) {
        break
      }
      Start-Sleep -Milliseconds 500
    }

    $Task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    $TaskInfo = Get-ScheduledTaskInfo -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($Task -or $TaskInfo) {
      Write-SmokeLog "Windows GUI interactive task state=$($Task.State) last_result=$($TaskInfo.LastTaskResult)"
    }
    if (Test-Path $WorkerLog) {
      Get-Content $WorkerLog -Tail 120 | ForEach-Object {
        [Console]::Error.WriteLine("[windows-gui-smoke] worker $_")
      }
    }
    if (Test-Path $InteractiveScreenshot) {
      $Bytes = (Get-Item $InteractiveScreenshot).Length
      Write-SmokeLog "interactive screenshot saved to $InteractiveScreenshot bytes=$Bytes"
      $script:LastWindowsGuiPreflightScreenshot = $InteractiveScreenshot
    }
    if (Test-Path $ErrorFile) {
      Fail ((Get-Content $ErrorFile -Raw).Trim())
    }
    if (-not (Test-Path $ResultFile)) {
      Fail "Windows GUI interactive smoke task did not finish"
    }
    (Get-Content $ResultFile -Raw).Trim()
  } finally {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
  }
}

Invoke-InteractiveGuiSmoke
REMOTE_PS
  } | ssh "$remote" "$(windows_powershell_command_for "$remote")"
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
