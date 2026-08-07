import { statSync } from 'node:fs'
import { basename, join } from 'node:path'

export function parseEnvFile(text) {
  const values = {}
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line || line.startsWith('#')) {
      continue
    }
    const separator = line.indexOf('=')
    if (separator <= 0) {
      continue
    }
    const key = line.slice(0, separator).trim()
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
      continue
    }
    let value = line.slice(separator + 1).trim()
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1)
    }
    values[key] = value
      .replace(/\\n/g, '\n')
      .replace(/\\r/g, '\r')
      .replace(/\\t/g, '\t')
  }
  return values
}

export function splitCsv(value) {
  return (value || '')
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)
}

export function normalizeTag(value) {
  if (!value || !value.trim()) {
    throw new Error('Release tag must not be empty')
  }
  return value.startsWith('v') ? value : `v${value}`
}

export function semverFromTag(tag) {
  const stripped = normalizeTag(tag).replace(/^v/, '')
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(stripped)) {
    throw new Error(`Release tag must be semver-shaped, got "${tag}"`)
  }
  return stripped
}

// v0.1.32 already shipped as build 1033. This epoch offset keeps every later
// semver-derived build ahead of the original numbering sequence.
const BUILD_NUMBER_EPOCH = 1

export function buildNumberFromVersion(version) {
  const semver = semverFromTag(version)
  const match = semver.match(/^(\d+)\.(\d+)\.(\d+)$/)
  if (!match) {
    throw new Error(`Build numbers require a stable release version, got "${version}"`)
  }
  const [, majorText, minorText, patchText] = match
  const [major, minor, patch] = [majorText, minorText, patchText].map(Number)
  if (minor > 999 || patch > 999) {
    throw new Error('Build-number minor and patch components must each be at most 999')
  }
  const build = major * 1_000_000 + minor * 1_000 + patch + BUILD_NUMBER_EPOCH
  if (!Number.isSafeInteger(build) || build > 2_147_483_647) {
    throw new Error(`Build number is outside the supported Android integer range: ${build}`)
  }
  return String(build)
}

export function bumpPbxprojReleaseVersions(pbxprojText, version) {
  const semver = semverFromTag(version)
  const build = buildNumberFromVersion(semver)
  return pbxprojText
    .replace(/(\bMARKETING_VERSION\s*=\s*)[^;]+(;)/g, `$1${semver}$2`)
    .replace(/(\bCURRENT_PROJECT_VERSION\s*=\s*)[^;]+(;)/g, `$1${build}$2`)
}

export function bumpXcodegenProjectVersions(projectText, version) {
  const semver = semverFromTag(version)
  const build = buildNumberFromVersion(semver)
  return projectText
    .replace(/(\bMARKETING_VERSION:\s*)"?[^"\n]+"?/g, `$1"${semver}"`)
    .replace(/(\bCURRENT_PROJECT_VERSION:\s*)"?[^"\n]+"?/g, `$1"${build}"`)
}

