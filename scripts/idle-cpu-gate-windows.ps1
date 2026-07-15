param(
  [int]$WarmupSecs = -1,
  [int]$DurationSecs = -1,
  [int]$IntervalSecs = -1
)

$ErrorActionPreference = 'Stop'

function Get-EnvInt([string]$Name, [int]$Default) {
  $value = [Environment]::GetEnvironmentVariable($Name)
  $parsed = 0
  if (-not [string]::IsNullOrWhiteSpace($value) -and [int]::TryParse($value, [ref]$parsed)) {
    return $parsed
  }
  return $Default
}

function Get-EnvDouble([string]$Name, [double]$Default) {
  $value = [Environment]::GetEnvironmentVariable($Name)
  $parsed = 0.0
  if (-not [string]::IsNullOrWhiteSpace($value) -and [double]::TryParse($value, [ref]$parsed)) {
    return $parsed
  }
  return $Default
}

function Split-Roles([string]$Value, [string[]]$Default) {
  if ([string]::IsNullOrWhiteSpace($Value)) {
    return $Default
  }
  return @($Value -split '[,\s]+' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

function Classify-IrisDriveProcess([string]$Name, [string]$CommandLine) {
  $command = if ($CommandLine) { $CommandLine } else { $Name }
  $lowerName = $Name.ToLowerInvariant()
  $lowerCommand = $command.ToLowerInvariant()
  if (($lowerName -eq 'idrive' -or $lowerName -eq 'idrive.exe') -and $lowerCommand -match '\sdaemon(\s|$)') {
    return 'daemon'
  }
  if ($lowerName -match 'irisdrive|iris-drive|iris drive') {
    return 'app'
  }
  if ($lowerCommand -match 'iris drive' -and $lowerCommand -notmatch '\sdaemon(\s|$)') {
    return 'app'
  }
  return $null
}

if ($WarmupSecs -lt 0) { $WarmupSecs = Get-EnvInt 'IRIS_DRIVE_IDLE_CPU_WARMUP_SECS' 30 }
if ($DurationSecs -lt 0) { $DurationSecs = Get-EnvInt 'IRIS_DRIVE_IDLE_CPU_DURATION_SECS' 60 }
if ($IntervalSecs -lt 0) { $IntervalSecs = Get-EnvInt 'IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS' 5 }

$thresholds = @{
  app = Get-EnvDouble 'IRIS_DRIVE_IDLE_CPU_APP_MAX' 5.0
  daemon = Get-EnvDouble 'IRIS_DRIVE_IDLE_CPU_DAEMON_MAX' 10.0
  provider = Get-EnvDouble 'IRIS_DRIVE_IDLE_CPU_PROVIDER_MAX' 3.0
}
$required = Split-Roles ([Environment]::GetEnvironmentVariable('IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES')) @('app', 'daemon')
$commandMatch = [Environment]::GetEnvironmentVariable('IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH')
if ($null -eq $commandMatch) { $commandMatch = '' }
$commandMatch = $commandMatch.Trim()

[Console]::Error.WriteLine("[idle-cpu] warmup ${WarmupSecs}s, sample ${DurationSecs}s every ${IntervalSecs}s")
Start-Sleep -Seconds $WarmupSecs

$samples = @{}
$seen = @{}
foreach ($role in $thresholds.Keys) {
  $samples[$role] = New-Object System.Collections.Generic.List[double]
  $seen[$role] = New-Object 'System.Collections.Generic.HashSet[int]'
}

$deadline = (Get-Date).AddSeconds($DurationSecs)
while ($true) {
  $processes = @{}
  Get-CimInstance Win32_Process | ForEach-Object {
    $command = if ($_.CommandLine) { $_.CommandLine } else { $_.Name }
    if ($commandMatch.Length -eq 0 -or $command.IndexOf($commandMatch, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
      $role = Classify-IrisDriveProcess $_.Name $command
      if ($role) {
        $processes[[int]$_.ProcessId] = @{ role = $role; command = $command }
      }
    }
  }

  $totals = @{}
  foreach ($role in $thresholds.Keys) { $totals[$role] = 0.0 }
  Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | ForEach-Object {
    $processId = [int]$_.IDProcess
    if ($processes.ContainsKey($processId)) {
      $role = $processes[$processId].role
      $totals[$role] = [double]$totals[$role] + [double]$_.PercentProcessorTime
      [void]$seen[$role].Add($processId)
    }
  }
  foreach ($role in $totals.Keys) {
    if ($totals[$role] -gt 0 -or $seen[$role].Count -gt 0) {
      $samples[$role].Add([double]$totals[$role])
    }
  }

  if ((Get-Date) -ge $deadline) { break }
  Start-Sleep -Seconds $IntervalSecs
}

$summaryRoles = @{}
$failures = New-Object System.Collections.Generic.List[string]
$sampleRoles = @($samples.Keys | ForEach-Object { [string]$_ })
$allRoles = @((@($required) + $sampleRoles) | Sort-Object -Unique)
foreach ($role in $allRoles) {
  $values = @($samples[$role])
  if ($values.Count -eq 0) {
    if ($required -contains $role) {
      $failures.Add("${role}: required process role was not observed")
    }
    continue
  }
  $sum = 0.0
  $peak = 0.0
  foreach ($value in $values) {
    $sum += [double]$value
    if ([double]$value -gt $peak) { $peak = [double]$value }
  }
  $avg = $sum / $values.Count
  $limit = if ($thresholds.ContainsKey($role)) { [double]$thresholds[$role] } else { [double]$thresholds.app }
  $summaryRoles[$role] = @{
    avg_cpu = [Math]::Round($avg, 2)
    peak_cpu = [Math]::Round($peak, 2)
    samples = $values.Count
    pids = @($seen[$role] | Sort-Object)
    limit = $limit
  }
  if ($avg -gt $limit) {
    $failures.Add("${role}: avg CPU $([Math]::Round($avg, 2))% > $([Math]::Round($limit, 2))%")
  }
}

@{
  platform = 'windows'
  required_roles = @($required | Sort-Object)
  roles = $summaryRoles
} | ConvertTo-Json -Depth 6

if ($failures.Count -gt 0) {
  foreach ($failure in $failures) {
    [Console]::Error.WriteLine("[idle-cpu] FAIL: $failure")
  }
  exit 1
}
[Console]::Error.WriteLine('[idle-cpu] OK')
