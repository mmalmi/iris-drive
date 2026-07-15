param(
  [string]$Profile = "debug",
  [string]$OverrideIdrive = "",
  [string]$RebuildIdrive = "1"
)

$ErrorActionPreference = 'Stop'
$repo = Join-Path $HOME 'src\iris-drive'
$repoIdrive = Join-Path $repo (Join-Path (Join-Path 'target' $Profile) 'idrive.exe')
$cargoProfileArgs = @()
if ($Profile -eq 'release') { $cargoProfileArgs += '--release' }

function Test-IrisDriveCli([string]$candidate) {
  if ([string]::IsNullOrWhiteSpace($candidate) -or -not (Test-Path -LiteralPath $candidate)) {
    return $false
  }
  & $candidate app-keys --help *> $null
  return $LASTEXITCODE -eq 0
}

$idrive = $OverrideIdrive
if ([string]::IsNullOrWhiteSpace($idrive) -and $RebuildIdrive -ne '0' -and (Test-Path -LiteralPath (Join-Path $repo 'Cargo.toml'))) {
  $cargo = Get-Command cargo -ErrorAction SilentlyContinue
  if ($cargo) {
    Push-Location $repo
    cargo build -q @cargoProfileArgs -p idrive --bin idrive
    Pop-Location
    $idrive = $repoIdrive
  }
}

if ([string]::IsNullOrWhiteSpace($idrive) -and (Test-IrisDriveCli $repoIdrive)) {
  $idrive = $repoIdrive
}
if ([string]::IsNullOrWhiteSpace($idrive)) {
  if (Test-Path -LiteralPath (Join-Path $repo 'Cargo.toml')) {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargo) {
      Push-Location $repo
      cargo build -q @cargoProfileArgs -p idrive --bin idrive
      Pop-Location
      $idrive = $repoIdrive
    }
  }
}
if ([string]::IsNullOrWhiteSpace($idrive) -or -not (Test-IrisDriveCli $idrive)) {
  $idrive = $repoIdrive
}
if ($Profile -eq 'debug' -and -not (Test-IrisDriveCli $idrive)) {
  $idrive = Join-Path $HOME '.cargo\bin\idrive.exe'
  if (-not (Test-Path -LiteralPath $idrive)) {
    $cmd = Get-Command idrive.exe -ErrorAction SilentlyContinue
    if ($cmd) { $idrive = $cmd.Source }
  }
}
if (-not (Test-IrisDriveCli $idrive)) {
  if (Test-Path -LiteralPath (Join-Path $repo 'Cargo.toml')) {
    Push-Location $repo
    cargo build -q @cargoProfileArgs -p idrive --bin idrive
    Pop-Location
    $idrive = $repoIdrive
  }
}
if (-not (Test-IrisDriveCli $idrive)) {
  throw "current idrive.exe with app-keys support not found"
}

Write-Output $idrive
