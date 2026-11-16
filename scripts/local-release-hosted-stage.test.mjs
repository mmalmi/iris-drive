import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, writeFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { fixtureCheckout, writeProduct, writeZip } from './lib/release-fixture.mjs'
import { downloadHostedProduct, hostedWorkDir, stageHostedRelease } from './release-hosted-stage.mjs'
import { gitHubRepository } from './release-env.mjs'

const quiet = () => {}
const linuxEntries = [
  { name: 'idrive-v9.9.9-x86_64-unknown-linux-gnu.tar.gz', data: 'cli' },
  { name: 'iris-drive-v9.9.9-linux-x64.deb', data: 'deb' },
]

function stage(checkout, platform, entries, options = {}) {
  const product = writeProduct(join(checkout.root, 'artifacts', 'hosted-input', `${platform}-${Math.random()}`),
    { platform, commit: checkout.commit, tree: checkout.tree, entries, ...options })
  return stageHostedRelease({ repoRoot: checkout.root, commit: checkout.commit, platform, productDirectory: product,
    work: hostedWorkDir(checkout.root, '9.9.9', checkout.commit), log: quiet, ...options.stage })
}

test('Linux products are verified, linked into dist and recorded, and a rerun is idempotent', async () => {
  const checkout = fixtureCheckout()
  await stage(checkout, 'linux', linuxEntries)
  assert.equal(readFileSync(join(checkout.root, 'dist', 'iris-drive-v9.9.9-linux-x64.deb'), 'utf8'), 'deb')
  const record = JSON.parse(readFileSync(join(hostedWorkDir(checkout.root, '9.9.9', checkout.commit), 'linux-staged.json')))
  assert.equal(record.commit, checkout.commit)
  assert.deepEqual(Object.keys(record.outputs).sort(), linuxEntries.map(entry => entry.name).sort())
  await stage(checkout, 'linux', linuxEntries)
  writeFileSync(join(checkout.root, 'dist', 'iris-drive-v9.9.9-linux-x64.deb'), 'other')
  await assert.rejects(stage(checkout, 'linux', linuxEntries), /differs from the verified hosted product/)
})

test('a product for another commit, version or source state is refused before anything is staged', async () => {
  const checkout = fixtureCheckout()
  await assert.rejects(stage(checkout, 'linux', linuxEntries, { receipt: { commit: 'c'.repeat(40) } }),
    /does not match the requested clean source/)
  await assert.rejects(stage(checkout, 'linux', linuxEntries, { version: '9.9.8' }), /this checkout releases 9\.9\.9/)
  await assert.rejects(stage(checkout, 'windows', linuxEntries, { receipt: { sourceCleanBeforeBuild: false } }),
    /does not match the requested clean source/)
  writeFileSync(join(checkout.root, 'untracked.txt'), 'local change')
  await assert.rejects(stage(checkout, 'linux', linuxEntries), /clean source checkout/)
  assert.ok(!existsSync(join(checkout.root, 'dist')))
})

test('Linux products must hold exactly the planned release assets', async () => {
  const checkout = fixtureCheckout()
  await assert.rejects(stage(checkout, 'linux', [...linuxEntries, { name: 'extra.txt', data: 'x' }]),
    /does not hold exactly/)
})

function windowsEntries(root, { extraPublish = false } = {}) {
  const publish = { 'Iris Drive.exe': 'gui', 'idrive.exe': 'cli-exe' }
  const app = writeZip(join(root, 'zips', 'app.zip'), publish)
  const cli = writeZip(join(root, 'zips', 'cli.zip'), { 'idrive.exe': 'cli-exe', LICENSE: 'license' })
  return [
    { name: './', type: '5' },
    { name: './idrive-v9.9.9-x86_64-pc-windows-msvc.zip', data: cli },
    { name: './iris-drive-v9.9.9-windows-x64-setup.exe', data: 'setup' },
    { name: './iris-drive-v9.9.9-windows-x64.zip', data: app },
    ...Object.entries(publish).map(([name, data]) => ({ name: `./publish/${name}`, data })),
    ...(extraPublish ? [{ name: './publish/unshipped.dll', data: 'x' }] : []),
  ]
}

