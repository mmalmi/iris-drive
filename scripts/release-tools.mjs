#!/usr/bin/env node
// Pinned haps, htree and git-remote-htree for the final release publish.
// Archives are fixed by SHA-256 in release-tools.lock.json, re-verified before
// every use and extracted into a fresh directory for that run only.
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import {
  chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync,
} from 'node:fs'
import os from 'node:os'
import { delimiter, dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { gunzipSync } from 'node:zlib'

export const releaseToolsLock = JSON.parse(
  readFileSync(new URL('./release-tools.lock.json', import.meta.url), 'utf8'))
export const installCommand = 'node scripts/release-tools.mjs install'

const sha256 = data => createHash('sha256').update(data).digest('hex')

export function hostTarget(platform = process.platform, arch = process.arch) {
  const system = { darwin: 'apple-darwin', linux: 'unknown-linux-gnu' }[platform]
  const cpu = { arm64: 'aarch64', x64: 'x86_64' }[arch]
  return system && cpu ? `${cpu}-${system}` : `${arch}-${platform}`
}

export function defaultCacheDir(env = process.env, platform = process.platform) {
  if (env.IRIS_DRIVE_RELEASE_TOOLS_DIR) return resolve(env.IRIS_DRIVE_RELEASE_TOOLS_DIR)
  const base = platform === 'darwin' ? join(os.homedir(), 'Library', 'Caches')
    : env.XDG_CACHE_HOME || join(os.homedir(), '.cache')
  return join(base, 'iris-drive', 'release-tools')
}

function pinnedArchives(lock, target) {
  const haps = lock.haps.assets[target]
  const hashtree = lock.hashtree.assets[target]
  if (!haps || !hashtree) {
    throw new Error(`No pinned release tools for ${target}; add its archives to scripts/release-tools.lock.json`)
  }
  return [{ ...haps, url: lock.haps.url + haps.name }, { ...hashtree, url: lock.hashtree.url + hashtree.name }]
}

const archivePath = (cacheDir, asset) => join(cacheDir, 'archives', `${asset.sha256}-${asset.name}`)

function verifiedArchive(path, asset) {
  const data = readFileSync(path)
  if (data.length !== asset.bytes || sha256(data) !== asset.sha256) {
    throw new Error(`${asset.name} does not match its pinned SHA-256`)
  }
  return data
}

// Returns the named regular-file members of a .tar.gz; links and duplicates are refused.
export function tarMembers(gzipped, wanted) {
  const tar = gunzipSync(gzipped)
  const found = new Map()
  let offset = 0
  let nextName = null
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512)
    if (header.every(byte => byte === 0)) break
    const field = (start, length) => header.subarray(start, start + length).toString('utf8').replace(/\0.*$/s, '')
    const size = Number.parseInt(field(124, 12).trim() || '0', 8)
    const type = String.fromCharCode(header[156] || 48)
    const prefix = field(345, 155)
    const name = (nextName ?? (prefix ? `${prefix}/${field(0, 100)}` : field(0, 100))).replace(/^\.\//, '')
    nextName = null
    const body = tar.subarray(offset + 512, offset + 512 + size)
    offset += 512 + Math.ceil(size / 512) * 512
    if (type === 'x') nextName = body.toString('utf8').match(/^\d+ path=(.*)$/m)?.[1] ?? null
    if (type === 'L') nextName = body.toString('utf8').replace(/\0.*$/s, '')
    if (!wanted.includes(name) || ['x', 'L', 'g'].includes(type)) continue
    const base = name.split('/').pop()
    if (type !== '0' || found.has(name)) throw new Error(`${base} must be one regular file`)
    found.set(name, Buffer.from(body))
  }
  for (const name of wanted) {
    if (!found.has(name)) throw new Error(`Release tool archive is missing ${name}`)
  }
  return found
}

function checkVersions(dir, lock) {
  for (const [tool, path] of [['haps', join(dir, 'haps')], ['htree', join(dir, 'libexec', 'htree')]]) {
    const version = tool === 'haps' ? lock.haps.version : lock.hashtree.version
    const result = spawnSync(path, ['--version'], { encoding: 'utf8' })
    if (result.status !== 0 || result.stdout.trim() !== `${tool} ${version}`) {
      throw new Error(`Pinned ${tool} did not run here (expected ${tool} ${version})`)
    }
  }
  const helper = spawnSync(join(dir, 'libexec', 'git-remote-htree'), [], { encoding: 'utf8' })
  if (helper.status === 0 || !helper.stderr.includes('Usage: git-remote-htree')) {
    throw new Error('Pinned git-remote-htree did not run here')
  }
}

