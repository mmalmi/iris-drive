import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { generateKeyPairSync } from 'node:crypto'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { ascAuth } from './asc-auth.mjs'
import { fixtureCheckout } from './lib/release-fixture.mjs'
import { applyReleaseEnvDefaults } from './release-env.mjs'

const repo = fileURLToPath(new URL('..', import.meta.url))
const unnamed = Object.fromEntries([
  'IRIS_DRIVE_ASC_AUTH_KEY_PATH', 'IRIS_DRIVE_ASC_KEY_PATH', 'IRIS_DRIVE_ASC_AUTH_KEY_ID',
  'IRIS_DRIVE_ASC_KEY_ID', 'IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID', 'IRIS_DRIVE_ASC_ISSUER_ID',
].map(name => [name, '']))

// A keys folder like ~/.appstoreconnect that holds another team's key.
function otherTeamKeys(t) {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-asc-root-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  mkdirSync(join(root, 'private_keys'))
  writeFileSync(join(root, 'private_keys', 'AuthKey_OTHERTEAM.p8'),
    generateKeyPairSync('ec', { namedCurve: 'P-256' }).privateKey.export({ type: 'pkcs8', format: 'pem' }))
  writeFileSync(join(root, 'issuer.txt'), '11111111-1111-1111-1111-111111111111\n')
  return root
}

async function apple(t) {
  const calls = []
  const server = createServer((request, response) => {
    calls.push(request.url)
    response.writeHead(403, { 'Content-Type': 'application/json' })
    response.end('{"errors":[{"title":"Wrong team"}]}')
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  t.after(() => { server.closeAllConnections(); server.close() })
  return { calls, url: `http://127.0.0.1:${server.address().port}/v1/` }
}

function run(args, env) {
  return new Promise(resolve => {
    const child = spawn(process.execPath, args, { cwd: repo, env: { ...process.env, ...env } })
    let stderr = ''
    child.stderr.on('data', chunk => { stderr += chunk })
    child.on('close', status => resolve({ status, stderr }))
  })
}

test('App Store Connect auth uses only the key named in settings', t => {
  const root = otherTeamKeys(t)
  assert.deepEqual(ascAuth({ HOME: root, IRIS_DRIVE_ASC_ROOT: root }).problems, [
    'IRIS_DRIVE_ASC_AUTH_KEY_PATH is required',
    'IRIS_DRIVE_ASC_AUTH_KEY_ID is required',
    'IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID is required',
  ])
  const named = { IRIS_DRIVE_ASC_AUTH_KEY_PATH: '/keys/AuthKey_TEAMKEY.p8', IRIS_DRIVE_ASC_ISSUER_ID: 'issuer' }
  assert.deepEqual(ascAuth(named),
    { keyPath: '/keys/AuthKey_TEAMKEY.p8', keyId: 'TEAMKEY', issuerId: 'issuer', problems: [] })
  assert.deepEqual(ascAuth({ ...named, IRIS_DRIVE_ASC_KEY_ID: 'OTHER' }).problems,
    ['App Store Connect key ID OTHER does not match AuthKey_TEAMKEY.p8'])
})

for (const args of [
  ['scripts/testflight-app-store-connect.mjs', 'internal', 'status'],
  ['scripts/ios-profiles', 'list'],
]) test(`${args[0]} never picks a key from the keys folder`, async t => {
  const root = otherTeamKeys(t)
  const server = await apple(t)
  const result = await run(args, { ...unnamed, HOME: root, IRIS_DRIVE_ASC_ROOT: root, IRIS_DRIVE_ASC_BASE_URL: server.url })
  assert.equal(result.status, 2, result.stderr)
  assert.match(result.stderr, /IRIS_DRIVE_ASC_AUTH_KEY_PATH is required/)
  assert.deepEqual(server.calls, [])
})

test('App Store Connect failures name the key that Apple refused', async t => {
  const root = otherTeamKeys(t)
  const server = await apple(t)
  const result = await run(['scripts/testflight-app-store-connect.mjs', 'internal', 'status'], {
    ...unnamed, HOME: root, IRIS_DRIVE_ASC_BASE_URL: server.url,
    IRIS_DRIVE_ASC_AUTH_KEY_PATH: join(root, 'private_keys', 'AuthKey_OTHERTEAM.p8'),
    IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID: '11111111-1111-1111-1111-111111111111',
  })
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /key OTHERTEAM.*HTTP 403/)
})

test('release scripts in a linked worktree read the main checkout settings', t => {
  const { root, git } = fixtureCheckout()
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const linked = join(root, 'linked')
  git('worktree', 'add', '-q', '--detach', linked, 'HEAD')
  writeFileSync(join(root, '.env.release.local'), [
    'IRIS_DRIVE_ASC_AUTH_KEY_ID=MAIN', 'IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID=main-issuer',
    'IRIS_DRIVE_ASC_KEY_PATH=/main/AuthKey_MAIN.p8', '',
  ].join('\n'))
  writeFileSync(join(linked, '.env.release.local'), 'IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID=linked-issuer\n')
  const env = { IRIS_DRIVE_ASC_KEY_PATH: '' }
  applyReleaseEnvDefaults({ repoRoot: linked, env })
  assert.deepEqual(env, {
    IRIS_DRIVE_ASC_KEY_PATH: '',
    IRIS_DRIVE_ASC_AUTH_KEY_ID: 'MAIN',
    IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID: 'linked-issuer',
  })
})

test('iOS build names its App Store Connect key and reads main checkout settings', () => {
  const script = readFileSync(join(repo, 'scripts/ios-build'), 'utf8')
  assert.doesNotMatch(script, /private_keys|issuer\.txt|IRIS_DRIVE_ASC_ROOT/)
  assert.match(script, /--git-common-dir/)
})
