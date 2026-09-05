import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { gunzipSync } from 'node:zlib'

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

test('direct iOS and Windows release entry points remap compiler paths', () => {
  const ios = readFileSync(new URL('./ios-build', import.meta.url), 'utf8')
  const windows = readFileSync(new URL('./windows-publish.ps1', import.meta.url), 'utf8')

  for (const script of [ios, windows]) {
    assert.match(script, /--remap-path-prefix=/)
    assert.match(script, /\/usr\/src\/iris-drive/)
    assert.match(script, /\/usr\/src\/home/)
  }
  assert.match(ios, /release-build-hygiene\.mjs/)
  assert.match(ios, /CC_SHELL_ESCAPED_FLAGS/)
  assert.match(ios, /-ffile-prefix-map=/)
  assert.match(ios, /-fdebug-prefix-map=/)
  const settings = ios.match(/build_settings_args\(\) \{([\s\S]*?)\n\}/)?.[1] ?? ''
  assert.equal([...settings.matchAll(/swift_path_maps\+?=.*-debug-prefix-map/g)].length, 2)
  assert.equal([...settings.matchAll(/clang_path_maps\+?=.*-ffile-prefix-map/g)].length, 2)
  assert.equal([...settings.matchAll(/clang_path_maps\+?=.*-fdebug-prefix-map/g)].length, 2)
  assert.match(settings, /OTHER_SWIFT_FLAGS=.*\$swift_path_maps/)
  assert.match(settings, /OTHER_CFLAGS=.*\$clang_path_maps/)
  assert.match(settings, /OTHER_CPLUSPLUSFLAGS=.*\$clang_path_maps/)
  assert.match(
    ios,
    /run_ios_rust\(\)[\s\S]*ensure_release_native_path_remap[\s\S]*cargo build/,
  )

  const archive = ios.match(/run_ios_archive\(\) \{([\s\S]*?)\n\}/)?.[1] ?? ''
  assert.equal(
    [...archive.matchAll(/"\$\{build_args\[@\]\}"/g)].length,
    2,
    'automatic and manual archives must use the same common build settings',
  )
})

