import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { plannedReleaseAssetNames } from './local-release-lib.mjs'
import {
  providerConfigEnv, providerPublishEnv, providerResolveUrl, releaseHeadProvider, requireProviderHead,
} from './release-head-provider.mjs'

function providerDir(t, config) {
  const dir = mkdtempSync(join(tmpdir(), 'iris-drive-release-provider-'))
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  if (config !== undefined) writeFileSync(join(dir, 'config.toml'), config)
  return dir
}

const fipsConfig = '[server]\nbind_address="127.0.0.1:18088"\n[nostr]\nevent_transport="fips-local-only"\n'

test('the release-head provider must exist, hold the release key and use FIPS', t => {
  const env = dir => ({ [providerConfigEnv]: dir })
  assert.throws(() => releaseHeadProvider(env(providerDir(t))), /Missing release-head provider config/)
  const keyless = providerDir(t, fipsConfig)
  assert.throws(() => releaseHeadProvider(env(keyless)), /has no release key/)
  const relay = providerDir(t, '[server]\nbind_address="127.0.0.1:18088"\n[nostr]\nevent_transport="relay"\n')
  writeFileSync(join(relay, 'keys'), 'fixture')
  assert.throws(() => releaseHeadProvider(env(relay)), /must set nostr\.event_transport = "fips-local-only"/)
  const ready = providerDir(t, fipsConfig)
  writeFileSync(join(ready, 'keys'), 'fixture')
  assert.deepEqual(releaseHeadProvider(env(ready)), { configDir: ready, baseUrl: 'http://127.0.0.1:18088' })
  assert.equal(
    providerResolveUrl({ baseUrl: 'http://127.0.0.1:18088' }, 'npub1example', 'releases/iris-drive'),
    'http://127.0.0.1:18088/api/nostr/resolve/npub1example/releases%2Firis-drive',
  )
})

test('publishing through the provider keeps the store htree add wrote', () => {
  const provider = { configDir: '/provider/config' }
  const publish = env => providerPublishEnv(env, provider, '/home/release')
  assert.deepEqual(publish({ PATH: '/bin' }),
    { PATH: '/bin', HTREE_CONFIG_DIR: '/provider/config', HTREE_DATA_DIR: '/home/release/.hashtree/data' })
  assert.equal(publish({ HTREE_CONFIG_DIR: '/scratch' }).HTREE_DATA_DIR, '/scratch/data')
  assert.equal(publish({ HTREE_CONFIG_DIR: '/scratch', HTREE_DATA_DIR: '/data' }).HTREE_DATA_DIR, '/data')
})

test('a release is published only once the provider serves its root', () => {
  const root = `${'a'.repeat(64)}:${'b'.repeat(64)}`
  requireProviderHead({ cid: root }, root, 'url')
  assert.throws(() => requireProviderHead({ cid: `${'c'.repeat(64)}:${'b'.repeat(64)}` }, root, 'url'), /serves c+:b+, expected a+/)
  assert.throws(() => requireProviderHead({ cid: null }, root, 'url'), /serves no release head/)
  assert.throws(() => requireProviderHead({ cid: root }, undefined, 'url'), /expected a published root/)
})

test('final dry-run plans the head through the provider and its read-back', t => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-provider-plan-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const assetDir = join(root, 'dist')
  mkdirSync(assetDir)
  for (const name of plannedReleaseAssetNames('v9.9.9', ['macos', 'linux', 'windows', 'android'])) {
    writeFileSync(join(assetDir, name), name)
  }
  const result = spawnSync(process.execPath, [fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
    '--final', '--dry-run', '--skip-zapstore', '--tag', 'v9.9.9', '--asset-dir', assetDir,
    '--stage-dir', join(root, 'stage')], {
    encoding: 'utf8',
    env: { ...process.env, IRIS_DRIVE_RELEASE_NPUB: 'npub1example', [providerConfigEnv]: '/provider/config',
      IRIS_DRIVE_RELEASE_RESOLVER_REFRESH_BASE_URLS: 'off' },
  })
  assert.equal(result.status, 0, result.stderr)
  const publish = result.stdout.indexOf('$ htree release publish releases/iris-drive v9.9.9 dry-run --legacy-keys')
  assert.ok(publish > 0, result.stdout)
  assert.ok(result.stdout.indexOf('Would publish the release head through the provider /provider/config') < publish)
  assert.ok(result.stdout.indexOf('Would verify the provider serves the new releases/iris-drive root') > publish)
})
