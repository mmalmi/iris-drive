import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import {
  assertNoPrivateBuildMetadata,
  normalizedTarOwnerArgs,
  packageUnixCliTarball,
  releaseRustBuildEnvironment,
} from './release-build-hygiene.mjs'

test('release Rust builds remap workspace and home paths', () => {
  const env = releaseRustBuildEnvironment(
    { RUSTFLAGS: '-C target-cpu=generic' },
    { repoRoot: '/Users/private/src/iris-drive', homeDir: '/Users/private' },
  )

  assert.match(env.RUSTFLAGS, /-C target-cpu=generic/)
  assert.match(env.RUSTFLAGS, /--remap-path-prefix=\/Users\/private\/src\/iris-drive=\/usr\/src\/iris-drive/)
  assert.match(env.RUSTFLAGS, /--remap-path-prefix=\/Users\/private=\/usr\/src\/home/)
})

test('release Rust builds preserve encoded flags', () => {
  const env = releaseRustBuildEnvironment(
    { CARGO_ENCODED_RUSTFLAGS: '-C\u001ftarget-cpu=generic' },
    { repoRoot: '/home/private/iris-drive', homeDir: '/home/private' },
  )

  assert.equal(env.CARGO_ENCODED_RUSTFLAGS.split('\u001f')[0], '-C')
  assert.match(env.CARGO_ENCODED_RUSTFLAGS, /--remap-path-prefix=\/home\/private=\/usr\/src\/home/)
})

test('release binary audit rejects private build paths without echoing them', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-hygiene-'))
  const binary = join(root, 'idrive')
  const privateHome = '/home/private-builder'
  writeFileSync(binary, `binary-prefix ${privateHome}/.cargo/registry binary-suffix`)

  assert.throws(
    () => assertNoPrivateBuildMetadata([binary], {}, { repoRoot: root, homeDir: privateHome }),
    (error) => {
      assert.match(error.message, /private build metadata/)
      assert.doesNotMatch(error.message, /private-builder/)
      return true
    },
  )
})

test('release tar ownership is generic on GNU and BSD tar', () => {
  assert.deepEqual(normalizedTarOwnerArgs('linux'), [
    '--owner=0',
    '--group=0',
    '--numeric-owner',
  ])
  assert.deepEqual(normalizedTarOwnerArgs('darwin'), [
    '--uid',
    '0',
    '--gid',
    '0',
    '--uname',
    'root',
    '--gname',
    'root',
  ])
})

test('direct iOS and Windows release entry points remap Rust paths', () => {
  const ios = readFileSync(new URL('./ios-build', import.meta.url), 'utf8')
  const windows = readFileSync(new URL('./windows-publish.ps1', import.meta.url), 'utf8')

  for (const script of [ios, windows]) {
    assert.match(script, /--remap-path-prefix=/)
    assert.match(script, /\/usr\/src\/iris-drive/)
    assert.match(script, /\/usr\/src\/home/)
  }
  assert.match(ios, /release-build-hygiene\.mjs/)
})

test('Unix CLI packaging produces normalized tar metadata', { skip: process.platform === 'win32' }, () => {
  const distDir = mkdtempSync(join(tmpdir(), 'iris-drive-release-tar-'))
  const run = (command, args) => {
    const result = spawnSync(command, args, { encoding: 'utf8' })
    assert.equal(result.status, 0, result.stderr)
  }
  packageUnixCliTarball({
    binaryPath: '/bin/echo',
    distDir,
    dryRun: false,
    env: {},
    run,
    tag: 'v0.0.0',
    targetTriple: 'test-target',
  })
  assert.equal(existsSync(join(distDir, 'idrive-v0.0.0-test-target.tar.gz')), true)
})
