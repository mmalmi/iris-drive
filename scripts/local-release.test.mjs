import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'

import {
  buildReleaseManifest,
  buildReleaseManifestFiles,
  buildZapstorePublishPlan,
  describeAsset,
  plannedReleaseAssetNames,
  readWorkspaceVersionTag,
  validateReleaseAssetSet,
} from './local-release-lib.mjs'

test('readWorkspaceVersionTag reads the workspace package version', () => {
  const tag = readWorkspaceVersionTag(`
[workspace]
members = []

[workspace.package]
version = "0.2.27"
`)

  assert.equal(tag, 'v0.2.27')
})

test('buildReleaseManifest marks idrive archives as binary archives', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-test-'))
  const cli = join(root, 'idrive-v0.2.27-x86_64-unknown-linux-musl.tar.gz')
  writeFileSync(cli, 'archive')

  const manifest = buildReleaseManifest({
    tag: 'v0.2.27',
    commit: 'abc123',
    createdAt: 1774523304,
    assetPaths: [cli],
  })

  assert.equal(manifest.app, 'iris-drive')
  assert.equal(manifest.version, '0.2.27')
  assert.equal(manifest.assets[0].kind, 'binary-archive')
  assert.equal(manifest.assets[0].executable, 'idrive')
  assert.equal(manifest.assets[0].target, 'x86_64-unknown-linux-musl')
})

test('buildReleaseManifestFiles writes both updater manifest names', () => {
  const files = buildReleaseManifestFiles({
    app: 'iris-drive',
    version: '0.2.27',
    tag: 'v0.2.27',
    assets: [],
  })

  assert.deepEqual(files.map(([name]) => name), ['release.json', 'manifest.json'])
})

test('describeAsset labels idrive release assets', () => {
  assert.equal(
    describeAsset('idrive-v0.2.27-x86_64-pc-windows-msvc.zip'),
    'Windows x64 idrive CLI',
  )
  assert.equal(
    describeAsset('idrive-v0.2.27-x86_64-unknown-linux-gnu.tar.gz'),
    'Linux x64 idrive CLI',
  )
})

test('buildZapstorePublishPlan fails on missing required publish inputs', () => {
  const distRoot = join(tmpdir(), 'iris-drive-dist')
  const base = {
    tag: '0.2.27',
    assetDir: distRoot,
    distDir: distRoot,
    apkExists: true,
    zspAvailable: true,
    signWith: 'nsec1example',
    zapstoreYamlExists: true,
  }

  assert.throws(
    () => buildZapstorePublishPlan({ ...base, apkExists: false }),
    /Missing Android APK/,
  )
  assert.throws(
    () => buildZapstorePublishPlan({ ...base, zspAvailable: false }),
    /Missing zsp/,
  )
  assert.throws(
    () => buildZapstorePublishPlan({ ...base, signWith: '' }),
    /Missing Zapstore signing key/,
  )
})

test('buildZapstorePublishPlan resolves the Zapstore release source path', () => {
  const distRoot = join(tmpdir(), 'iris-drive-dist')
  const plan = buildZapstorePublishPlan({
    tag: '0.2.27',
    assetDir: distRoot,
    distDir: distRoot,
    apkExists: true,
    zspAvailable: true,
    signWith: 'nsec1example',
    zapstoreYamlExists: true,
  })

  assert.equal(plan.apkName, 'iris-drive-v0.2.27-android-arm64.apk')
  assert.equal(plan.apkPath, join(distRoot, 'iris-drive-v0.2.27-android-arm64.apk'))
  assert.equal(plan.releaseSourcePath, join(distRoot, 'zapstore-current-android-arm64.apk'))
})

test('validateReleaseAssetSet requires complete public app artifacts for final releases', () => {
  assert.throws(
    () => validateReleaseAssetSet(['idrive-v0.2.27-aarch64-apple-darwin.tar.gz'], {
      requireCompleteAppRelease: true,
    }),
    /macOS DMG.*Linux x64 desktop package.*Windows x64 installer.*signed Android APK/,
  )
})

