param([string]$ResultPath)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$root = Split-Path -Parent $PSScriptRoot
$stage = Join-Path ([IO.Path]::GetTempPath()) ('drive-runtime-inputs-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $stage | Out-Null
foreach ($name in @('IrisDrive.Windows.csproj', 'packages.lock.json', 'sdk-packages.json')) {
  Copy-Item (Join-Path $root "windows\$name") (Join-Path $stage $name)
}
Copy-Item (Join-Path $PSScriptRoot 'windows-build-inputs.ps1') (Join-Path $stage 'windows-build-inputs.ps1')
$project = Join-Path $stage 'IrisDrive.Windows.csproj'
$assets = Join-Path $stage 'obj\project.assets.json'
$pinsPath = Join-Path $stage 'sdk-packages.json'
$before = (Get-PSDrive C).Free
if ($before -lt 3GB) { throw 'Guest below 3GiB floor' }
if (Get-Process dotnet,MSBuild,cargo,rustc -ErrorAction SilentlyContinue) { throw 'Conflicting writer' }
$started = Get-Date
$original = [IO.File]::ReadAllBytes($project)
$checks = @()
$failure = $null
. (Join-Path $stage 'windows-build-inputs.ps1')
function Restore-Checked([string]$label, [bool]$expectMismatch) {
  $remaining = 175000 - ((Get-Date) - $started).TotalMilliseconds
  if ($remaining -le 0) { throw 'Overall restore bound reached' }
  $out = Join-Path $stage "$label.log"
  $err = Join-Path $stage "$label.err"
  $process = Start-Process dotnet -ArgumentList @('restore', $project, '--runtime', 'win-x64', '--locked-mode') -PassThru -RedirectStandardOutput $out -RedirectStandardError $err -WindowStyle Hidden
  if (-not $process.WaitForExit([int][Math]::Min(45000, $remaining))) {
    taskkill /PID $process.Id /T /F | Out-Null
    throw "Restore $label timed out"
  }
  $process.Refresh()
  $text = [IO.File]::ReadAllText($out) + [IO.File]::ReadAllText($err)
  if ($expectMismatch) {
    if ($text -notmatch 'NU1004') { throw "Expected lock rejection: $text" }
  } elseif ($process.ExitCode -ne 0) { throw "Restore failed: $text" }
  if ((Get-PSDrive C).Free -lt 3GB -or $before - (Get-PSDrive C).Free -gt 256MB) { throw 'Space bound reached' }
}
try {
  Restore-Checked 'locked' $false
  $checks += 'locked restore'
  Assert-PinnedDownloadPackages -AssetsPath $assets -PinsPath $pinsPath
  $checks += 'real archive SHA512'
  $pins = [IO.File]::ReadAllText($pinsPath) | ConvertFrom-Json
  $pins[0].sha512 = '0' * 128
  $badPins = Join-Path $stage 'tampered-pins.json'
  [IO.File]::WriteAllText($badPins, ($pins | ConvertTo-Json -Depth 3))
  $rejected = $false
  try { Assert-PinnedDownloadPackages -AssetsPath $assets -PinsPath $badPins } catch {
    if ($_.Exception.Message -notmatch 'content hash mismatch') { throw }
    $rejected = $true
  }
  if (-not $rejected) { throw 'Tampered hash accepted' }
  $checks += 'tampered SHA512 rejected'
  $text = [Text.Encoding]::UTF8.GetString($original).Replace('</Project>', '<ItemGroup><PackageReference Include="Newtonsoft.Json" Version="13.0.3" /></ItemGroup></Project>')
  [IO.File]::WriteAllText($project, $text)
  Restore-Checked 'mismatch' $true
  $checks += 'changed dependency rejected NU1004'
} catch {
  $failure = $_.Exception.Message
} finally {
  [IO.File]::WriteAllBytes($project, $original)
}
if (-not $failure) {
  try {
    Restore-Checked 'final' $false
    Assert-PinnedDownloadPackages -AssetsPath $assets -PinsPath $pinsPath
    $checks += 'final locked restore and hashes'
  } catch { $failure = $_.Exception.Message }
}
$after = (Get-PSDrive C).Free
$logs = @{}
foreach ($name in @('locked', 'mismatch', 'final')) {
  $path = Join-Path $stage "$name.log"
  if (Test-Path $path) { $logs[$name] = [IO.File]::ReadAllText($path) }
}
$result = @{start=$started.ToUniversalTime().ToString('o');terminal=(Get-Date).ToUniversalTime().ToString('o');checks=$checks;failure=$failure;freeBefore=$before;freeAfter=$after;growth=$before-$after;stageBytes=(Get-ChildItem $stage -Recurse -File|Measure-Object Length -Sum).Sum;helperSha256=(Get-FileHash (Join-Path $stage 'windows-build-inputs.ps1') -Algorithm SHA256).Hash;pinsSha256=(Get-FileHash $pinsPath -Algorithm SHA256).Hash;projectSha256=(Get-FileHash $project -Algorithm SHA256).Hash;lockSha256=(Get-FileHash (Join-Path $stage 'packages.lock.json') -Algorithm SHA256).Hash;logs=$logs;writers=@(Get-Process dotnet,MSBuild,cargo,rustc -ErrorAction SilentlyContinue|Select-Object Id,ProcessName)}
if ($ResultPath) { [IO.File]::WriteAllText($ResultPath, ($result|ConvertTo-Json -Depth 5)) }
$result|ConvertTo-Json -Depth 5
# Remove only this test's fresh disposable project, after every restore has exited.
if ($result.writers.Count -eq 0) { Remove-Item -LiteralPath $stage -Recurse -Force }
if ($failure) { exit 1 }
