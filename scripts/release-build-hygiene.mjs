import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  utimesSync,
  writeFileSync,
} from 'node:fs'
import os from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { gunzipSync } from 'node:zlib'

const RUST_PATH_DESTINATIONS = {
  home: '/usr/src/home',
  workspace: '/usr/src/iris-drive',
}
const PROJECT_LICENSE_PATH = fileURLToPath(new URL('../LICENSE', import.meta.url))

function remapFlag(source, destination) {
  return `--remap-path-prefix=${source}=${destination}`
}

function nativeRemapFlags(source, destination) {
  return [
    `-ffile-prefix-map=${source}=${destination}`,
    `-fdebug-prefix-map=${source}=${destination}`,
  ]
}

function shellQuote(value) {
  return `'${value.replaceAll("'", `'"'"'`)}'`
}

function appendFlags(existing, flags) {
  return [String(existing ?? '').trim(), ...flags.map(shellQuote)]
    .filter(Boolean)
    .join(' ')
}

export function releaseRustBuildEnvironment(
  env,
  { repoRoot, homeDir = os.homedir(), platform = process.platform } = {},
) {
  const flags = [
    remapFlag(repoRoot, RUST_PATH_DESTINATIONS.workspace),
    remapFlag(homeDir, RUST_PATH_DESTINATIONS.home),
  ]
  const result = { ...env }
  const encoded = String(env.CARGO_ENCODED_RUSTFLAGS ?? '').trim()
  if (encoded) {
    result.CARGO_ENCODED_RUSTFLAGS = [encoded, ...flags].join('\u001f')
  } else {
    const rustFlags = String(env.RUSTFLAGS ?? '').trim()
    if (rustFlags) {
      result.RUSTFLAGS = [rustFlags, ...flags].join(' ')
    } else {
      result.CARGO_ENCODED_RUSTFLAGS = flags.join('\u001f')
    }
  }
  if (platform !== 'win32') {
    const nativeFlags = [
      ...nativeRemapFlags(repoRoot, RUST_PATH_DESTINATIONS.workspace),
      ...nativeRemapFlags(homeDir, RUST_PATH_DESTINATIONS.home),
    ]
    result.CFLAGS = appendFlags(env.CFLAGS, nativeFlags)
    result.CXXFLAGS = appendFlags(env.CXXFLAGS, nativeFlags)
    result.CC_SHELL_ESCAPED_FLAGS = '1'
  }
  return result
}

function privateBuildMetadataNeedles(env, { repoRoot, homeDir, hostname }) {
  return [repoRoot, homeDir, env.CARGO_HOME, env.RUSTUP_HOME, hostname]
    .map((value) => String(value ?? '').trim())
    .filter((value, index, values) => value.length >= 6 && values.indexOf(value) === index)
}

function asciiLowercase(bytes) {
  for (let index = 0; index < bytes.length; index += 1) {
    if (bytes[index] >= 0x41 && bytes[index] <= 0x5a) {
      bytes[index] += 0x20
    }
  }
  return bytes
}

function metadataNeedleVariants(needle, platform) {
  const variants = new Set([needle])
  if (platform === 'win32') {
    variants.add(needle.replaceAll('\\', '/'))
    variants.add(needle.replaceAll('/', '\\'))
  }
  return variants
}

function containsPrivateBuildMetadata(bytes, needles, platform) {
  const haystack = platform === 'win32' ? asciiLowercase(bytes) : bytes
  for (const needle of needles) {
    for (let variant of metadataNeedleVariants(needle, platform)) {
      if (platform === 'win32') {
        variant = variant.toLowerCase()
      }
      const encodings = platform === 'win32' ? ['utf8', 'utf16le'] : ['utf8']
      if (encodings.some((encoding) => haystack.includes(Buffer.from(variant, encoding)))) {
        return true
      }
    }
  }
  return false
}

export function assertNoPrivateBuildMetadata(
  paths,
  env,
  {
    repoRoot,
    homeDir = os.homedir(),
    hostname = os.hostname(),
    platform = process.platform,
  } = {},
) {
  const needles = privateBuildMetadataNeedles(env, { repoRoot, homeDir, hostname })
  for (const path of paths) {
    if (!existsSync(path)) {
      throw new Error(`Missing release binary for privacy audit: ${basename(path)}`)
    }
    const bytes = readFileSync(path)
    if (containsPrivateBuildMetadata(bytes, needles, platform)) {
      throw new Error(
        `Release binary ${basename(path)} contains private build metadata; rebuild with remapped Rust paths.`,
      )
    }
  }
}

