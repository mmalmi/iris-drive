param(
  [ValidateSet("Debug", "Release")]
  [string]$Configuration = "Debug",

  [string]$Runtime = "win-x64",

  [switch]$DesktopShortcut,

  [switch]$SkipCliBuild,

  [switch]$AllowLockfileUpdate
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Project = Join-Path $Root "windows\IrisDrive.Windows.csproj"

if (-not $SkipCliBuild) {
  $CargoArgs = @("build", "-p", "idrive")
  if ($Configuration -eq "Release") {
    $CargoArgs += "--release"
  }
  if (-not $AllowLockfileUpdate) {
    $CargoArgs += "--locked"
  }
  & cargo @CargoArgs
}

& dotnet publish $Project -c $Configuration -r $Runtime --self-contained true -p:WindowsPackageType=None

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