export function bumpCargoPackageVersion(cargoTomlText, version) {
  const semver = semverFromTag(version)
  const match = cargoTomlText.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[)/m)
  if (!match) {
    throw new Error('Could not find [package] table in Cargo.toml')
  }
  const original = match[0]
  if (!/(\nversion\s*=\s*")[^"]+(")/.test(original)) {
    throw new Error('Could not find version field inside [package] table')
  }
  const replaced = original.replace(/(\nversion\s*=\s*")[^"]+(")/, `$1${semver}$2`)
  return cargoTomlText.replace(original, replaced)
}

export function readWorkspaceVersionTag(cargoTomlText) {
  const match = cargoTomlText.match(
    /^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"\n]+)"/m,
  )
  if (!match) {
    throw new Error('Could not find [workspace.package] version in Cargo.toml')
  }
  return normalizeTag(match[1])
}

export function describeAsset(name) {
  if (/^idrive-v.*-aarch64-apple-darwin\.tar\.gz$/.test(name)) {
    return 'macOS Apple Silicon idrive CLI'
  }
  if (/^idrive-v.*-x86_64-apple-darwin\.tar\.gz$/.test(name)) {
    return 'macOS Intel idrive CLI'
  }
  if (/^idrive-v.*-x86_64-unknown-linux-musl\.tar\.gz$/.test(name)) {
    return 'Linux x64 idrive CLI'
  }
  if (/^idrive-v.*-x86_64-unknown-linux-gnu\.tar\.gz$/.test(name)) {
    return 'Linux x64 idrive CLI'
  }
  if (/^idrive-v.*-aarch64-unknown-linux-musl\.tar\.gz$/.test(name)) {
    return 'Linux ARM64 idrive CLI'
  }
  if (/^idrive-v.*-x86_64-pc-windows-msvc\.zip$/.test(name)) {
    return 'Windows x64 idrive CLI (unsigned)'
  }
  if (/^idrive-v.*-aarch64-pc-windows-msvc\.zip$/.test(name)) {
    return 'Windows ARM64 idrive CLI (unsigned)'
  }
  if (/^iris-drive-v.*-macos-arm64\.dmg$/.test(name)) {
    return 'Iris Drive for macOS'
  }
  if (/^iris-drive-v.*-macos-arm64\.app\.tar\.gz$/.test(name)) {
    return 'Iris Drive macOS updater archive'
  }
  if (/^iris-drive-v.*-linux-x64\.AppImage$/.test(name)) {
    return 'Iris Drive for Linux AppImage'
  }
  if (/^iris-drive-v.*-linux-x64\.deb$/.test(name)) {
    return 'Iris Drive for Debian/Ubuntu (.deb)'
  }
  if (/^iris-drive-v.*-windows-x64-setup\.exe$/.test(name)) {
    return 'Iris Drive for Windows (unsigned)'
  }
  if (/^iris-drive-v.*-android-arm64\.apk$/.test(name)) {
    return 'Iris Drive for Android'
  }
  if (/^iris-drive-v.*-android-arm64\.aab$/.test(name)) {
    return 'Iris Drive Android app bundle'
  }
  return name
}

export function validateReleaseAssetSet(
  assetNames,
  { requireCompleteAppRelease = false } = {},
) {
  const names = [...assetNames]
  const hasMacosDmg = names.some((name) => /^iris-drive-v.*-macos-arm64\.dmg$/.test(name))
  const hasMacosUpdater = names.some((name) =>
    /^iris-drive-v.*-macos-arm64\.app\.tar\.gz$/.test(name),
  )
  const hasLinuxX64Desktop = names.some((name) =>
    /^iris-drive-v.*-linux-x64\.(AppImage|deb)$/.test(name),
  )
  const hasWindowsX64Cli = names.some((name) =>
    /^idrive-v.*-x86_64-pc-windows-msvc\.zip$/.test(name),
  )
  const hasWindowsX64Setup = names.some((name) =>
    /^iris-drive-v.*-windows-x64-setup\.exe$/.test(name),
  )
  const hasSignedAndroidApk = names.some((name) =>
    /^iris-drive-v.*-android-arm64\.apk$/.test(name),
  )
  const hasUnsignedAndroid = names.some((name) =>
    /^iris-drive-v.*-android-arm64-unsigned\.(apk|aab)$/.test(name),
  )

  if (hasUnsignedAndroid) {
    throw new Error(
      'Release includes unsigned Android artifacts. Configure Android signing for public releases.',
    )
  }

  if (hasMacosDmg && !hasMacosUpdater) {
    throw new Error(
      'Release includes a macOS DMG but no macOS .app.tar.gz updater archive.',
    )
  }

  if (requireCompleteAppRelease) {
    const missing = []
    if (!hasMacosDmg) {
      missing.push('macOS DMG')
    }
    if (!hasMacosUpdater) {
      missing.push('macOS updater archive')
    }
    if (!hasLinuxX64Desktop) {
      missing.push('Linux x64 desktop package')
    }
    if (!hasWindowsX64Cli) {
      missing.push('Windows x64 CLI archive')
    }
    if (!hasWindowsX64Setup) {
      missing.push('Windows x64 installer')
    }
    if (!hasSignedAndroidApk) {
      missing.push('signed Android APK')
    }
    if (missing.length > 0) {
      throw new Error(`Release is missing required app artifact(s): ${missing.join(', ')}.`)
    }
  }
}

export function plannedReleaseAssetNames(tag, steps, { signedAndroid = true } = {}) {
  const normalizedTag = normalizeTag(tag)
  const names = []
  const selected = new Set(steps)
  if (selected.has('macos')) {
    names.push(`idrive-${normalizedTag}-aarch64-apple-darwin.tar.gz`)
    names.push(`iris-drive-${normalizedTag}-macos-arm64.dmg`)
    names.push(`iris-drive-${normalizedTag}-macos-arm64.app.tar.gz`)
  }
  if (selected.has('linux')) {
    names.push(`idrive-${normalizedTag}-x86_64-unknown-linux-gnu.tar.gz`)
    names.push(`iris-drive-${normalizedTag}-linux-x64.deb`)
  }
  if (selected.has('windows')) {
    names.push(`idrive-${normalizedTag}-x86_64-pc-windows-msvc.zip`)
    names.push(`iris-drive-${normalizedTag}-windows-x64-setup.exe`)
  }
  if (selected.has('android')) {
    const suffix = signedAndroid ? '' : '-unsigned'
    names.push(`iris-drive-${normalizedTag}-android-arm64${suffix}.apk`)
    names.push(`iris-drive-${normalizedTag}-android-arm64${suffix}.aab`)
  }
  return names
}

export function canonicalReleaseAssetNames(tag) {
  return plannedReleaseAssetNames(tag, ['macos', 'linux', 'windows', 'android'])
}

export function validateCanonicalReleaseAssetSet(
  tag,
  assetNames,
  directoryEntryNames = assetNames,
) {
  const expected = canonicalReleaseAssetNames(tag)
  const normalizedTag = normalizeTag(tag).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const tagPattern = new RegExp(`(?:^|[-_])${normalizedTag}(?=$|[-_.])`)
  const actual = [...assetNames].sort()
  const discovered = [...directoryEntryNames].filter((name) => tagPattern.test(name)).sort()
  const missing = expected.filter((name) => !actual.includes(name))
  const unexpected = discovered.filter((name) => !expected.includes(name))
  const errors = []
  if (missing.length > 0) {
    errors.push(`Missing canonical release asset(s): ${missing.join(', ')}`)
  }
  if (unexpected.length > 0) {
    errors.push(`Unexpected tag-matching release asset(s): ${unexpected.join(', ')}`)
  }
  if (errors.length > 0) {
    throw new Error(errors.join('; '))
  }
}

export function parseNotarytoolSubmitOutput(text) {
  const idMatches = [...String(text).matchAll(/^\s*id:\s*([0-9a-f-]+)/gim)]
  const statusMatches = [
    ...String(text).matchAll(/(?:Current status:|^\s*status:)\s*([A-Za-z]+)/gim),
  ]
  return {
    id: idMatches.at(-1)?.[1] ?? '',
    status: statusMatches.at(-1)?.[1]?.toLowerCase() ?? '',
  }
}

export function buildZapstorePublishPlan({
  tag,
  assetDir,
  distDir,
  apkExists,
  zspAvailable,
  signWith,
  zapstoreYamlExists,
}) {
  const normalizedTag = normalizeTag(tag)
  const apkName = `iris-drive-${normalizedTag}-android-arm64.apk`
  const apkPath = join(assetDir, apkName)
  if (!apkExists) {
    throw new Error(`Missing Android APK for Zapstore publish: ${apkPath}`)
  }
  if (!zspAvailable) {
    throw new Error('Missing zsp; cannot publish Zapstore release')
  }
  const trimmedSignWith = String(signWith ?? '').trim()
  if (!trimmedSignWith) {
    throw new Error(
      'Missing Zapstore signing key; set SIGN_WITH or NOSTR_KEY_PATH in .env.zapstore.local',
    )
  }
  if (!zapstoreYamlExists) {
    throw new Error('Missing zapstore.yaml; cannot publish Zapstore release')
  }
  return {
    apkName,
    apkPath,
    releaseSourcePath: join(distDir, 'zapstore-current-android-arm64.apk'),
    signWith: trimmedSignWith,
  }
}

function idriveTarget(name) {
  const targets = [
    'aarch64-apple-darwin',
    'x86_64-apple-darwin',
    'x86_64-unknown-linux-musl',
    'x86_64-unknown-linux-gnu',
    'aarch64-unknown-linux-musl',
    'x86_64-pc-windows-msvc',
    'aarch64-pc-windows-msvc',
  ]
  return targets.find((target) =>
    name.endsWith(`-${target}.tar.gz`) || name.endsWith(`-${target}.zip`),
  ) || null
}

function inferAssetMetadata(name) {
  const target = idriveTarget(name)
  if (target) {
    return {
      target,
      kind: 'binary-archive',
      executable: target.includes('windows') ? 'idrive.exe' : 'idrive',
    }
  }
  if (/^iris-drive-v.*-macos-arm64\.app\.tar\.gz$/.test(name)) {
    return {
      target: 'darwin-aarch64',
      kind: 'app-bundle',
      executable: 'Iris Drive.app',
    }
  }
  if (/\.dmg$/.test(name)) {
    return { kind: 'archive' }
  }
  if (/\.AppImage$/.test(name)) {
    return { kind: 'appimage' }
  }
  if (/\.deb$/.test(name)) {
    return { kind: 'deb' }
  }
  if (/-setup\.exe$/.test(name)) {
    return { kind: 'nsis' }
  }
  if (/\.apk$/.test(name)) {
    return { kind: 'archive' }
  }
  return {}
}

export function buildReleaseManifest({ tag, commit, createdAt, assetPaths, draft = false }) {
  const normalizedTag = normalizeTag(tag)
  const version = semverFromTag(normalizedTag)
  const assets = [...assetPaths]
    .map((assetPath) => {
      const name = basename(assetPath)
      return {
        name,
        path: `assets/${name}`,
        size: statSync(assetPath).size,
        ...inferAssetMetadata(name),
      }
    })
    .sort((left, right) => left.name.localeCompare(right.name))

  return {
    schema: 'hashtree-update-manifest-v1',
    app: 'iris-drive',
    version,
    id: normalizedTag,
    title: normalizedTag,
    tag: normalizedTag,
    commit,
    created_at: createdAt,
    published_at: createdAt,
    draft,
    prerelease: normalizedTag.includes('-'),
    notes_file: 'notes.md',
    assets,
  }
}

export function buildReleaseManifestFiles(manifest) {
  const text = `${JSON.stringify(manifest, null, 2)}\n`
  return [
    ['release.json', text],
    ['manifest.json', text],
  ]
}

function assetReference(name) {
  return `[${name}](assets/${encodeURIComponent(name)})`
}

function firstMatchingAsset(assetNames, patterns) {
  return assetNames.find((name) => patterns.some((pattern) => pattern.test(name))) ?? null
}

function pushAssetLine(lines, usedAssets, assetNames, patterns) {
  const name = firstMatchingAsset(assetNames, patterns)
  if (!name) {
    return null
  }
  usedAssets.add(name)
  lines.push(`- ${describeAsset(name)}: ${assetReference(name)}`)
  return name
}

function markMatchingAssetsUsed(usedAssets, assetNames, patterns) {
  for (const name of assetNames) {
    if (patterns.some((pattern) => pattern.test(name))) {
      usedAssets.add(name)
    }
  }
}

function pushDownloadSections(lines, assetNames) {
  const sortedNames = [...assetNames].sort((left, right) => left.localeCompare(right))
  const usedAssets = new Set()

  lines.push('## Downloads', '', '### Most People Will Want', '')

  pushAssetLine(lines, usedAssets, sortedNames, [
    /^iris-drive-v.*-macos-arm64\.dmg$/,
  ])
  pushAssetLine(lines, usedAssets, sortedNames, [
    /^iris-drive-v.*-linux-x64\.AppImage$/,
  ])
  pushAssetLine(lines, usedAssets, sortedNames, [
    /^iris-drive-v.*-linux-x64\.deb$/,
  ])
  pushAssetLine(lines, usedAssets, sortedNames, [
    /^iris-drive-v.*-windows-x64-setup\.exe$/,
  ])
  pushAssetLine(lines, usedAssets, sortedNames, [
    /^iris-drive-v.*-android-arm64\.apk$/,
  ])

  const cliLines = []
  const addCliAsset = (preferredPatterns, duplicatePatterns = preferredPatterns) => {
    const name = firstMatchingAsset(sortedNames, preferredPatterns)
    if (!name) {
      return
    }
    usedAssets.add(name)
    markMatchingAssetsUsed(usedAssets, sortedNames, duplicatePatterns)
    cliLines.push(`- ${describeAsset(name)}: ${assetReference(name)}`)
  }

  addCliAsset([
    /^idrive-v.*-aarch64-apple-darwin\.tar\.gz$/,
  ])
  addCliAsset([
    /^idrive-v.*-x86_64-apple-darwin\.tar\.gz$/,
  ])
  addCliAsset([
    /^idrive-v.*-x86_64-unknown-linux-gnu\.tar\.gz$/,
    /^idrive-v.*-x86_64-unknown-linux-musl\.tar\.gz$/,
  ], [/^idrive-v.*-x86_64-unknown-linux-(gnu|musl)\.tar\.gz$/])
  addCliAsset([
    /^idrive-v.*-aarch64-unknown-linux-musl\.tar\.gz$/,
  ])
  addCliAsset([
    /^idrive-v.*-x86_64-pc-windows-msvc\.zip$/,
  ])
  addCliAsset([
    /^idrive-v.*-aarch64-pc-windows-msvc\.zip$/,
  ])

  if (cliLines.length > 0) {
    lines.push('', '### Command Line', '', ...cliLines)
  }

  const otherLines = []
  for (const name of sortedNames) {
    if (usedAssets.has(name)) {
      continue
    }
    otherLines.push(`- ${describeAsset(name)}: ${assetReference(name)}`)
  }

  if (otherLines.length > 0) {
    lines.push('', '### Other Files', '', ...otherLines)
  }
}

export function renderReleaseNotes({ tag, commit, assetNames }) {
  const lines = [`# Iris Drive ${normalizeTag(tag)}`, '']
  pushDownloadSections(lines, assetNames)
  if (commit) {
    lines.push('', '## Release Build', '', `- Built from commit \`${commit}\`.`)
  }
  return `${lines.join('\n')}\n`
}
