import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { plannedReleaseAssetNames, validateReleaseAssetSet } from './local-release-lib.mjs'

test('project policy records unsigned Windows releases', () => {
  const policy = JSON.parse(
    readFileSync(new URL('../release-policy.json', import.meta.url), 'utf8'),
  )
  assert.equal(policy.windows.signing, 'unsigned')
})

test('final releases require the Windows CLI archive beside the installer', () => {
  const names = plannedReleaseAssetNames('v9.9.9', ['macos', 'linux', 'windows', 'android'])
    .filter((name) => !name.endsWith('-x86_64-pc-windows-msvc.zip'))
  assert.throws(() => validateReleaseAssetSet(names, { requireCompleteAppRelease: true }), /Windows x64 CLI archive/)
})

test('final release dry-run builds required Windows artifacts without signing inputs', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-windows-release-policy-test-'))
  const keystorePath = join(root, 'upload-keystore.jks')
  writeFileSync(keystorePath, 'test keystore placeholder')
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--final',
      '--dry-run',
      '--skip-zapstore',
      '--tag',
      'v9.9.9',
      '--only',
      'macos,linux,windows,android',
    ],
    {
      encoding: 'utf8',
      env: {
        PATH: process.env.PATH,
        HOME: process.env.HOME,
        ANDROID_KEYSTORE_PATH: keystorePath,
        ANDROID_KEYSTORE_PASSWORD: 'password',
        ANDROID_KEY_ALIAS: 'iris',
        ANDROID_KEY_PASSWORD: 'password',
        IRIS_DRIVE_MACOS_NOTARY_KEYCHAIN_PROFILE: 'iris-drive-notary',
      },
    },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Windows release signing: unsigned/)
  assert.doesNotMatch(result.stdout, /-RequireSigning/)
  assert.match(result.stdout, /idrive-v9\.9\.9-x86_64-pc-windows-msvc\.zip/)
  assert.match(result.stdout, /iris-drive-v9\.9\.9-windows-x64-setup\.exe/)
})