test('direct iOS remaps complete partial environments idempotently', {
  skip: process.platform === 'win32',
}, () => {
  const ios = readFileSync(new URL('./ios-build', import.meta.url), 'utf8')
  const helpers = ios.slice(
    ios.indexOf('ensure_release_rust_path_remap()'),
    ios.indexOf('asc_issuer_id()'),
  )
  const repoRoot = '/workspace/iris-drive'
  const home = '/home/release-builder'
  const expected = [
    `--remap-path-prefix=${repoRoot}=/usr/src/iris-drive`,
    `--remap-path-prefix=${home}=/usr/src/home`,
    `-ffile-prefix-map=${repoRoot}=/usr/src/iris-drive`,
    `-fdebug-prefix-map=${repoRoot}=/usr/src/iris-drive`,
    `-ffile-prefix-map=${home}=/usr/src/home`,
    `-fdebug-prefix-map=${home}=/usr/src/home`,
  ]
  const result = spawnSync('bash', ['-c', `${helpers}
ensure_release_rust_path_remap
ensure_release_native_path_remap
ensure_release_rust_path_remap
ensure_release_native_path_remap
printf '%s\\n%s\\n%s\\n%s\\n' "$CARGO_ENCODED_RUSTFLAGS" "$CFLAGS" "$CXXFLAGS" "$CC_SHELL_ESCAPED_FLAGS"
`], {
    encoding: 'utf8',
    env: {
      ...process.env,
      ROOT: repoRoot,
      HOME: home,
      CARGO_ENCODED_RUSTFLAGS: ['-C', 'target-cpu=generic'].join('\u001f'),
      RUSTFLAGS: expected[0],
      CFLAGS: `-O2 '${expected[2]}'`,
      CXXFLAGS: `-stdlib=libc++ '${expected[3]}'`,
    },
  })

  assert.equal(result.status, 0, result.stderr)
  const [rustFlags, cFlags, cxxFlags, shellEscaped] = result.stdout.trimEnd().split('\n')
  for (const flag of expected.slice(0, 2)) {
    assert.equal(rustFlags.split(flag).length - 1, 1)
  }
  for (const flags of [cFlags, cxxFlags]) {
    for (const flag of expected.slice(2)) {
      assert.equal(flags.split(flag).length - 1, 1)
    }
  }
  assert.equal(shellEscaped, '1')
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

test('Linux Debian metadata carries the project MIT license', () => {
  const licensePath = new URL('../LICENSE', import.meta.url)
  const manifestPath = fileURLToPath(new URL('../linux/Cargo.toml', import.meta.url))
  const manifest = readFileSync(manifestPath, 'utf8')
  const metadata = spawnSync(
    'cargo',
    ['metadata', '--manifest-path', manifestPath, '--no-deps', '--format-version', '1'],
    { encoding: 'utf8' },
  )

  assert.equal(metadata.status, 0, metadata.stderr)
  const [packageMetadata] = JSON.parse(metadata.stdout).packages
  assert.equal(packageMetadata.license, 'MIT')
  assert.deepEqual(packageMetadata.authors, ['Iris Drive contributors'])
  assert.equal(packageMetadata.repository, 'htree://self/iris-drive')
  assert.equal(existsSync(licensePath), true)

  const license = readFileSync(licensePath, 'utf8')
  assert.match(license, /Permission is hereby granted, free of charge/)
  assert.match(license, /THE SOFTWARE IS PROVIDED "AS IS"/)
  assert.match(
    manifest,
    /copyright = "2026 Iris Drive contributors"/,
  )
  assert.match(manifest, /license-file = \["\.\.\/LICENSE", "0"\]/)
})

test('Linux Debian acceptance requires the exact project license', { skip: process.platform === 'win32' }, (t) => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-deb-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const dataRoot = join(root, 'data')
  const copyrightDir = join(dataRoot, 'usr/share/doc/iris-drive')
  const licensePath = fileURLToPath(new URL('../LICENSE', import.meta.url))
  const verifier = fileURLToPath(new URL('./verify-linux-deb-license.mjs', import.meta.url))

  const arMember = (name, contents) => {
    const bytes = Buffer.from(contents)
    const header = Buffer.alloc(60, ' ')
    header.write(`${name}/`, 0, 16, 'ascii')
    header.write('0', 16, 12, 'ascii')
    header.write('0', 28, 6, 'ascii')
    header.write('0', 34, 6, 'ascii')
    header.write('100644', 40, 8, 'ascii')
    header.write(String(bytes.length), 48, 10, 'ascii')
    header.write('`\n', 58, 2, 'ascii')
    return Buffer.concat([header, bytes, bytes.length % 2 ? Buffer.from('\n') : Buffer.alloc(0)])
  }

  const buildDeb = (name, copyright) => {
    rmSync(dataRoot, { recursive: true, force: true })
    mkdirSync(copyrightDir, { recursive: true })
    writeFileSync(join(copyrightDir, 'copyright'), copyright)
    writeFileSync(join(root, 'debian-binary'), '2.0\n')
    const controlRoot = join(root, 'control')
    mkdirSync(controlRoot, { recursive: true })
    writeFileSync(join(controlRoot, 'control'), 'Package: iris-drive\n')
    for (const [archive, source, member] of [
      ['control.tar.xz', controlRoot, 'control'],
      ['data.tar.xz', dataRoot, '.'],
    ]) {
      const tar = spawnSync('tar', ['-cJf', join(root, archive), '-C', source, member], { encoding: 'utf8' })
      assert.equal(tar.status, 0, tar.stderr)
    }
    const deb = join(root, name)
    writeFileSync(deb, Buffer.concat([
      Buffer.from('!<arch>\n'),
      ...['debian-binary', 'control.tar.xz', 'data.tar.xz']
        .map((member) => arMember(member, readFileSync(join(root, member)))),
    ]))
    return deb
  }

  const license = readFileSync(licensePath, 'utf8')
  const metadata = [
    'Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/',
    'Upstream-Name: iris-drive-linux',
    'Source: htree://self/iris-drive',
    'Copyright: 2026 Iris Drive contributors',
    'License: MIT',
    '',
  ].join('\n')
  const accepted = spawnSync(verifier, [buildDeb('accepted.deb', metadata + license), licensePath], {
    encoding: 'utf8',
  })
  assert.equal(accepted.status, 0, accepted.stderr)

  for (const [name, copyright] of [
    ['unlicensed.deb', metadata.replace('License: MIT', 'License: UNLICENSED') + license],
    ['mismatch.deb', `${metadata}different terms\n`],
  ]) {
    const rejected = spawnSync(verifier, [buildDeb(name, copyright), licensePath], { encoding: 'utf8' })
    assert.notEqual(rejected.status, 0)
    assert.match(rejected.stderr, /Linux package license validation failed\./)
    assert.equal(rejected.stderr.includes(root), false)
  }

  const release = readFileSync(new URL('./local-release.mjs', import.meta.url), 'utf8')
  const block = release.match(/function buildLinuxArtifacts[\s\S]*?\nfunction androidSigningIsComplete/)?.[0] ?? ''
  const verification = block.indexOf('verify-linux-deb-license.mjs')
  const staging = block.indexOf('copyFileSync(debPath')
  assert.ok(verification >= 0 && verification < staging)
})

test('Unix CLI packaging produces normalized tar metadata', { skip: process.platform === 'win32' }, (t) => {
  const distDir = mkdtempSync(join(tmpdir(), 'iris-drive-release-tar-'))
  const extractDir = mkdtempSync(join(tmpdir(), 'iris-drive-release-tar-extract-'))
  t.after(() => {
    rmSync(distDir, { recursive: true, force: true })
    rmSync(extractDir, { recursive: true, force: true })
  })
  const run = (command, args) => {
    const result = spawnSync(command, args, { encoding: 'utf8' })
    assert.equal(result.status, 0, result.stderr)
  }
  packageUnixCliTarball({
    binaryPath: '/bin/echo',
    distDir,
    dryRun: false,
    env: { SOURCE_DATE_EPOCH: '123456789' },
    run,
    tag: 'v0.0.0',
    targetTriple: 'test-target',
  })
  const archivePath = join(distDir, 'idrive-v0.0.0-test-target.tar.gz')
  assert.equal(existsSync(archivePath), true)

  const listing = spawnSync('tar', ['-tzf', archivePath], { encoding: 'utf8' })
  assert.equal(listing.status, 0, listing.stderr)
  assert.deepEqual(listing.stdout.trim().split('\n'), [
    'idrive/LICENSE',
    'idrive/README.txt',
    'idrive/install.sh',
    'idrive/idrive',
  ])

  const archivedLicense = spawnSync('tar', ['-xOzf', archivePath, 'idrive/LICENSE'])
  assert.equal(archivedLicense.status, 0, archivedLicense.stderr?.toString())
  assert.deepEqual(archivedLicense.stdout, readFileSync(new URL('../LICENSE', import.meta.url)))

  // Inspect archived permissions independently of the caller's umask.
  const extraction = spawnSync('tar', ['-xpzf', archivePath, '-C', extractDir], { encoding: 'utf8' })
  assert.equal(extraction.status, 0, extraction.stderr)
  for (const [member, expectedMode] of [
    ['LICENSE', 0o644],
    ['README.txt', 0o644],
    ['install.sh', 0o755],
    ['idrive', 0o755],
  ]) {
    assert.equal(statSync(join(extractDir, 'idrive', member)).mode & 0o777, expectedMode)
  }

  const repoRoot = fileURLToPath(new URL('..', import.meta.url))
  assert.equal(gunzipSync(readFileSync(archivePath)).includes(Buffer.from(repoRoot)), false)
})
