param([string]$SmokeScript = (Join-Path $PSScriptRoot 'desktop-gui-smoke.sh'))

$ErrorActionPreference = 'Stop'
$Source = Get-Content -Raw $SmokeScript
$WorkerPattern = "(?ms)^@'\r?\n(param\(.*?)^'@ \| Set-Content -Encoding ASCII " + [regex]::Escape('$WorkerScript')
$Worker = [regex]::Match($Source, $WorkerPattern).Groups[1].Value
if (-not $Worker) { throw 'Could not extract the actual Windows GUI worker' }
$Tokens = $null
$ParseErrors = $null
$Ast = [System.Management.Automation.Language.Parser]::ParseInput($Worker, [ref]$Tokens, [ref]$ParseErrors)
if ($ParseErrors) { throw ($ParseErrors | Out-String) }

# These functions shadow the operating-system cmdlets for every tested action.
$script:Stopped = @()
function Get-Process {
  [CmdletBinding()]
  param([string]$Name, [int]$Id)
  $script:Processes | Where-Object {
    (-not $Name -or $_.ProcessName -eq $Name) -and (-not $Id -or $_.Id -eq $Id)
  }
}
function Stop-Process {
  [CmdletBinding()]
  param([Parameter(ValueFromPipeline = $true)]$InputObject, [int]$Id, [switch]$Force)
  process {
    if ($InputObject) { $script:Stopped += $InputObject.Id }
    elseif ($Id) { $script:Stopped += $Id }
    else { throw 'Unexpected stop invocation' }
  }
}
function Log([string]$Message) {}
Add-Type -TypeDefinition @'
namespace IrisDriveSmoke {
  public static class NativeMethods {
    public static bool IsWindowVisible(System.IntPtr handle) { return handle.ToInt64() > 0; }
  }
}
'@

$Helpers = @('Get-SmokeIrisDriveProcesses', 'Stop-SmokeIrisDriveProcesses', 'Current-IrisWindowProcess')
foreach ($Function in $Ast.EndBlock.Statements | Where-Object {
  $_ -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $_.Name -in $Helpers
}) {
  if ($Function.Extent.Text -match '(?i)\w+\\(?:Get|Stop)-Process|\.Kill\(') {
    throw 'Refusing an action that could bypass process mocks'
  }
  Invoke-Expression $Function.Extent.Text
}
$Main = @($Ast.EndBlock.Statements | Where-Object { $_ -is [System.Management.Automation.Language.TryStatementAst] })
if ($Main.Count -ne 1) { throw 'Expected exactly one worker main block' }
$InitialCleanup = @($Main[0].Body.Statements | Where-Object {
  $_.Extent.Text -match '\bStop-(?:Process|SmokeIrisDriveProcesses)\b'
})
if ($InitialCleanup.Count -ne 1) { throw 'Expected exactly one startup process cleanup' }
$Finally = ($Main[0].Finally.Statements | ForEach-Object { $_.Extent.Text }) -join "`n"
if (($InitialCleanup[0].Extent.Text + $Finally) -match '(?i)\w+\\(?:Get|Stop)-Process|\.Kill\(') {
  throw 'Refusing cleanup that could bypass process mocks'
}

$Exe = 'C:\fixtures\selected\IrisDrive.exe'
function Candidate([int]$Id, [string]$Path, [int]$Handle, [string]$Name = 'IrisDrive') {
  [pscustomobject]@{ Id = $Id; Path = $Path; ProcessName = $Name; MainWindowHandle = [IntPtr]$Handle;
    StartTime = [DateTime]'2026-01-01'; HasExited = $false }
}
$script:Processes = @(
  (Candidate 101 $Exe 1),
  (Candidate 102 $Exe 0),
  (Candidate 103 'c:\FIXTURES\SELECTED\IrisDrive.exe' 2),
  (Candidate 201 'C:\fixtures\other\IrisDrive.exe' 3),
  (Candidate 202 'C:\fixtures\other\IrisDrive.exe' 0),
  (Candidate 203 'C:\fixtures\selected-other\IrisDrive.exe' 4),
  (Candidate 204 '' 5),
  (Candidate 205 'C:\fixtures\other\idrive.exe' 0 'idrive')
)
$Unreadable = Candidate 206 '' 6
$Unreadable.PSObject.Properties.Remove('Path')
$Unreadable | Add-Member -MemberType ScriptProperty -Name Path -Value { throw 'Path access denied' }
$script:Processes += $Unreadable

& ([scriptblock]::Create($InitialCleanup[0].Extent.Text))
if (($script:Stopped -join ',') -ne '101,102,103') {
  throw "Startup cleanup stopped foreign or missed owned processes: $($script:Stopped -join ',')"
}
foreach ($Id in @(201, 203, 204, 206)) {
  $Started = [pscustomobject]@{ Id = $Id; HasExited = $false }
  if (Current-IrisWindowProcess) { throw "Selected a foreign/unverifiable window: $Id" }
}
$Started = [pscustomobject]@{ Id = 101; HasExited = $false }
if ((Current-IrisWindowProcess).Id -ne 101) { throw 'Did not select the launched owned window' }
$Started = [pscustomobject]@{ Id = 102; HasExited = $false }
if (Current-IrisWindowProcess) { throw 'Selected a hidden owned window' }

foreach ($Selected in @($script:Processes[0], $script:Processes[3])) {
  $script:Stopped = @()
  $Process = $Selected
  $Started = $null
  & ([scriptblock]::Create($Finally))
  $Expected = if ($Selected.Id -eq 101) { '101' } else { '' }
  if (($script:Stopped -join ',') -ne $Expected) { throw 'Final cleanup escaped executable ownership' }
  $script:Stopped = @()
  $Process = $null
  $Started = $Selected
  & ([scriptblock]::Create($Finally))
  if (($script:Stopped -join ',') -ne $Expected) { throw 'Launch fallback cleanup escaped executable ownership' }
}
Write-Output 'WINDOWS_GUI_PROCESS_SCOPE_OK (actual worker helpers/actions; only mocked processes)'
