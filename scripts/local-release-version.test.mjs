import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import {
  buildNumberFromVersion,
  bumpCargoPackageVersion,
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
