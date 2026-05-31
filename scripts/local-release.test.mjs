import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import {
  buildReleaseManifest,
  buildReleaseManifestFiles,
  buildZapstorePublishPlan,
  describeAsset,
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
