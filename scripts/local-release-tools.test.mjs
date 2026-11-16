import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { delimiter, join } from 'node:path'
import { tarGz } from './lib/tar-fixture.mjs'
import {
  installCommand,
  installReleaseTools,
  prepareReleaseTools,
  releaseToolsEnv,
  releaseToolsLock,
  withHapsKey,
} from './release-tools.mjs'

const sha256 = data => createHash('sha256').update(data).digest('hex')
const target = 'aarch64-apple-darwin'
const script = text => `#!/bin/sh\n${text}\n`
const helper = script('echo "Error: Usage: git-remote-htree <remote-name> <url>" >&2; exit 1')

function fixture({ hapsVersion = '0.1.27', helperBytes = helper, hapsType = '0' } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-tools-'))
  const hapsArchive = tarGz([
    { name: 'bundle.json', data: '{}', mode: 0o644 },
    { name: 'haps', data: script(`echo "haps ${hapsVersion}"`), type: hapsType,
      linkname: hapsType === '2' ? '/bin/sh' : '' },
    { name: 'libexec/htree', data: script('echo "htree 0.2.150"') },
  ])
  const hashtreeArchive = tarGz([
    { name: 'hashtree/', type: '5' },
    { name: 'hashtree/htree', data: script('echo "htree 0.4.5"') },
    { name: 'hashtree/git-remote-htree', data: helperBytes },
  ])
  const source = join(root, 'source')
  mkdirSync(source)
  const asset = (name, data, extra = {}) => {
    writeFileSync(join(source, name), data)
    return { name, bytes: data.length, sha256: sha256(data), ...extra }
  }
  const lock = {
    schema: 1,
    haps: { version: '0.1.27', commit: 'a'.repeat(40), url: 'https://example.invalid/haps/v0.1.27/',
      assets: { [target]: asset('haps.tar.gz', hapsArchive) } },
    hashtree: { version: '0.4.5', commit: 'b'.repeat(40), url: 'https://example.invalid/hashtree/v0.4.5/',
      assets: { [target]: asset('hashtree.tar.gz', hashtreeArchive, { gitRemoteHtreeSha256: sha256(helper) }) } },
  }
  const download = (url, dest) => writeFileSync(dest, readFileSync(join(source, url.split('/').pop())))
  return { root, source, lock, cacheDir: join(root, 'cache'), download }
}

test('committed lock pins exact releases, commits and archive hashes for this Mac', () => {
  const { haps, hashtree } = releaseToolsLock
  for (const tool of [haps, hashtree]) {
    assert.match(tool.version, /^\d+\.\d+\.\d+$/)
    assert.match(tool.commit, /^[0-9a-f]{40}$/)
    assert.ok(tool.url.startsWith('https://github.com/mmalmi/'), tool.url)
    assert.ok(tool.url.endsWith(`/releases/download/v${tool.version}/`), tool.url)
    assert.ok(!/latest/i.test(tool.url))
    assert.ok(tool.assets[target], 'release Mac target is pinned')
    for (const asset of Object.values(tool.assets)) {
      assert.match(asset.sha256, /^[0-9a-f]{64}$/)
      assert.ok(Number.isInteger(asset.bytes) && asset.bytes > 0)
      assert.ok(asset.name.includes(tool === haps ? `v${tool.version}` : 'hashtree-'), asset.name)
    }
  }
  for (const asset of Object.values(hashtree.assets)) assert.match(asset.gitRemoteHtreeSha256, /^[0-9a-f]{64}$/)
  assert.deepEqual(Object.keys(haps.assets).sort(), Object.keys(hashtree.assets).sort())
})

test('install downloads, verifies and then runs the pinned tools from the cache', () => {
  const { lock, cacheDir, download } = fixture()
  const logs = []
  installReleaseTools({ lock, cacheDir, target, download, log: line => logs.push(line) })
  assert.equal(readdirSync(join(cacheDir, 'archives')).length, 2)
  assert.match(logs.join('\n'), /haps 0\.1\.27 and htree 0\.4\.5 are ready/)
  const dir = prepareReleaseTools({ lock, cacheDir, target })
  assert.ok(existsSync(join(dir, 'haps')) && existsSync(join(dir, 'libexec', 'htree')))
  assert.equal(readFileSync(join(dir, 'libexec', 'git-remote-htree'), 'utf8'), helper)
  assert.deepEqual(JSON.parse(readFileSync(join(dir, 'bundle.json'), 'utf8')),
    { hashtree: 'v0.4.5', archive_sha256: lock.hashtree.assets[target].sha256 })
})