// Extracts verified tools into a new directory; the caller owns its removal.
export function prepareReleaseTools({ lock = releaseToolsLock, cacheDir = defaultCacheDir(), target = hostTarget() }) {
  const [hapsAsset, hashtreeAsset] = pinnedArchives(lock, target)
  if (![hapsAsset, hashtreeAsset].every(asset => existsSync(archivePath(cacheDir, asset)))) {
    throw new Error(`Missing pinned haps ${lock.haps.version} and htree ${lock.hashtree.version}; run: ${installCommand}`)
  }
  const haps = tarMembers(verifiedArchive(archivePath(cacheDir, hapsAsset), hapsAsset), ['haps'])
  const helpers = tarMembers(verifiedArchive(archivePath(cacheDir, hashtreeAsset), hashtreeAsset),
    ['hashtree/htree', 'hashtree/git-remote-htree'])
  if (sha256(helpers.get('hashtree/git-remote-htree')) !== hashtreeAsset.gitRemoteHtreeSha256) {
    throw new Error('git-remote-htree does not match its pinned SHA-256')
  }
  mkdirSync(join(cacheDir, 'runs'), { recursive: true })
  const dir = mkdtempSync(join(cacheDir, 'runs', `${target}-`))
  try {
    mkdirSync(join(dir, 'libexec'))
    for (const [path, data] of [[join(dir, 'haps'), haps.get('haps')],
      [join(dir, 'libexec', 'htree'), helpers.get('hashtree/htree')],
      [join(dir, 'libexec', 'git-remote-htree'), helpers.get('hashtree/git-remote-htree')]]) {
      writeFileSync(path, data, { flag: 'wx' })
      chmodSync(path, 0o755)
    }
    writeFileSync(join(dir, 'bundle.json'), `${JSON.stringify(
      { hashtree: `v${lock.hashtree.version}`, archive_sha256: hashtreeAsset.sha256 }, null, 2)}\n`)
    checkVersions(dir, lock)
    return dir
  } catch (error) {
    rmSync(dir, { recursive: true, force: true })
    throw error
  }
}

function curlDownload(url, dest) {
  const result = spawnSync('curl', ['--fail', '--location', '--proto', '=https', '--proto-redir', '=https',
    '--silent', '--show-error', '--max-time', '900', '--output', dest, url], { stdio: ['ignore', 'inherit', 'inherit'] })
  if (result.status !== 0) throw new Error(`Could not download ${url}`)
}

export function installReleaseTools({
  lock = releaseToolsLock, cacheDir = defaultCacheDir(), target = hostTarget(),
  download = curlDownload, log = console.log,
} = {}) {
  for (const asset of pinnedArchives(lock, target)) {
    const dest = archivePath(cacheDir, asset)
    if (!existsSync(dest)) {
      mkdirSync(dirname(dest), { recursive: true })
      const partial = `${dest}.partial-${process.pid}`
      try {
        download(asset.url, partial)
        verifiedArchive(partial, asset)
        renameSync(partial, dest)
      } finally {
        rmSync(partial, { force: true })
      }
    }
    verifiedArchive(dest, asset)
    log(`verified ${asset.name}`)
  }
  rmSync(prepareReleaseTools({ lock, cacheDir, target }), { recursive: true, force: true })
  log(`haps ${lock.haps.version} and htree ${lock.hashtree.version} are ready in ${cacheDir}`)
}

export function withHapsKey(env, home = os.homedir()) {
  const keyPath = String(env.NOSTR_KEY_PATH ?? '').trim().replace(/^(['"])(.*)\1$/, '$2')
  if (env.HAPS_KEY_FILE || !keyPath) return env
  return { ...env, HAPS_KEY_FILE: keyPath.replace(/^~(?=\/|$)/, home) }
}

// Environment for haps/htree release commands; removes the tools when the process exits.
export function releaseToolsEnv({
  env, dryRun = false, lock = releaseToolsLock, cacheDir = defaultCacheDir(env), target = hostTarget(),
  log = console.log, home = os.homedir(),
}) {
  if (dryRun) {
    log(`Would use pinned haps ${lock.haps.version} and htree ${lock.hashtree.version} from ${cacheDir} (${installCommand})`)
    return withHapsKey(env, home)
  }
  const dir = prepareReleaseTools({ lock, cacheDir, target })
  process.once('exit', () => rmSync(dir, { recursive: true, force: true }))
  log(`Release tools: haps ${lock.haps.version}, htree ${lock.hashtree.version} (pinned, verified)`)
  return withHapsKey({ ...env, PATH: [dir, join(dir, 'libexec'), env.PATH].filter(Boolean).join(delimiter) }, home)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [command, separator, ...rest] = process.argv.slice(2)
  try {
    if (command === 'install') {
      installReleaseTools({ cacheDir: defaultCacheDir() })
    } else if (command === 'run' && separator === '--' && rest.length > 0) {
      const { parseEnvFile } = await import('./local-release-lib.mjs')
      const zapstoreEnv = join(dirname(fileURLToPath(import.meta.url)), '..', '.env.zapstore.local')
      const fileEnv = existsSync(zapstoreEnv) ? parseEnvFile(readFileSync(zapstoreEnv, 'utf8')) : {}
      const keyEnv = fileEnv.NOSTR_KEY_PATH ? { NOSTR_KEY_PATH: fileEnv.NOSTR_KEY_PATH } : {}
      const env = releaseToolsEnv({ env: { ...keyEnv, ...process.env } })
      const result = spawnSync(rest[0], rest.slice(1), { env, stdio: 'inherit' })
      process.exitCode = result.status ?? 1
    } else {
      console.error(`Usage: ${installCommand}\n       node scripts/release-tools.mjs run -- <command> [args...]`)
      process.exitCode = 2
    }
  } catch (error) {
    console.error(`error: ${error.message}`)
    process.exitCode = 1
  }
}