test('validateReleaseAssetSet rejects unsigned Android public artifacts', () => {
  assert.throws(
    () => validateReleaseAssetSet(['iris-drive-v0.2.27-android-arm64-unsigned.apk']),
    /unsigned Android/,
  )
})

test('plannedReleaseAssetNames names the public release artifacts', () => {
  assert.deepEqual(plannedReleaseAssetNames('0.2.27', ['macos', 'linux', 'windows', 'android']), [
    'idrive-v0.2.27-aarch64-apple-darwin.tar.gz',
    'iris-drive-v0.2.27-macos-arm64.dmg',
    'idrive-v0.2.27-x86_64-unknown-linux-gnu.tar.gz',
    'iris-drive-v0.2.27-linux-x64.deb',
    'idrive-v0.2.27-x86_64-pc-windows-msvc.zip',
    'iris-drive-v0.2.27-windows-x64-setup.exe',
    'iris-drive-v0.2.27-android-arm64.apk',
    'iris-drive-v0.2.27-android-arm64.aab',
  ])
})

test('plannedReleaseAssetNames covers the complete final release validator', () => {
  const names = plannedReleaseAssetNames('v0.2.27', ['macos', 'linux', 'windows', 'android'])
  assert.doesNotThrow(() => validateReleaseAssetSet(names, { requireCompleteAppRelease: true }))
})

test('local-release dry-run validates planned build assets over partial existing dist assets', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-cli-test-'))
  const assetDir = join(root, 'dist')
  const stageDir = join(root, 'stage')
  mkdirSync(assetDir)
  writeFileSync(join(assetDir, 'iris-drive-v9.9.9-macos-arm64.dmg'), 'partial')

  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--publish',
      '--final',
      '--dry-run',
      '--skip-zapstore',
      '--tag',
      'v9.9.9',
      '--only',
      'macos,linux,windows,android',
      '--asset-dir',
      assetDir,
      '--stage-dir',
      stageDir,
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        ANDROID_KEYSTORE_PATH: '/tmp/iris-drive-test.jks',
        ANDROID_KEYSTORE_PASSWORD: 'password',
        ANDROID_KEY_ALIAS: 'iris',
        ANDROID_KEY_PASSWORD: 'password',
        IRIS_DRIVE_WINDOWS_INSTALLER_PATH: '/tmp/IrisDriveSetup.exe',
      },
    },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Would stage 8 planned asset\(s\)/)
})

test('local-release build-only mode does not stage existing unsigned artifacts', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-build-only-test-'))
  const assetDir = join(root, 'dist')
  mkdirSync(assetDir)
  writeFileSync(join(assetDir, 'iris-drive-v9.9.9-android-arm64-unsigned.apk'), 'unsigned')

  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--tag',
      'v9.9.9',
      '--only',
      '',
      '--asset-dir',
      assetDir,
    ],
    { encoding: 'utf8' },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Release build steps: \(none\)/)
})

test('local-release dry-run routes iOS builds through the TestFlight script', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--dry-run',
      '--only',
      'ios',
      '--tag',
      'v9.9.9',
    ],
    { encoding: 'utf8' },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /scripts\/ios-build ios-testflight/)
  assert.doesNotMatch(result.stdout, /Would archive\/export\/upload/)
})

test('local-release dry-run uses a public-capable iOS upload for internal plus public TestFlight', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--dry-run',
      '--only',
      'ios',
      '--tag',
      'v9.9.9',
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        IRIS_DRIVE_IOS_TESTFLIGHT_CHANNELS: 'internal,public',
      },
    },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /scripts\/ios-build ios-testflight-public/)
})

test('TestFlight helper documents iris-drive App Store Connect inputs', () => {
  const result = spawnSync('bash', ['scripts/testflight-internal', '--help'], {
    cwd: fileURLToPath(new URL('..', import.meta.url)),
    encoding: 'utf8',
  })

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /IRIS_DRIVE_ASC_AUTH_KEY_PATH/)
  assert.match(result.stdout, /IRIS_DRIVE_TESTFLIGHT_GROUPS/)
})
