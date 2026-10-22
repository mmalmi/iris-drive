import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import {
  buildNumberFromVersion,
  bumpCargoPackageVersion,
  bumpHapsManifestVersions,
  bumpWorkspaceVersion,
  bumpPbxprojReleaseVersions,
  bumpXcodegenProjectVersions,
  readWorkspaceVersionTag,
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

test('release build numbers are strictly increasing and collision-free', () => {
  assert.deepEqual(
    ['v0.1.32', 'v0.1.33', 'v0.1.34', 'v0.1.999', 'v0.2.0'].map(buildNumberFromVersion),
    ['1033', '1034', '1035', '2000', '2001'],
  )
  assert.throws(() => buildNumberFromVersion('v0.1.33-rc.1'), /stable release/)
  assert.throws(() => buildNumberFromVersion('v0.1.1000'), /at most 999/)
  assert.throws(() => buildNumberFromVersion('v0.1000.0'), /at most 999/)
})

test('release version helpers sync native platform metadata', () => {
  assert.equal(bumpWorkspaceVersion(
    '[workspace.package]\nversion = "0.1.39"\n[workspace.dependencies]\niris-drive-core = { version = "0.1.39", path = "crates/iris-drive-core" }\n',
    'v0.1.40',
  ), '[workspace.package]\nversion = "0.1.40"\n[workspace.dependencies]\niris-drive-core = { version = "0.1.40", path = "crates/iris-drive-core" }\n')
  assert.equal(
    bumpCargoPackageVersion('[package]\nname = "iris-drive-linux"\nversion = "0.1.4"\n\n[dependencies]\n', 'v0.1.5'),
    '[package]\nname = "iris-drive-linux"\nversion = "0.1.5"\n\n[dependencies]\n',
  )
  assert.equal(
    bumpXcodegenProjectVersions(
      'MARKETING_VERSION: "0.1.4"\nCURRENT_PROJECT_VERSION: "1004"\n',
      'v0.1.5',
    ),
    'MARKETING_VERSION: "0.1.5"\nCURRENT_PROJECT_VERSION: "1006"\n',
  )
  assert.equal(
    bumpPbxprojReleaseVersions('MARKETING_VERSION = 0.1.4;\nCURRENT_PROJECT_VERSION = 1004;\n', 'v0.1.5'),
    'MARKETING_VERSION = 0.1.5;\nCURRENT_PROJECT_VERSION = 1006;\n',
  )
})

test('release versions update every Haps package without changing dependency versions', () => {
  const input = `# https://haps.hashtree.cc
[package]
name = "iris-drive"
version = "0.1.39"
[package.dependencies.helper]
version = "0.2.0"
[packages.idrive.package]
name = "idrive"
version = "0.1.39"
[targets."aarch64-apple-darwin".package]
version = "0.1.39"
[build]
commands = [["cargo", "build", "--locked"]]
`
  const next = bumpHapsManifestVersions(input, 'v0.1.40')
  assert.equal((next.match(/version = "0.1.40"/g) ?? []).length, 3)
  assert.match(next, /version = "0.2.0"/)
  assert.match(next, /commands = \[\["cargo", "build", "--locked"\]\]/)
  assert.ok(next.startsWith('# https://haps.hashtree.cc\n'))
  assert.equal(bumpHapsManifestVersions(next, 'v0.1.40'), next)
  assert.throws(() => bumpHapsManifestVersions('[package]\nname = "iris-drive"\n', 'v0.1.40'), /version/)
})

test('platform version preview includes Haps packages and leaves the manifest unchanged', () => {
  const manifest = new URL('../haps.toml', import.meta.url)
  const before = readFileSync(manifest, 'utf8')
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
    '--build', '--only', 'platform-versions', '--tag', 'v9.9.9', '--dry-run',
  ], { encoding: 'utf8' })
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Synced platform versions to v9\.9\.9:.*haps\.toml/)
  assert.equal(readFileSync(manifest, 'utf8'), before)
})

test('standalone iOS release paths share the canonical build sequence', () => {
  const iosBuild = readFileSync(fileURLToPath(new URL('./ios-build', import.meta.url)), 'utf8')
  const testflight = readFileSync(
    fileURLToPath(new URL('./testflight-app-store-connect.mjs', import.meta.url)),
    'utf8',
  )
  const cli = spawnSync(
    process.execPath,
    [fileURLToPath(new URL('./release-build-number.mjs', import.meta.url)), 'v0.1.33'],
    { encoding: 'utf8' },
  )

  assert.equal(cli.status, 0, cli.stderr)
  assert.equal(cli.stdout.trim(), '1034')
  assert.match(iosBuild, /node "\$ROOT\/scripts\/release-build-number\.mjs" "\$1"/)
  assert.match(testflight, /buildNumberFromVersion\(versionName\)/)
  assert.doesNotMatch(testflight, /function semanticVersionCode/)
})
