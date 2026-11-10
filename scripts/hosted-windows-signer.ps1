param(
    [Parameter(Mandatory)][string]$Product,
    [Parameter(Mandatory)][string]$Evidence,
    [switch]$InteractiveWorker
)
$ErrorActionPreference = 'Stop'
$exe = Join-Path $Product 'IrisDrive.exe'
$resultFile = Join-Path $Evidence 'window-result.json'
$session = [Diagnostics.Process]::GetCurrentProcess().SessionId

# A service runner can use a signed-in interactive token on its own hosted VM.
if (!$InteractiveWorker -and ($session -eq 0 -or ![Environment]::UserInteractive)) {
    $taskName = 'IrisDriveSigner-' + [guid]::NewGuid().ToString('N')
    try {
        $arguments = "-NoProfile -STA -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Product `"$Product`" -Evidence `"$Evidence`" -InteractiveWorker"
        $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $arguments
        $principal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive
        Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal | Out-Null
        Start-ScheduledTask -TaskName $taskName
        $deadline = (Get-Date).AddSeconds(160)
        while (!(Test-Path $resultFile) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 400 }
        if (!(Test-Path $resultFile)) {
            $last = (Get-ScheduledTaskInfo $taskName).LastTaskResult
            throw "No interactive worker result. Runner session $session. Task result $last."
        }
        $result = Get-Content -Raw $resultFile | ConvertFrom-Json
        if (!$result.passed) { throw $result.error }
    } catch {
        $_.Exception.Message | Set-Content (Join-Path $Evidence 'dispatch-error.txt')
        throw
    } finally {
        if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
            Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
        }
    }
    return
}

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class SignerWindow {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool GetUserObjectInformation(IntPtr desktop, int index, StringBuilder name, uint size, out uint needed);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
}
'@

function Save-AppWindow([IntPtr]$handle, [string]$name) {
    $rect = New-Object 'SignerWindow+Rect'
    if (![SignerWindow]::GetWindowRect($handle, [ref]$rect)) { throw 'Cannot measure app window' }
    $bitmap = New-Object Drawing.Bitmap ($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top)
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try {
                if (![SignerWindow]::PrintWindow($handle, $dc, 2)) { throw 'App window capture failed' }
            } finally { $graphics.ReleaseHdc($dc) }
        } finally { $graphics.Dispose() }
        $bitmap.Save((Join-Path $Evidence $name), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
}

function Find-Control($window, $type, [string]$label, [string]$id) {
    $items = $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
    foreach ($item in $items) {
        if ($item.Current.IsOffscreen -or $item.Current.ControlType -ne $type) { continue }
        if ($id -and $item.Current.AutomationId -ne $id) { continue }
        if (!$label -or $item.Current.Name.Trim() -eq $label -or $item.Current.Name.Trim().EndsWith(" $label")) { return $item }
        $text = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), $label
        if ($item.FindFirst([Windows.Automation.TreeScope]::Descendants, $text)) { return $item }
    }
}

function Wait-Control($window, $type, [string]$label, [string]$id) {
    $deadline = (Get-Date).AddSeconds(15)
    do {
        $control = Find-Control $window $type $label $id
        if ($control) { return $control }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    throw "Missing visible control ($type, $label, $id)"
}

$profile = Join-Path ([IO.Path]::GetTempPath()) ('iris-drive-signer-' + [guid]::NewGuid().ToString('N'))
$receipt = [ordered]@{ session = $session; interactive = [Environment]::UserInteractive; executableSha256 = (Get-FileHash $exe).Hash; stage = 'input desktop'; passed = $false; closed = $false }
$app = $null
$handle = [IntPtr]::Zero
try {
    if (!$receipt.interactive -or $session -eq 0) { throw 'The hosted worker has no interactive Windows session' }
    [void][SignerWindow]::SetProcessDPIAware()
    $desktop = [SignerWindow]::OpenInputDesktop(0, $false, 1)
    if ($desktop -eq [IntPtr]::Zero) { throw 'Cannot open the current Windows input desktop' }
    try {
        $name = New-Object Text.StringBuilder 256
        [uint32]$needed = 0
        if (![SignerWindow]::GetUserObjectInformation($desktop, 2, $name, 512, [ref]$needed) -or $name.ToString() -ne 'Default') {
            throw 'The default user desktop is not the active input desktop'
        }
    } finally { [void][SignerWindow]::CloseDesktop($desktop) }
    $receipt.stage = 'visible window'
    New-Item -Type Directory -Path $profile | Out-Null
    $env:IRIS_DRIVE_CONFIG_DIR = $profile
    $env:IRIS_DRIVE_WINDOWS_CLOUD_ROOT = 'off'
    $app = Start-Process -FilePath $exe -WorkingDirectory $Product -PassThru
    $deadline = (Get-Date).AddSeconds(45)
    do {
        $app.Refresh()
        if ($app.HasExited) { throw "App exited before opening a window ($($app.ExitCode))" }
        $handle = $app.MainWindowHandle
        if ($handle -ne [IntPtr]::Zero -and [SignerWindow]::IsWindowVisible($handle)) { break }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    [uint32]$owner = 0
    [void][SignerWindow]::GetWindowThreadProcessId($handle,[ref]$owner)
    if ($owner -ne $app.Id -or $app.SessionId -ne $session -or ![SignerWindow]::IsWindowVisible($handle)) { throw 'No visible WPF window owned by the app in this session' }
    $window = [Windows.Automation.AutomationElement]::FromHandle($handle)
    $button = [Windows.Automation.ControlType]::Button
    $invoke = [Windows.Automation.InvokePattern]::Pattern
    $receipt.stage = 'existing account choice'
    $entry = Wait-Control $window $button 'Sign in' ''
    if (Find-Control $window $button 'Sign in with remote signer' '') { throw 'Signer appears on the welcome screen' }
    $entry.GetCurrentPattern($invoke).Invoke()
    $receipt.stage = 'remote signer choice'
    $signer = Wait-Control $window $button 'Sign in with remote signer' ''
    $signer.GetCurrentPattern($invoke).Invoke()
    $receipt.stage = 'signer form'
    $null = Wait-Control $window ([Windows.Automation.ControlType]::Text) 'Signer connection link' 'RestoreInputLabel'
    $null = Wait-Control $window ([Windows.Automation.ControlType]::Edit) '' 'RestoreSecretBox'
    $null = Wait-Control $window $button 'Connect signer' 'RestoreSubmitButton'
    Save-AppWindow $handle 'remote-signer.png'
    $receipt.stage = 'completed'
    $receipt.passed = $true
} catch {
    $receipt.error = $_.Exception.Message
    if ($handle -ne [IntPtr]::Zero) { try { Save-AppWindow $handle 'failure-window.png' } catch {} }
} finally {
    # Closing the main window can minimize to tray. Only terminate this published product.
    if ($app -and !$app.HasExited) { $null = $app.CloseMainWindow(); $null = $app.WaitForExit(1500) }
    $prefix = [IO.Path]::GetFullPath($Product).TrimEnd('\') + '\'
    $owned = { Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) } }
    foreach ($process in (& $owned)) { Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue }
    $receipt.closed = @(& $owned).Count -eq 0
    if (!$receipt.closed) { $receipt.error = 'Product processes remained after closure'; $receipt.passed = $false }
    if (Test-Path $profile) { Remove-Item -Recurse -Force $profile }
    $receipt | ConvertTo-Json | Set-Content "$resultFile.tmp"
    Move-Item "$resultFile.tmp" $resultFile
}
if (!$receipt.passed) { throw $receipt.error }
