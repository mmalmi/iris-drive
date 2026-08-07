import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'

import { canonicalReleaseAssetNames } from './local-release-lib.mjs'

const tag = 'v9.9.9'
const releaseScript = fileURLToPath(new URL('./local-release.mjs', import.meta.url))

function createAssetDir(root, { directory = '', extra = '' } = {}) {
  const assetDir = join(root, 'dist')
  mkdirSync(assetDir)
  for (const name of canonicalReleaseAssetNames(tag)) {
    if (name === directory) {
      mkdirSync(join(assetDir, name))
    } else {
      writeFileSync(join(assetDir, name), name)
    }
  }
  if (extra) {
    writeFileSync(join(assetDir, extra), 'unexpected release residue')
  }
  return assetDir
}

function finalDryRun(root, assetDir) {
  return spawnSync(
    process.execPath,
    [
      releaseScript,
      '--final',
      '--dry-run',
      '--skip-zapstore',
      '--tag',
      tag,
      '--asset-dir',
      assetDir,
      '--stage-dir',
      join(root, 'stage'),
    ],
    { encoding: 'utf8' },
  )
}

test('final release requires every canonical asset to be a regular file', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-file-test-'))
  const missing = 'iris-drive-v9.9.9-android-arm64.aab'
  const result = finalDryRun(root, createAssetDir(root, { directory: missing }))

  assert.equal(result.status, 1)
  assert.match(result.stderr, new RegExp(`Missing canonical release asset\\(s\\): ${missing}`))
  assert.doesNotMatch(result.stdout, /htree release publish/)
})

test('final release rejects tag-matching intermediate residue', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-residue-test-'))
  const extra = 'iris-drive-v9.9.9-macos-arm64.unsigned-intermediate.app.tar.gz'
  const result = finalDryRun(root, createAssetDir(root, { extra }))

  assert.equal(result.status, 1)
  assert.match(result.stderr, new RegExp(`Unexpected tag-matching release asset\\(s\\): ${extra}`))
  assert.doesNotMatch(result.stdout, /htree release publish/)
})
