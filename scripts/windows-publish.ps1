param(
  [ValidateSet("Debug", "Release")]
  [string]$Configuration = "Debug",

  [string]$Runtime = "win-x64",

  [switch]$DesktopShortcut,

  [switch]$SkipCliBuild,

  [switch]$AllowLockfileUpdate,

  [switch]$StopRunningApp,

  [switch]$Installer,

  [string]$Tag,

  [string]$OutputDir
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Project = Join-Path $Root "windows\IrisDrive.Windows.csproj"
$WorkspaceCargoToml = Join-Path $Root "Cargo.toml"

function Invoke-Checked {
  param(
    [string]$FilePath,
    [string[]]$Arguments
  )

  & $FilePath @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "$FilePath failed with exit code $LASTEXITCODE"
  }
}

function Get-WorkspaceVersion {
  $Text = Get-Content -Raw -Path $WorkspaceCargoToml
  $Match = [regex]::Match($Text, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
  if (!$Match.Success) {
    throw "Could not read workspace version from $WorkspaceCargoToml"
  }
  return $Match.Groups[1].Value
}

function Resolve-InnoSetupCompiler {
  $Command = Get-Command iscc -ErrorAction SilentlyContinue
  if ($Command) {
    return $Command.Source
  }

  $Candidates = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
  )
  foreach ($Candidate in $Candidates) {
    if ($Candidate -and (Test-Path $Candidate)) {
      return $Candidate
    }
  }

  throw "Inno Setup compiler not found. Install JRSoftware.InnoSetup or put ISCC.exe on PATH."
}

function Resolve-OutputPath {
  param([string]$Path)
  if ([System.IO.Path]::IsPathRooted($Path)) {
    return $Path
  }
  return [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $Path))
}

if ($StopRunningApp) {
  Get-Process IrisDrive -ErrorAction SilentlyContinue | Stop-Process -Force
  Get-Process idrive -ErrorAction SilentlyContinue | Stop-Process -Force
}

if (-not $SkipCliBuild) {
  $CargoArgs = @("build", "-p", "idrive")
  if ($Configuration -eq "Release") {
    $CargoArgs += "--release"
  }
  if (-not $AllowLockfileUpdate) {
    $CargoArgs += "--locked"
  }
  Invoke-Checked cargo $CargoArgs
}

Invoke-Checked dotnet @(
  "publish",
  $Project,
  "-c",
  $Configuration,
  "-r",
  $Runtime,
  "--self-contained",
  "true",
  "-p:WindowsPackageType=None"
)

$PublishDir = Join-Path $Root "windows\bin\$Configuration\net8.0-windows\$Runtime\publish"
$CargoProfile = if ($Configuration -eq "Release") { "release" } else { "debug" }
$Idrive = Join-Path $Root "target\$CargoProfile\idrive.exe"
if (Test-Path $Idrive) {
  Copy-Item $Idrive (Join-Path $PublishDir "idrive.exe") -Force
}

if ($DesktopShortcut) {
  $Target = Join-Path $PublishDir "IrisDrive.exe"
  $Icon = Join-Path $PublishDir "IrisDrive.ico"
  if (-not (Test-Path $Target)) {
    throw "Missing published app: $Target"
  }
  if (-not (Test-Path $Icon)) {
    throw "Missing published icon: $Icon"
  }

  $Desktop = [Environment]::GetFolderPath("DesktopDirectory")
  $LinkPath = Join-Path $Desktop "Iris Drive.lnk"
  if (Test-Path $LinkPath) {
    Remove-Item -Force $LinkPath
  }

  $Shell = New-Object -ComObject WScript.Shell
  $Link = $Shell.CreateShortcut($LinkPath)
  $Link.TargetPath = $Target
  $Link.WorkingDirectory = $PublishDir
  $Link.IconLocation = "$Icon,0"
  $Link.Description = "Iris Drive"
  $Link.Save()
  [Runtime.InteropServices.Marshal]::FinalReleaseComObject($Link) | Out-Null
  [Runtime.InteropServices.Marshal]::FinalReleaseComObject($Shell) | Out-Null
  ie4uinit.exe -show | Out-Null
}

Write-Output "Published Iris Drive to $PublishDir"
Write-Output "Self-contained publish: no .NET Desktop Runtime install required."

if ($Installer) {
  if ($Runtime -ne "win-x64") {
    throw "The installer script currently supports win-x64 only, got $Runtime"
  }

  $VersionTag = if ($Tag) { $Tag } else { "v$(Get-WorkspaceVersion)" }
  if (!$VersionTag.StartsWith("v")) {
    $VersionTag = "v$VersionTag"
  }
  $Version = $VersionTag.TrimStart("v")
  $InstallerOutputDir = if ($OutputDir) { Resolve-OutputPath $OutputDir } else { Join-Path $Root "dist" }
  New-Item -ItemType Directory -Force -Path $InstallerOutputDir | Out-Null

  $AppExe = Join-Path $PublishDir "IrisDrive.exe"
  if (!(Test-Path $AppExe)) {
    throw "Published Windows app not found: $AppExe"
  }

  $env:IRIS_DRIVE_RELEASE_VERSION = $Version
  $env:IRIS_DRIVE_PROJECT_ROOT = $Root
  $env:IRIS_DRIVE_WINDOWS_PUBLISH_DIR = $PublishDir
  $env:IRIS_DRIVE_WINDOWS_INSTALLER_OUTPUT_DIR = $InstallerOutputDir
  $env:IRIS_DRIVE_WINDOWS_INSTALLER_BASENAME = "iris-drive-$VersionTag-windows-x64-setup"
  $InnoSetupCompiler = Resolve-InnoSetupCompiler
  Invoke-Checked $InnoSetupCompiler @((Join-Path $Root "scripts\windows-installer.iss"))

  $InstallerPath = Join-Path $InstallerOutputDir "$($env:IRIS_DRIVE_WINDOWS_INSTALLER_BASENAME).exe"
  if (!(Test-Path $InstallerPath)) {
    throw "Expected Windows installer was not produced: $InstallerPath"
  }
  Write-Output "Built Iris Drive installer: $InstallerPath"
}
