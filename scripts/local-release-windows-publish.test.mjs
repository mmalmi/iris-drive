import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('./windows-publish.ps1', import.meta.url), 'utf8')
const localReleaseSource = readFileSync(new URL('./local-release.mjs', import.meta.url), 'utf8')
const projectSource = readFileSync(new URL('../windows/IrisDrive.Windows.csproj', import.meta.url), 'utf8')
const serviceSource = readFileSync(new URL('../windows/IrisDriveService.cs', import.meta.url), 'utf8')
const appVersionSource = readFileSync(new URL('../windows/IrisDriveServiceAppVersion.cs', import.meta.url), 'utf8')

function position(pattern, description) {
  const match = source.match(pattern)
  assert.ok(match, `Windows publisher must ${description}`)
  return match.index
}

function range(pattern, description) {
  const match = source.match(pattern)
  assert.ok(match, `Windows publisher must ${description}`)
  return { start: match.index, end: match.index + match[0].length, match }
}

function assertBefore(first, second, description) {
  assert.ok(first < second, description)
}

test('Windows publish runtime is fixed before the exact publish directory is cleaned', () => {
  const runtime = position(
    /\[ValidateSet\(\s*["']win-x64["']\s*\)\]\s*\[string\]\$Runtime/i,
    'constrain Runtime to win-x64',
  )
  const publishDir = position(/\$PublishDir\s*=\s*Join-Path\b/i, 'resolve the exact publish directory')
  const cleanup = position(
    /Remove-Item\b(?=[\s\S]{0,200}\$PublishDir)(?=[\s\S]{0,200}-Recurse)(?=[\s\S]{0,200}-Force)/i,
    'recursively remove the exact publish directory',
  )
  const publish = position(/Invoke-Checked\s+dotnet\s+\$DotnetArgs/i, 'invoke dotnet publish')

  assertBefore(runtime, cleanup, 'Runtime must be constrained before publish cleanup')
  assertBefore(publishDir, cleanup, 'The exact publish directory must be resolved before cleanup')
  assertBefore(cleanup, publish, 'The exact publish directory must be cleaned before dotnet publish')
})

test('app-core is always built and idrive is built unless explicitly skipped', () => {
  const cargo = position(
    /\$CargoArgs\s*=\s*@\(\s*["']build["']\s*,\s*["']-p["']\s*,\s*["']iris-drive-app-core["']\s*\)/i,
    'start every Cargo build with iris-drive-app-core',
  )
  const conditionalCli = range(
    /if\s*\(\s*(?:-not|!)\s+\$SkipCliBuild\s*\)\s*\{[\s\S]{0,400}?\$CargoArgs\s*\+=\s*@\([\s\S]{0,160}?["']-p["'][\s\S]{0,80}?["']idrive["'][\s\S]{0,80}?\)[\s\S]{0,160}?\}/i,
    'add idrive to the Cargo build unless SkipCliBuild is set',
  )
  const invoke = position(/Invoke-Checked\s+cargo\s+\$CargoArgs/i, 'run the combined Cargo build')

  assertBefore(cargo, conditionalCli.start, 'The unconditional app-core build must precede the optional CLI build')
  assertBefore(conditionalCli.end, invoke, 'The unconditional Cargo invocation must follow CLI selection')
  assert.match(
    source,
    /if\s*\(\s*-not\s+\$SkipCliBuild\s+-and\s+\(Test-Path\s+\$Idrive\)\s*\)\s*\{\s*Remove-Item\s+-Path\s+\$Idrive\s+-Force/i,
  )
  assert.doesNotMatch(source, /Remove-Item\s+-Path\s+\$CargoOutputDir\s+-Recurse/i)
})

test('native path mapping is probed before the expensive Cargo build', () => {
  const probe = position(
    /Test-ReleaseNativePathRemapping\s+-Compiler\s+\$ReleaseNative\.Compiler\s+-Flags\s+\$ReleaseNative\.Flags/i,
    'run the clang-cl path-mapping probe',
  )
  const cargo = position(/Invoke-Checked\s+cargo\s+\$CargoArgs/i, 'run Cargo after the probe')

  assertBefore(probe, cargo, 'Native path mapping must be proven before Cargo starts')
  assert.doesNotMatch(
    source,
    /\$ObjectText[\s\S]{0,240}expected source path mapping/i,
    'basename-only no-debug objects must not require a synthetic path marker',
  )
})

test('both native artifacts are required before dotnet publish and explicitly staged', () => {
  const idrivePath = range(
    /\$(\w+)\s*=\s*Join-Path[^\r\n]*idrive\.exe/i,
    'resolve idrive.exe',
  )
  const appCorePath = range(
    /\$(\w+)\s*=\s*Join-Path[^\r\n]*iris_drive_app_core\.dll/i,
    'resolve iris_drive_app_core.dll',
  )
  const idrive = idrivePath.match[1]
  const appCore = appCorePath.match[1]
  const idriveRequired = position(
    new RegExp(`if\\s*\\(\\s*(?:-not|!)\\s*\\(?\\s*Test-Path\\s+(?:-Path\\s+)?\\$${idrive}\\s*\\)?\\s*\\)\\s*\\{\\s*throw`, 'i'),
    'fail when idrive.exe is missing',
  )
  const appCoreRequired = position(
    new RegExp(`if\\s*\\(\\s*(?:-not|!)\\s*\\(?\\s*Test-Path\\s+(?:-Path\\s+)?\\$${appCore}\\s*\\)?\\s*\\)\\s*\\{\\s*throw`, 'i'),
    'fail when iris_drive_app_core.dll is missing',
  )
  const publish = position(/Invoke-Checked\s+dotnet\s+\$DotnetArgs/i, 'invoke dotnet publish')
  const idriveCopy = position(
    new RegExp(`Copy-Item\\s+(?:-Path\\s+)?\\$${idrive}\\s+(?:-Destination\\s+)?\\(Join-Path\\s+\\$PublishDir\\s+["']idrive\\.exe["']\\)\\s+-Force`, 'i'),
    'explicitly stage idrive.exe',
  )
  const appCoreCopy = position(
    new RegExp(`Copy-Item\\s+(?:-Path\\s+)?\\$${appCore}\\s+(?:-Destination\\s+)?\\(Join-Path\\s+\\$PublishDir\\s+["']iris_drive_app_core\\.dll["']\\)\\s+-Force`, 'i'),
    'explicitly stage iris_drive_app_core.dll',
  )

  for (const required of [idrivePath.start, appCorePath.start, idriveRequired, appCoreRequired]) {
    assertBefore(required, publish, 'Native artifacts must be resolved and required before dotnet publish')
  }
  assertBefore(publish, idriveCopy, 'idrive.exe must be staged after dotnet publish')
  assertBefore(publish, appCoreCopy, 'iris_drive_app_core.dll must be staged after dotnet publish')
})

test('dotnet publish disables portable debug artifacts only for Release', () => {
  const args = position(/\$DotnetArgs\s*=\s*@\(/i, 'construct dotnet publish arguments')
  const releaseSymbols = range(
    /if\s*\(\s*\$Configuration\s+-eq\s+["']Release["']\s*\)\s*\{[\s\S]{0,300}?\$DotnetArgs\s*\+=\s*@\([\s\S]{0,200}?["']-p:DebugType=None["'][\s\S]{0,200}?["']-p:DebugSymbols=false["'][\s\S]{0,100}?\)[\s\S]{0,80}?\}/i,
    'disable DebugType and DebugSymbols only for Release',
  )
  const publish = position(/Invoke-Checked\s+dotnet\s+\$DotnetArgs/i, 'run dotnet publish')

  assertBefore(args, releaseSymbols.start, 'Dotnet arguments must exist before Release options')
  assertBefore(releaseSymbols.end, publish, 'Release symbol options must precede dotnet publish')
})

test('recursive debug artifacts are rejected before Inno Setup', () => {
  const scan = range(
    /\$(\w+)\s*=\s*@?\(?\s*Get-ChildItem\b(?=[\s\S]{0,240}\$PublishDir)(?=[\s\S]{0,240}-Recurse)(?=[\s\S]{0,240}-File)(?=[\s\S]{0,240}\*\.pdb)(?=[\s\S]{0,240}\*\.dbg)[^\r\n]*/i,
    'recursively scan the publish directory for PDB and DBG files',
  )
  const rejection = position(
    new RegExp(`if\\s*\\([^\\r\\n]*\\$${scan.match[1]}[^\\r\\n]*\\)\\s*\\{\\s*throw`, 'i'),
    'reject discovered debug artifacts',
  )
  const inno = position(
    /\$InnoSetupCompiler\s*=\s*Resolve-InnoSetupCompiler/i,
    'resolve Inno Setup after the debug-artifact gate',
  )

  assertBefore(scan.start, rejection, 'Debug artifacts must be scanned before rejection')
  assertBefore(rejection, inno, 'Debug artifacts must be rejected before Inno Setup')
})

test('the complete uncompressed payload is privacy-audited before Inno Setup', () => {
  const audit = position(
    /release-build-hygiene-cli\.mjs[\s\S]{0,240}\$PublishDir/i,
    'privacy-audit the complete publish directory',
  )
  const inno = position(
    /\$InnoSetupCompiler\s*=\s*Resolve-InnoSetupCompiler/i,
    'resolve Inno Setup after the payload privacy gate',
  )

  assertBefore(audit, inno, 'Payload privacy audit must finish before Inno Setup')
})

test('the mixed Windows payload carries the project MIT notice', () => {
  const rootLicense = position(
    /\$\w*License\w*\s*=\s*Join-Path\s+\$Root\s+["']LICENSE["']/i,
    'resolve the repository license',
  )
  const destination = position(
    /Join-Path\s+\$PublishDir\s+["']LICENSE["']/i,
    'place the project license in the publish directory',
  )
  const copy = position(
    /Copy-Item\s+(?:-Path\s+)?\$\w*License\w*\s+(?:-Destination\s+)?\$\w*License\w*\s+-Force/i,
    'copy the repository license bytes into the payload',
  )
  const required = position(
    /foreach\s*\(\s*\$RequiredName\s+in\s+@\([\s\S]{0,500}?["']LICENSE["'][\s\S]{0,80}?\)\s*\)/i,
    'include the staged project license in the required payload set',
  )
  const audit = position(
    /release-build-hygiene-cli\.mjs[\s\S]{0,240}\$PublishDir/i,
    'privacy-audit the complete publish directory',
  )
  const inno = position(
    /\$InnoSetupCompiler\s*=\s*Resolve-InnoSetupCompiler/i,
    'resolve Inno Setup after the payload gates',
  )

  assertBefore(rootLicense, destination, 'The repository license must be resolved before staging')
  assertBefore(destination, copy, 'The license destination must be resolved before copying')
  assertBefore(copy, audit, 'The license must be present for the payload privacy audit')
  assertBefore(required, audit, 'The license must be required before the payload privacy audit')
  assertBefore(audit, inno, 'The notice-bearing payload must be audited before Inno Setup')
})

test('the standalone Windows CLI archive contains only idrive.exe and the root license', () => {
  const packageBlock = source.match(/\$CliArchivePath\s*=[\s\S]+?Write-Output\s+["']Built Iris Drive CLI archive:/i)
  assert.ok(packageBlock, 'Windows publisher must have a bounded CLI archive block')
  assert.match(packageBlock[0], /Copy-Item[\s\S]{0,240}\$Idrive[\s\S]{0,160}["']idrive\.exe["']/i)
  assert.match(packageBlock[0], /Copy-Item[\s\S]{0,240}\$RootLicense[\s\S]{0,160}["']LICENSE["']/i)
  assert.match(
    packageBlock[0],
    /Compress-Archive\s+-Path\s+@\([\s\S]{0,240}["']idrive\.exe["'][\s\S]{0,240}["']LICENSE["'][\s\S]{0,120}\)/i,
  )
  assert.doesNotMatch(packageBlock[0], /Compress-Archive\s+-Path\s+[^\r\n]*[\\/]\*/i)
  assert.match(packageBlock[0], /\[DateTime\]["']2000-01-01T00:00:00["']/i)
  assert.match(packageBlock[0], /\.LastWriteTime\s*=\s*\$timestamp/i)
  assert.doesNotMatch(packageBlock[0], /LastWriteTimeUtc|1980-01-01/i)

  const acceptance = localReleaseSource.match(
    /assertZipEntriesEqualFiles\(\s*\[[\s\S]{0,500}?["']idrive\.exe["'][\s\S]{0,300}?["']LICENSE["'][\s\S]{0,200}?Windows CLI archive validation failed\./i,
  )
  assert.ok(acceptance, 'The release orchestrator must byte-verify both CLI archive entries')
})

test('the final installer is privacy-audited after Inno Setup', () => {
  const inno = position(
    /Invoke-Checked\s+\$InnoSetupCompiler/i,
    'run Inno Setup before auditing its output',
  )
  const exists = position(
    /if\s*\(\s*!\s*\(Test-Path\s+\$InstallerPath\)\s*\)\s*\{\s*throw/i,
    'require the produced installer before auditing it',
  )
  const audit = position(
    /release-build-hygiene-cli\.mjs[\s\S]{0,240}\$InstallerPath/i,
    'privacy-audit the final installer',
  )

  assertBefore(inno, exists, 'Inno Setup must finish before installer existence is checked')
  assertBefore(exists, audit, 'The installer must exist before its privacy audit')
})

test('Windows publishing remains unsigned', () => {
  assert.doesNotMatch(
    source,
    /\b(?:signtool|Set-AuthenticodeSignature|RequireSigning)\b|^\s*SignTool\s*=/im,
  )
  assert.doesNotMatch(source, /Llvm\\ARM64/i)
})


test('Windows runtime downloads are locked and checked before native compilation', () => {
  const restore = position(/Invoke-Checked dotnet @\("restore"/, 'restore locked runtime inputs')
  const verify = position(/Assert-PinnedDownloadPackages -AssetsPath/, 'verify downloaded package archives')
  const cargo = position(/Invoke-Checked\s+cargo\s+\$CargoArgs/i, 'compile native code')
  assertBefore(restore, verify, 'Restore precedes archive verification')
  assertBefore(verify, cargo, 'Input verification precedes native compilation')
  assert.match(source, /"--locked-mode"/)
  assert.match(source, /"publish",[\s\S]{0,80}"--no-restore"/)
})

test('every Windows app build is stamped with the workspace version or fails', () => {
  const read = projectSource.match(
    /<IrisDriveWorkspaceVersion\b[^>]*>[\s\S]*?<\/IrisDriveWorkspaceVersion>/g,
  )
  assert.ok(read, 'The Windows project must derive IrisDriveWorkspaceVersion')
  const derivation = read.join('\n')
  assert.match(derivation, /ReadAllText/, 'The version must be read from a file')
  assert.match(derivation, /ReadAllText\('\$\(IrisDriveWorkspaceCargoToml\)'\)/)
  assert.match(
    projectSource,
    /<IrisDriveWorkspaceCargoToml>[^<]*'\$\(MSBuildThisFileDirectory\)',\s*'\.\.',\s*'Cargo\.toml'[^<]*<\/IrisDriveWorkspaceCargoToml>/,
    'The version must come from the root Cargo.toml',
  )
  assert.match(derivation, /\\\[workspace\\\.package\\\]/, 'The version must come from [workspace.package]')
  assert.match(projectSource, /<Version>\$\(IrisDriveWorkspaceVersion\)<\/Version>/)
  assert.match(
    projectSource,
    /<Error\s+Condition="'\$\(IrisDriveWorkspaceVersion\)'\s*==\s*''"/,
    'A build without a readable workspace version must fail instead of using the SDK default 1.0.0',
  )
  assert.match(
    projectSource,
    /<IncludeSourceRevisionInInformationalVersion>false<\/IncludeSourceRevisionInInformationalVersion>/,
    'The informational version the app reports must be the bare release version',
  )
  assert.doesNotMatch(
    projectSource,
    /<(?:Version|VersionPrefix|AssemblyVersion|FileVersion|InformationalVersion)>\s*\d/,
    'The Windows project must not carry a second hand-maintained version',
  )
})

test('the publish step proves the exe carries the workspace version before packaging', () => {
  const version = position(/\$AppVersion\s*=\s*Get-WorkspaceVersion\b/, 'read the workspace version once')
  const tagGate = position(
    /if\s*\(\s*\$Tag\s+-and\s+\$Tag\.TrimStart\(["']v["']\)\s+-ne\s+\$AppVersion\s*\)\s*\{\s*throw/i,
    'reject a release tag that differs from the workspace version',
  )
  const cargo = position(/Invoke-Checked\s+cargo\s+\$CargoArgs/i, 'run the Cargo build')
  const publish = position(/Invoke-Checked\s+dotnet\s+\$DotnetArgs/i, 'run dotnet publish')
  const stamped = position(
    /\$StampedApp\s*=\s*\(Get-Item\s+-LiteralPath\s+\(Join-Path\s+\$PublishDir\s+["']IrisDrive\.exe["']\)\)\.VersionInfo/i,
    'read the published IrisDrive.exe version info',
  )
  const gate = position(
    /if\s*\(\s*\$StampedApp\.ProductVersion\s+-ne\s+\$AppVersion\s+-or\s+\$StampedApp\.FileVersion\s+-ne\s+["']\$AppVersion\.0["']\s*\)\s*\{\s*throw/i,
    'fail when IrisDrive.exe ProductVersion or FileVersion differs from the workspace version',
  )
  const inno = position(/\$InnoSetupCompiler\s*=\s*Resolve-InnoSetupCompiler/i, 'build the installer')

  assertBefore(version, tagGate, 'The workspace version must be read before the tag gate')
  assertBefore(tagGate, cargo, 'A mismatched tag must fail before the expensive Cargo build')
  assertBefore(publish, stamped, 'The stamp is checked on the published exe')
  assertBefore(stamped, gate, 'The stamp is read before it is checked')
  assertBefore(gate, inno, 'The stamp must be proven before the installer is built')
  assert.match(source, /\$Version\s*=\s*\$AppVersion\b/, 'The installer must carry the same version as the exe')
})

test('the Windows app reports and update-checks its stamped version without a fallback', () => {
  assert.doesNotMatch(serviceSource, /\bAppVersion\s*=>/, 'AppVersion has one definition')
  const appVersion = appVersionSource.match(/public\s+string\s+AppVersion\b[\s\S]*?;/)
  assert.ok(appVersion, 'IrisDriveService must expose AppVersion')
  assert.match(appVersion[0], /AssemblyInformationalVersionAttribute/)
  assert.match(appVersion[0], /throw\b/, 'A missing stamp must fail loudly')
  assert.doesNotMatch(appVersion[0], /GetName\(\)\.Version|"\d+\.\d+/, 'No assembly-version or hard-coded fallback')
  assert.match(serviceSource, /CheckUpdate\(DefaultConfigDirectory,\s*AppVersion\)/)
  assert.match(serviceSource, /DownloadUpdate\(DefaultConfigDirectory,\s*AppVersion,/)
})
