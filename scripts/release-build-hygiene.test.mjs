import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

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

test('release native builds safely remap workspace and home paths', () => {
  const repoRoot = '/Users/private person/src/iris drive'
  const homeDir = '/Users/private person'
  const flags = [
    `'-ffile-prefix-map=${repoRoot}=/usr/src/iris-drive'`,
    `'-fdebug-prefix-map=${repoRoot}=/usr/src/iris-drive'`,
    `'-ffile-prefix-map=${homeDir}=/usr/src/home'`,
    `'-fdebug-prefix-map=${homeDir}=/usr/src/home'`,
  ].join(' ')

  for (const platform of ['darwin', 'linux']) {
    const env = releaseRustBuildEnvironment(
      { CFLAGS: '-O2', CXXFLAGS: '-stdlib=libc++' },
      { repoRoot, homeDir, platform },
    )
    assert.equal(env.CFLAGS, `-O2 ${flags}`)
    assert.equal(env.CXXFLAGS, `-stdlib=libc++ ${flags}`)
    assert.equal(env.CC_SHELL_ESCAPED_FLAGS, '1')
  }
})

test('release Rust path remapping is preserved for Windows builds', () => {
  const env = releaseRustBuildEnvironment(
    { CFLAGS: '/O2', CXXFLAGS: '/EHsc' },
    {
      repoRoot: 'C:\\private\\iris-drive',
      homeDir: 'C:\\private',
      platform: 'win32',
    },
  )

  assert.equal(env.CFLAGS, '/O2')
  assert.equal(env.CXXFLAGS, '/EHsc')
  assert.match(env.CARGO_ENCODED_RUSTFLAGS, /--remap-path-prefix=C:\\private\\iris-drive=/)
  assert.equal(env.CC_SHELL_ESCAPED_FLAGS, undefined)
})

test('Windows publisher maps native paths with clang-cl', () => {
  const windows = readFileSync(new URL('./windows-publish.ps1', import.meta.url), 'utf8')

  assert.match(windows, /Resolve-ClangCl/)
  assert.match(windows, /CC_x86_64_pc_windows_msvc/)
  assert.match(windows, /CXX_x86_64_pc_windows_msvc/)
  assert.match(windows, /CFLAGS_x86_64_pc_windows_msvc/)
  assert.match(windows, /CXXFLAGS_x86_64_pc_windows_msvc/)
  assert.match(windows, /CC_SHELL_ESCAPED_FLAGS/)
  assert.match(windows, /\/clang:-Werror=unknown-argument/)
  assert.match(windows, /\/clang:-ffile-prefix-map=/)
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

test('Windows binary audit catches case, slash, and UTF-16 path variants', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-hygiene-windows-'))
  const binary = join(root, 'idrive.exe')
  const privateRoot = String.raw`C:\Users\Private Builder\src\Iris Drive`
  writeFileSync(binary, Buffer.from('c:/users/private builder/src/iris drive/native.c', 'utf16le'))

  assert.throws(
    () => assertNoPrivateBuildMetadata([binary], {}, {
      repoRoot: privateRoot,
      homeDir: String.raw`C:\Users\Private Builder`,
      hostname: 'PRIVATE-BUILDER',
      platform: 'win32',
    }),
    (error) => {
      assert.match(error.message, /private build metadata/)
      assert.doesNotMatch(error.message, /private builder/i)
      return true
    },
  )
})

test('payload audit scans nested files without disclosing private paths', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-payload-'))
  const payload = join(root, 'payload', 'nested')
  mkdirSync(payload, { recursive: true })
  writeFileSync(join(payload, 'library.bin'), `embedded ${root} metadata`)

  const result = spawnSync(
    process.execPath,
    [fileURLToPath(new URL('./release-build-hygiene-cli.mjs', import.meta.url)), root, join(root, 'payload')],
    { encoding: 'utf8' },
  )
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /privacy audit failed/)
  assert.doesNotMatch(result.stderr, new RegExp(root.replaceAll(/[.*+?^${}()|[\]\\]/g, '\\$&')))
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

test('Linux release lookup keeps checkout paths out of production code', () => {
  const source = readFileSync(new URL('../linux/src/daemon_control.rs', import.meta.url), 'utf8')
  assert.equal([...source.matchAll(/env!\("CARGO_MANIFEST_DIR"\)/g)].length, 1)
  assert.match(
    source,
    /#\[cfg\(debug_assertions\)\]\s*fn debug_checkout_idrive_path[\s\S]*?env!\("CARGO_MANIFEST_DIR"\)/,
  )
  assert.match(source, /std::env::current_exe\(\)/)
  assert.match(source, /with_file_name\("idrive"\)/)
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
