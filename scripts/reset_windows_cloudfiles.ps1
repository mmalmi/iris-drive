param(
    [Parameter(Mandatory = $true)][string]$ConfigDir,
    [Parameter(Mandatory = $true)][string]$SyncRoot
)

$ErrorActionPreference = 'Stop'
if ($env:IRIS_NATIVE_LAB_ALLOW_RESET -ne '1') {
    Write-Error 'Cloud Files reset requires IRIS_NATIVE_LAB_ALLOW_RESET=1'
    exit 75
}

function Assert-LabPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $allowed = @(
        [IO.Path]::GetFullPath($env:TEMP),
        [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'IrisDriveLab'))
    )
    if (-not ($allowed | Where-Object { $full.StartsWith($_ + [IO.Path]::DirectorySeparatorChar) })) {
        throw "Refusing to reset non-lab path: $full"
    }
    return $full
}

$ConfigDir = Assert-LabPath $ConfigDir
$SyncRoot = Assert-LabPath $SyncRoot

Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -and ($_.CommandLine.Contains($ConfigDir) -or $_.CommandLine.Contains($SyncRoot)) } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }

if (-not ('IrisDriveLab.CfApi' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace IrisDriveLab {
    public static class CfApi {
        [DllImport("CfApi.dll", CharSet = CharSet.Unicode)]
        public static extern int CfUnregisterSyncRoot(string syncRootPath);
    }
}
'@
}

if (Test-Path -LiteralPath $SyncRoot) {
    $hresult = [IrisDriveLab.CfApi]::CfUnregisterSyncRoot($SyncRoot)
    if ($hresult -ne 0) {
        Write-Error ('CfUnregisterSyncRoot failed: 0x{0:X8}' -f ([uint32]$hresult))
        exit 75
    }
}

Remove-Item -LiteralPath $SyncRoot -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $ConfigDir -Recurse -Force -ErrorAction SilentlyContinue
Write-Output "reset windows-cloudfiles $SyncRoot"
