#!/usr/bin/env node
import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { closeSync, constants, accessSync, mkdirSync, openSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { stageHostedProduct } from './stage-hosted-product.mjs'

const root = resolve(process.env.SOURCE_ROOT)
const inputs = resolve(process.env.PRODUCT_INPUTS)
const output = resolve(process.env.CHECK_OUTPUT)
const scripts = dirname(fileURLToPath(import.meta.url))
const webCommit = process.env.WEB_COMMIT
if (!/^[0-9a-f]{40}$/.test(webCommit)) throw new Error('Exact web source commit required')
mkdirSync(output, { recursive: true })
let env = { ...process.env }
function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit', ...options })
  if (result.status !== 0) throw new Error(`${command} failed: ${result.status ?? result.error}`)
}
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
if (commit !== env.SOURCE_COMMIT) throw new Error('Native source candidate mismatch')
const product = stageHostedProduct({ repoRoot: root, productDirectory: inputs, platform: 'linux', run, env, destinationParent: output })
const archives = readdirSync(product).filter(name => name.endsWith('-x86_64-unknown-linux-gnu.tar.gz'))
if (archives.length !== 1) throw new Error('Expected exactly one packaged CLI')
const cliPayload = join(output, 'cli-payload')
mkdirSync(cliPayload)
run('python3', [join(scripts, 'unpack-hosted-product.py'), join(product, archives[0]), cliPayload])
const cli = join(cliPayload, 'idrive/idrive')
if (!statSync(cli).isFile()) throw new Error('Prepared CLI is not a regular file')
accessSync(cli, constants.X_OK)
run(cli, ['--version'])
const htree = resolve(env.HTREE_BIN)
if (execFileSync(htree, ['--version'], { encoding: 'utf8' }).trim() !== 'htree 0.2.142') {
  throw new Error('Web journeys require the exact public htree 0.2.142 server')
}
const web = join(output, 'web')
const publicWeb = 'htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-drive-web'
const sourceConfig = join(output, 'source-config')
const sourceData = join(output, 'source-data')
mkdirSync(sourceConfig)
writeFileSync(join(sourceConfig, 'config.toml'), [
  '[storage]', `data_dir = ${JSON.stringify(sourceData)}`,
  '[nostr]', 'relays = ["wss://temp.iris.to", "wss://nos.lol", "wss://relay.snort.social"]',
  '[blossom]', 'servers = []',
  'read_servers = ["https://upload.iris.to", "https://cdn.iris.to", "https://blossom.primal.net"]',
  'write_servers = []', 'force_upload = false', '',
].join('\n'))
run('git', ['clone', publicWeb, web], { timeout: 600000, env: { ...env,
  PATH: `${dirname(htree)}:${env.PATH}`, HTREE_CONFIG_DIR: sourceConfig,
  HTREE_DATA_DIR: sourceData, HTREE_PREFER_LOCAL_DAEMON: '0', NOSTR_PREFER_LOCAL: '0' } })
run('git', ['checkout', '--detach', webCommit], { cwd: web })
if (execFileSync('git', ['rev-parse', 'HEAD'], { cwd: web, encoding: 'utf8' }).trim() !== webCommit
    || execFileSync('git', ['status', '--porcelain', '--untracked-files=all'], { cwd: web, encoding: 'utf8' }).trim()) {
  throw new Error('Published web checkout is not the exact clean requested revision')
}
run('pnpm', ['install', '--frozen-lockfile'], { cwd: web, timeout: 300000 })
run('pnpm', ['exec', 'playwright', 'install', '--with-deps', 'chromium'], { cwd: web, timeout: 300000 })
env = { ...env, CI: 'true', IRIS_DRIVE_BIN: cli, IRIS_DRIVE_REPO: root,
  PW_WORKERS: '1', PW_APP_PORT: '5186', PW_TEST_BLOSSOM_URL: 'http://127.0.0.1:18886',
  HTREE_ADDR: '127.0.0.1:18886', HTREE_CONFIG_DIR: join(output, 'htree-config'),
  HTREE_DATA_DIR: join(output, 'htree-data') }
const log = openSync(join(output, 'journeys.log'), 'wx')
try {
  run('pnpm', ['exec', 'playwright', 'test', 'e2e/shared-folder-journey.spec.ts',
    'e2e/shared-folder-reconnect.spec.ts', 'e2e/web-native-device-link.spec.ts',
    '--project=chromium', '--retries=0'], { cwd: web, timeout: 600000, stdio: ['ignore', log, log] })
} finally { closeSync(log) }
const results = readFileSync(join(output, 'journeys.log'), 'utf8')
if (!/4 passed/.test(results) || /[1-9]\d* skipped/.test(results)) throw new Error('All four journeys must execute and pass')
writeFileSync(join(output, 'receipt.json'), JSON.stringify({ schema: 1, status: 'passed', commit,
  webSource: publicWeb, webCommit, testsPassed: 4, testsSkipped: 0,
  htreeVersion: '0.2.142', htreeSha256: createHash('sha256').update(readFileSync(htree)).digest('hex'),
  product: JSON.parse(readFileSync(join(inputs, 'build-receipt.json'), 'utf8')) }, null, 2))