export function normalizedTarOwnerArgs(platform = process.platform) {
  return platform === 'darwin'
    ? ['--uid', '0', '--gid', '0', '--uname', 'root', '--gname', 'root']
    : ['--owner=0', '--group=0', '--numeric-owner']
}

function tarOctal(bytes) {
  const value = bytes.toString('ascii').replace(/\0.*$/, '').trim()
  return value ? Number.parseInt(value, 8) : 0
}

function tarText(bytes) {
  return bytes.toString('utf8').replace(/\0.*$/, '')
}

function assertNormalizedTarMetadata(path, expectedMtime) {
  const archive = gunzipSync(readFileSync(path))
  for (let offset = 0; offset + 512 <= archive.length;) {
    const header = archive.subarray(offset, offset + 512)
    if (header.every((byte) => byte === 0)) {
      return
    }
    const size = tarOctal(header.subarray(124, 136))
    const uid = tarOctal(header.subarray(108, 116))
    const gid = tarOctal(header.subarray(116, 124))
    const mtime = tarOctal(header.subarray(136, 148))
    const uname = tarText(header.subarray(265, 297))
    const gname = tarText(header.subarray(297, 329))
    if (
      uid !== 0 ||
      gid !== 0 ||
      !['', 'root'].includes(uname) ||
      !['', 'root'].includes(gname) ||
      mtime !== expectedMtime
    ) {
      throw new Error('Release tar contains non-reproducible ownership or timestamps.')
    }
    offset += 512 + Math.ceil(size / 512) * 512
  }
  throw new Error('Release tar is missing its end marker.')
}

function writeUnixInstallScript(path) {
  writeFileSync(
    path,
    `#!/bin/bash
set -e

INSTALL_DIR="\${1:-/usr/local/bin}"
install -d "\${INSTALL_DIR}"
install -m 755 idrive "\${INSTALL_DIR}/"
`,
  )
  chmodSync(path, 0o755)
}

function writeUnixReadme(path) {
  writeFileSync(
    path,
    `idrive - Iris Drive CLI
==========================

Binary included:
  idrive  - CLI and daemon helper

Quick install:
  ./install.sh
  ./install.sh ~/.local/bin
`,
  )
  chmodSync(path, 0o644)
}

export function packageUnixCliTarball({
  binaryPath,
  distDir,
  dryRun,
  env,
  run,
  tag,
  targetTriple,
}) {
  const bundleDir = join(distDir, 'idrive')
  const tarPath = join(distDir, `idrive-${targetTriple}.tar`)
  const unversioned = `${tarPath}.gz`
  const versioned = join(distDir, `idrive-${tag}-${targetTriple}.tar.gz`)
  const mtime = Number.parseInt(String(env.SOURCE_DATE_EPOCH ?? '0'), 10) || 0
  const members = ['idrive/LICENSE', 'idrive/README.txt', 'idrive/install.sh', 'idrive/idrive']
  if (!dryRun) {
    if (!existsSync(binaryPath)) {
      throw new Error(`Missing idrive binary for ${targetTriple}: ${binaryPath}`)
    }
    if (!existsSync(PROJECT_LICENSE_PATH)) {
      throw new Error('Missing project LICENSE for CLI archive.')
    }
    rmSync(bundleDir, { recursive: true, force: true })
    mkdirSync(bundleDir, { recursive: true })
    copyFileSync(PROJECT_LICENSE_PATH, join(bundleDir, 'LICENSE'))
    chmodSync(join(bundleDir, 'LICENSE'), 0o644)
    copyFileSync(binaryPath, join(bundleDir, 'idrive'))
    chmodSync(join(bundleDir, 'idrive'), 0o755)
    writeUnixInstallScript(join(bundleDir, 'install.sh'))
    writeUnixReadme(join(bundleDir, 'README.txt'))
    for (const member of members) {
      utimesSync(join(distDir, member), mtime, mtime)
    }
  }
  run('tar', [...normalizedTarOwnerArgs(), '-cf', tarPath, '-C', distDir, ...members], { dryRun })
  run('gzip', ['-n', '-f', tarPath], { dryRun })
  if (!dryRun) {
    assertNormalizedTarMetadata(unversioned, mtime)
    copyFileSync(unversioned, versioned)
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
  assertNoPrivateBuildMetadata(process.argv.slice(2), process.env, { repoRoot })
  console.log('RELEASE_BINARY_PRIVACY_OK')
}
