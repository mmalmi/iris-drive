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
    return 'Windows x64 idrive CLI'
  }
  if (/^idrive-v.*-aarch64-pc-windows-msvc\.zip$/.test(name)) {
    return 'Windows ARM64 idrive CLI'
  }
  if (/^iris-drive-v.*-macos-arm64\.dmg$/.test(name)) {
    return 'Iris Drive for macOS'
  }
  if (/^iris-drive-v.*-linux-x64\.(AppImage|deb)$/.test(name)) {
    return 'Iris Drive for Linux x64'
  }
  if (/^iris-drive-v.*-windows-x64-setup\.exe$/.test(name)) {
    return 'Iris Drive for Windows'
  }
  if (/^iris-drive-v.*-android-arm64\.apk$/.test(name)) {
    return 'Iris Drive for Android'
  }
  return name
}

export function validateReleaseAssetSet(
  assetNames,
  { requireCompleteAppRelease = false } = {},
) {
  const names = [...assetNames]
  const hasMacosDmg = names.some((name) => /^iris-drive-v.*-macos-arm64\.dmg$/.test(name))
  const hasLinuxX64Desktop = names.some((name) =>
    /^iris-drive-v.*-linux-x64\.(AppImage|deb)$/.test(name),
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

  if (requireCompleteAppRelease) {
    const missing = []
    if (!hasMacosDmg) {
      missing.push('macOS DMG')
    }
    if (!hasLinuxX64Desktop) {
      missing.push('Linux x64 desktop package')
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

export function renderReleaseNotes({ tag, commit, assetNames }) {
  const lines = [`# Iris Drive ${normalizeTag(tag)}`, '', '## Downloads', '']
  for (const name of [...assetNames].sort((left, right) => left.localeCompare(right))) {
    lines.push(`- ${describeAsset(name)}: [${name}](assets/${encodeURIComponent(name)})`)
  }
  if (commit) {
    lines.push('', '## Release Build', '', `- Built from commit \`${commit}\`.`)
  }
  return `${lines.join('\n')}\n`
}