test('a download that differs from the pin is rejected and never cached', () => {
  const { lock, cacheDir, source, download } = fixture()
  writeFileSync(join(source, 'hashtree.tar.gz'), Buffer.alloc(lock.hashtree.assets[target].bytes))
  assert.throws(() => installReleaseTools({ lock, cacheDir, target, download, log() {} }),
    /hashtree\.tar\.gz does not match its pinned SHA-256/)
  assert.deepEqual(readdirSync(join(cacheDir, 'archives')).filter(name => name.includes('hashtree')), [])
})

test('missing archives fail with the exact install command', () => {
  const { lock, cacheDir } = fixture()
  assert.throws(() => prepareReleaseTools({ lock, cacheDir, target }),
    error => error.message.includes(installCommand) && /haps 0\.1\.27/.test(error.message))
})

test('cached archives are re-verified before every use', () => {
  const { lock, cacheDir, download } = fixture()
  installReleaseTools({ lock, cacheDir, target, download, log() {} })
  const cached = join(cacheDir, 'archives', readdirSync(join(cacheDir, 'archives')).find(name => name.includes('haps')))
  const bytes = readFileSync(cached)
  bytes[bytes.length - 1] ^= 1
  writeFileSync(cached, bytes)
  assert.throws(() => prepareReleaseTools({ lock, cacheDir, target }), /haps\.tar\.gz does not match/)
})

test('a helper that differs from its own pin, a linked member or a wrong version is refused', () => {
  for (const [options, pattern] of [
    [{ helperBytes: script('exit 1') }, /git-remote-htree does not match its pinned SHA-256/],
    [{ hapsType: '2' }, /haps must be one regular file/],
    [{ hapsVersion: '0.1.19' }, /expected haps 0\.1\.27/],
  ]) {
    const { lock, cacheDir, download } = fixture(options)
    assert.throws(() => installReleaseTools({ lock, cacheDir, target, download, log() {} }), pattern)
  }
})

test('unpinned hosts are named instead of guessing a download', () => {
  const { lock, cacheDir } = fixture()
  assert.throws(() => prepareReleaseTools({ lock, cacheDir, target: 'riscv64gc-unknown-linux-gnu' }),
    /No pinned release tools for riscv64gc-unknown-linux-gnu/)
})

test('release env puts the verified tools first and derives the Haps key from the Zapstore key', () => {
  const { lock, cacheDir, download } = fixture()
  installReleaseTools({ lock, cacheDir, target, download, log() {} })
  const env = releaseToolsEnv({ env: { PATH: '/usr/bin', NOSTR_KEY_PATH: '~/keys/release.nsec' },
    lock, cacheDir, target, log() {}, home: '/home/release' })
  const [first, second, rest] = env.PATH.split(delimiter)
  assert.ok(existsSync(join(first, 'haps')))
  assert.equal(second, join(first, 'libexec'))
  assert.equal(rest, '/usr/bin')
  assert.equal(env.HAPS_KEY_FILE, '/home/release/keys/release.nsec')
  assert.equal(withHapsKey({ HAPS_KEY_FILE: '/explicit', NOSTR_KEY_PATH: '/other' }).HAPS_KEY_FILE, '/explicit')
})

test('dry runs describe the pinned tools without requiring or extracting them', () => {
  const { lock, cacheDir } = fixture()
  const logs = []
  const env = releaseToolsEnv({ env: { PATH: '/usr/bin' }, dryRun: true, lock, cacheDir, target,
    log: line => logs.push(line) })
  assert.equal(env.PATH, '/usr/bin')
  assert.match(logs.join('\n'), /Would use pinned haps 0\.1\.27 and htree 0\.4\.5/)
  assert.ok(!existsSync(join(cacheDir, 'runs')))
})

test('publishing without the pinned tools stops before staging and names the install command', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-tools-publish-'))
  const result = spawnSync(process.execPath, [fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
    '--tag', 'v9.9.9', '--publish', '--draft', '--asset-dir', join(root, 'dist'), '--stage-dir', join(root, 'stage')],
  { encoding: 'utf8', env: { ...process.env, IRIS_DRIVE_RELEASE_TOOLS_DIR: join(root, 'tools') } })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /Missing pinned haps 0\.1\.27 and htree 0\.4\.5; run: node scripts\/release-tools\.mjs install/)
  assert.ok(!existsSync(join(root, 'stage')))
})
