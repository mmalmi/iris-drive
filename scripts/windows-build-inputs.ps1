# Use .NET directly: Windows PowerShell launched from pwsh can inherit a
# PSModulePath that does not expose the Windows Get-FileHash cmdlet.
function Get-BuildInputFileHash {
    param([string] $Path, [ValidateSet('SHA256', 'SHA512')][string] $Algorithm)
    $stream = [IO.File]::OpenRead($Path)
    $hashAlgorithm = [Security.Cryptography.HashAlgorithm]::Create($Algorithm)
    try { return [BitConverter]::ToString($hashAlgorithm.ComputeHash($stream)).Replace('-', '') }
    finally { $hashAlgorithm.Dispose(); $stream.Dispose() }
}

# Validate restored SDK runtime inputs before compiling or publishing.
function Assert-PinnedDownloadPackages {
    param([string] $AssetsPath, [string] $PinsPath)
    $assets = Get-Content -Raw -LiteralPath $AssetsPath | ConvertFrom-Json
    $pins = Get-Content -Raw -LiteralPath $PinsPath | ConvertFrom-Json
    $checked = @{}
    foreach ($framework in $assets.project.frameworks.PSObject.Properties) {
        $downloads = $framework.Value.PSObject.Properties['downloadDependencies']
        if (-not $downloads) { continue }
        foreach ($package in $downloads.Value) {
            $pin = @($pins | Where-Object { $_.id -eq $package.name })
            if ($pin.Count -ne 1 -or $package.version -ne "[$($pin[0].version), $($pin[0].version)]") {
                throw "SDK download has no exact reviewed pin: $($package.name) $($package.version)"
            }
            $id = $pin[0].id.ToLowerInvariant()
            $version = $pin[0].version
            $key = "$id/$version"
            if ($checked.ContainsKey($key)) { continue }
            $archives = @($assets.packageFolders.PSObject.Properties | ForEach-Object {
                $candidate = Join-Path $_.Name "$id\$version\$id.$version.nupkg"
                if (Test-Path -LiteralPath $candidate -PathType Leaf) { $candidate }
            })
            if ($archives.Count -eq 0) { throw "Restored SDK package archive is missing: $key" }
            foreach ($archive in $archives) {
                $actual = (Get-BuildInputFileHash -Path $archive -Algorithm SHA512)
                if ($actual -ne $pin[0].sha512) { throw "SDK package content hash mismatch: $key" }
            }
            $checked[$key] = $true
        }
    }
    if ($checked.Count -eq 0) { throw 'The restore did not record any SDK download packages.' }
}
