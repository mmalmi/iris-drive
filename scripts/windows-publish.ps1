param(
  [ValidateSet("Debug", "Release")]
  [string]$Configuration = "Debug",

  [ValidateSet("win-x64")]
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

function Set-ReleaseRustPathRemapping {
  $Current = "$($env:CARGO_ENCODED_RUSTFLAGS)$($env:RUSTFLAGS)"
  if (-not $Current.Contains("--remap-path-prefix=$Root=")) {
    $HomeDirectory = [Environment]::GetFolderPath("UserProfile")
    $Flags = @(
      "--remap-path-prefix=$Root=/usr/src/iris-drive",
      "--remap-path-prefix=$HomeDirectory=/usr/src/home"
    )
    if ($env:CARGO_ENCODED_RUSTFLAGS) {
      $env:CARGO_ENCODED_RUSTFLAGS = [string]::Join(
        [char]0x1f,
        @($env:CARGO_ENCODED_RUSTFLAGS) + $Flags
      )
    } elseif ($env:RUSTFLAGS) {
      $env:RUSTFLAGS = "$($env:RUSTFLAGS) $($Flags -join ' ')"
    } else {
      $env:CARGO_ENCODED_RUSTFLAGS = [string]::Join([char]0x1f, $Flags)
    }
  }
}

function Resolve-ClangCl {
  $Command = Get-Command clang-cl.exe -ErrorAction SilentlyContinue
  if ($Command) {
    return $Command.Source
  }

  $VsWhereCommand = Get-Command vswhere.exe -ErrorAction SilentlyContinue
  $VsWhere = if ($VsWhereCommand) {
    $VsWhereCommand.Source
  } else {
    "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  }
  if ($VsWhere -and (Test-Path $VsWhere)) {
    $Installations = @(& $VsWhere -all -products * -property installationPath 2>$null)
    foreach ($Installation in $Installations) {
      $Candidate = Join-Path $Installation "VC\Tools\Llvm\x64\bin\clang-cl.exe"
      if (Test-Path $Candidate) {
        return $Candidate
      }
    }
  }

  throw "Release builds require the Visual Studio C++ Clang compiler component."
}

function Add-QuotedCompilerFlags {
  param(
    [string]$Current,
    [string[]]$Flags
  )

  $Values = @()
  if ($Current) {
    $Values += $Current.Trim()
  }
  foreach ($Flag in $Flags) {
    if ($Flag.Contains('"')) {
      throw "Release compiler path contains an unsupported character."
    }
    $Values += '"' + $Flag + '"'
  }
  return $Values -join " "
}

function Set-ReleaseNativePathRemapping {
  $Compiler = Resolve-ClangCl
  $HomeDirectory = [Environment]::GetFolderPath("UserProfile")
  $Flags = @(
    "/clang:-Werror=unknown-argument",
    "/clang:-ffile-prefix-map=$HomeDirectory=/usr/src/home",
    "/clang:-ffile-prefix-map=$Root=/usr/src/iris-drive"
  )
  $env:CC_x86_64_pc_windows_msvc = $Compiler
  $env:CXX_x86_64_pc_windows_msvc = $Compiler
  $env:CFLAGS_x86_64_pc_windows_msvc = Add-QuotedCompilerFlags $env:CFLAGS_x86_64_pc_windows_msvc $Flags
  $env:CXXFLAGS_x86_64_pc_windows_msvc = Add-QuotedCompilerFlags $env:CXXFLAGS_x86_64_pc_windows_msvc $Flags
  $env:CC_SHELL_ESCAPED_FLAGS = "1"
  return [pscustomobject]@{ Compiler = $Compiler; Flags = $Flags }
}

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

function Test-ReleaseNativePathRemapping {
  param(
    [string]$Compiler,
    [string[]]$Flags
  )

  $ProbeDir = Join-Path $Root "target\release-path-probe"
  $Source = Join-Path $ProbeDir "path_probe.c"
  $Object = Join-Path $ProbeDir "path_probe.obj"
  if (Test-Path $ProbeDir) {
    Remove-Item -Path $ProbeDir -Recurse -Force
  }
  New-Item -ItemType Directory -Force -Path $ProbeDir | Out-Null
  try {
    Set-Content -Path $Source -Encoding Ascii -NoNewline -Value "const char *iris_release_path_probe(void) { return __FILE__; }"
    Invoke-Checked $Compiler (@("/nologo", "/c", $Source, "/Fo$Object") + $Flags)
    Invoke-Checked node @((Join-Path $Root "scripts\release-build-hygiene-cli.mjs"), $Root, $Object)
    $ObjectText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($Object))
    if (-not $ObjectText.Contains("/usr/src/iris-drive")) {
      throw "Release compiler did not apply the expected source path mapping."
    }
  } finally {
    Remove-Item -Path $ProbeDir -Recurse -Force -ErrorAction SilentlyContinue
  }
}

if ($Configuration -eq "Release") {
  Set-ReleaseRustPathRemapping
  $ReleaseNative = Set-ReleaseNativePathRemapping
  $env:CARGO_TARGET_DIR = Join-Path $Root "target"
  Test-ReleaseNativePathRemapping -Compiler $ReleaseNative.Compiler -Flags $ReleaseNative.Flags
}

if ($StopRunningApp) {
  Get-Process IrisDrive -ErrorAction SilentlyContinue | Stop-Process -Force
  Get-Process idrive -ErrorAction SilentlyContinue | Stop-Process -Force
}

$CargoProfile = if ($Configuration -eq "Release") { "release" } else { "debug" }
$CargoOutputDir = Join-Path $Root "target\$CargoProfile"
$Idrive = Join-Path $CargoOutputDir "idrive.exe"
$AppCore = Join-Path $CargoOutputDir "iris_drive_app_core.dll"
$PublishDir = Join-Path $Root "windows\bin\$Configuration\net8.0-windows\$Runtime\publish"

if (-not $SkipCliBuild -and (Test-Path $Idrive)) {
  Remove-Item -Path $Idrive -Force
}
if (Test-Path $AppCore) {
  Remove-Item -Path $AppCore -Force
}
$CargoArgs = @("build", "-p", "iris-drive-app-core")
if (-not $SkipCliBuild) {
  $CargoArgs += @("-p", "idrive")
}
if ($Configuration -eq "Release") {
  $CargoArgs += "--release"
}
if (-not $AllowLockfileUpdate) {
  $CargoArgs += "--locked"
}
Invoke-Checked cargo $CargoArgs

if (-not (Test-Path $Idrive)) {
  throw "Missing required Rust artifact: idrive.exe"
}
if (-not (Test-Path $AppCore)) {
  throw "Missing required Rust artifact: iris_drive_app_core.dll"
}
if (Test-Path $PublishDir) {
  Remove-Item -Path $PublishDir -Recurse -Force
}

$DotnetArgs = @(
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
if ($Configuration -eq "Release") {
  $DotnetArgs += @("-p:DebugType=None", "-p:DebugSymbols=false")
}
Invoke-Checked dotnet $DotnetArgs

Copy-Item $Idrive (Join-Path $PublishDir "idrive.exe") -Force
Copy-Item $AppCore (Join-Path $PublishDir "iris_drive_app_core.dll") -Force

foreach ($RequiredName in @("IrisDrive.exe", "IrisDrive.ico", "IrisDrive.png", "idrive.exe", "iris_drive_app_core.dll")) {
  if (-not (Test-Path (Join-Path $PublishDir $RequiredName))) {
    throw "Missing required Windows payload file: $RequiredName"
  }
}

if ($Configuration -eq "Release") {
  $DebugArtifacts = @(Get-ChildItem -Path $PublishDir -Recurse -File -Include "*.pdb", "*.dbg")
  if ($DebugArtifacts.Count -ne 0) {
    throw "Release payload contains debug artifacts."
  }
  Invoke-Checked node @((Join-Path $Root "scripts\release-build-hygiene-cli.mjs"), $Root, $PublishDir)
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
  if ($Configuration -eq "Release") {
    Invoke-Checked node @((Join-Path $Root "scripts\release-build-hygiene-cli.mjs"), $Root, $InstallerPath)
  }
  Write-Output "Built Iris Drive installer: $InstallerPath"
}
