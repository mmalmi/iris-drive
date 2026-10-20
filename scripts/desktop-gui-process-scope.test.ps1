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
function Start-Process {
  [CmdletBinding()]
  param([string]$FilePath, [string[]]$ArgumentList, [string]$WorkingDirectory, [switch]$PassThru)
  $script:Launches += [pscustomobject]@{
    Exe = $FilePath; Arguments = $ArgumentList; Directory = $WorkingDirectory
    Config = $env:IRIS_DRIVE_CONFIG_DIR; CloudRoot = $env:IRIS_DRIVE_WINDOWS_CLOUD_ROOT
  }
  [pscustomobject]@{ Id = 999; HasExited = $false }
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
$EnvironmentSetup = @($Main[0].Body.Statements | Where-Object {
  $_ -is [System.Management.Automation.Language.AssignmentStatementAst] -and
  $_.Left -is [System.Management.Automation.Language.VariableExpressionAst] -and
  $_.Left.VariablePath.UserPath.StartsWith('env:')
})
$Launch = @($Main[0].Body.Statements | Where-Object {
  $_ -is [System.Management.Automation.Language.IfStatementAst] -and
  $_.Extent.Text -match '\bStart-Process\b'
})
if ($Launch.Count -ne 1) { throw 'Expected the actual worker launch branch' }
foreach ($Statement in @($EnvironmentSetup) + @($Launch)) {
  if ($Statement.Extent.StartOffset -ge $Launch[0].Extent.EndOffset) {
    throw 'Worker environment is configured after launch'
  }
  foreach ($Command in $Statement.FindAll({ param($Node)
    $Node -is [System.Management.Automation.Language.CommandAst]
  }, $true)) {
    if ($Command.GetCommandName() -notin @('Join-Path', 'Start-Process')) {
      throw 'Refusing a worker launch action that could bypass mocks'
    }
  }
}
$EnvNames = @($EnvironmentSetup | ForEach-Object { $_.Left.VariablePath.UserPath.Substring(4) }) +
  @('IRIS_DRIVE_WINDOWS_CLOUD_ROOT') | Select-Object -Unique
$SavedEnv = @{}
foreach ($Name in $EnvNames) { $SavedEnv[$Name] = [Environment]::GetEnvironmentVariable($Name) }
try {
  $Idrive = 'C:\fixtures\selected\idrive.exe'
  $PublishDir = 'C:\fixtures\selected'
  $ShellTrace = 'C:\fixtures\selected\shell.log'
  foreach ($ConfigDir in @('C:\fixtures\profile with spaces', "C:\fixtures\other's profile")) {
    foreach ($InheritedRoot in @($null, 'C:\fixtures\unowned cloud root')) {
      foreach ($WithLink in @($false, $true)) {
        [Environment]::SetEnvironmentVariable('IRIS_DRIVE_WINDOWS_CLOUD_ROOT', $InheritedRoot)
        $StartArguments = if ($WithLink) { @('https://drive.iris.to/device-test') } else { @() }
        $script:Launches = @()
        foreach ($Statement in $EnvironmentSetup) {
          & ([scriptblock]::Create($Statement.Extent.Text))
        }
        & ([scriptblock]::Create($Launch[0].Extent.Text))
        if ($script:Launches.Count -ne 1 -or $script:Launches[0].Exe -ne $Exe -or
            $script:Launches[0].Config -ne $ConfigDir -or
            $script:Launches[0].CloudRoot -ne (Join-Path $ConfigDir 'cloud-root')) {
          throw 'Actual worker launch escaped the selected config cloud root'
        }
      }
    }
  }
} finally {
  foreach ($Name in $EnvNames) { [Environment]::SetEnvironmentVariable($Name, $SavedEnv[$Name]) }
}

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
Write-Output 'WINDOWS_GUI_PROCESS_SCOPE_OK (actual worker launch environment and helpers/actions; only mocked processes)'