test('Windows release files are selected and the app zip must equal the hosted publish directory', async () => {
  const checkout = fixtureCheckout()
  const scratch = mkdtempSync(join(tmpdir(), 'iris-drive-windows-zips-'))
  await stage(checkout, 'windows', windowsEntries(scratch))
  assert.ok(existsSync(join(checkout.root, 'dist', 'iris-drive-v9.9.9-windows-x64-setup.exe')))
  const other = fixtureCheckout()
  const bad = writeProduct(join(scratch, 'bad'), { platform: 'windows', commit: other.commit, tree: other.tree,
    entries: windowsEntries(scratch, { extraPublish: true }) })
  await assert.rejects(stageHostedRelease({ repoRoot: other.root, commit: other.commit, platform: 'windows',
    productDirectory: bad, work: join(scratch, 'work'), log: quiet }), /python3 exited 1/)
  assert.ok(!existsSync(join(other.root, 'dist')))
  const selected = spawnSync('python3', ['-I', fileURLToPath(new URL('./select-hosted-release-products.py', import.meta.url)),
    'windows', join(bad, 'windows-products.tar.gz'), mkdtempSync(join(scratch, 'select-')),
    'idrive-v9.9.9-x86_64-pc-windows-msvc.zip', 'iris-drive-v9.9.9-windows-x64-setup.exe',
    'iris-drive-v9.9.9-windows-x64.zip'], { encoding: 'utf8' })
  assert.match(selected.stderr, /Windows app archive differs from the hosted publish directory/)
})

test('Android products are selected and handed to signing; only signed outputs reach dist', async () => {
  const checkout = fixtureCheckout()
  const calls = []
  const sign = ({ stage: input, output, files, version }) => {
    calls.push({ version, apk: readFileSync(join(input, 'apk/release/app-release-unsigned.apk'), 'utf8') })
    for (const name of files) writeFileSync(join(output, name), `signed ${name}`)
  }
  await stage(checkout, 'android', [
    { name: 'apk/release/app-release-unsigned.apk', data: 'apk' },
    { name: 'bundle/release/app-release.aab', data: 'aab' },
    { name: 'apk/uiTest/app-uiTest.apk', data: 'test apk' },
  ], { stage: { sign } })
  assert.deepEqual(calls, [{ version: '9.9.9', apk: 'apk' }])
  assert.equal(readFileSync(join(checkout.root, 'dist', 'iris-drive-v9.9.9-android-arm64.apk'), 'utf8'),
    'signed iris-drive-v9.9.9-android-arm64.apk')
})

test('macOS and iOS products are only verified, for the Mac signing and iOS export steps', async () => {
  const checkout = fixtureCheckout()
  const result = await stage(checkout, 'ios-release', [{ name: 'IrisDriveIOS.xcarchive/Info.plist', data: 'x' }])
  assert.ok(result.productDirectory)
  assert.ok(!existsSync(join(checkout.root, 'dist')))
})

test('downloads come only from a completed hosted products run', () => {
  const calls = []
  const run = (command, args) => {
    calls.push([command, ...args])
    return args[1] === 'view' ? JSON.stringify({ workflowName: 'Unsigned native products', status: 'completed' }) : ''
  }
  const commit = 'a'.repeat(40)
  downloadHostedProduct({ commit, platform: 'linux', runId: '42', repository: 'owner/drive', destination: '/tmp/x', run })
  assert.deepEqual(calls[1], ['gh', 'run', 'download', '42', '-R', 'owner/drive', '-n', `drive-linux-${commit}`, '-D', '/tmp/x'])
  const other = () => JSON.stringify({ workflowName: 'Native CI', status: 'completed' })
  assert.throws(() => downloadHostedProduct({ commit, platform: 'linux', runId: '42', repository: 'owner/drive',
    destination: '/tmp/x', run: other }), /not a completed "Unsigned native products" run/)
  assert.throws(() => downloadHostedProduct({ commit, platform: 'linux', runId: 'latest', repository: 'owner/drive',
    destination: '/tmp/x', run }), /numeric id/)
})

test('the hosted repository comes from the github remote, not from committed configuration', () => {
  const checkout = fixtureCheckout()
  assert.equal(gitHubRepository(checkout.root), '')
  checkout.git('remote', 'add', 'github', 'git@github.com:owner/drive.git')
  assert.equal(gitHubRepository(checkout.root), 'owner/drive')
})
