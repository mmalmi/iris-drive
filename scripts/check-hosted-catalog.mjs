#!/usr/bin/env node
import { execFileSync } from 'node:child_process'
import { accessSync, constants, mkdirSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { archiveDigest } from './hosted-product-receipt.mjs'
import { irisSitesCommit, irisSitesSource, requirePassingCatalog } from './native-catalog-release.mjs'
import { releaseToolsEnv } from './release-tools.mjs'
import { stageHostedProduct } from './stage-hosted-product.mjs'

const root = resolve(process.env.SOURCE_ROOT)
const inputs = resolve(process.env.PRODUCT_INPUTS)
const output = resolve(process.env.CHECK_OUTPUT)
const scripts = dirname(fileURLToPath(import.meta.url))
const env = { ...process.env, CI: 'true' }
const run = (command, args, options = {}) => execFileSync(command, args, {
  cwd: root, env, stdio: 'inherit', ...options,
})
const git = (cwd, ...args) => execFileSync('git', args, { cwd, encoding: 'utf8' }).trim()
const commit = git(root, 'rev-parse', 'HEAD')
if (commit !== env.SOURCE_COMMIT) throw new Error('Native source candidate mismatch')

const product = stageHostedProduct({ repoRoot: root, productDirectory: inputs, platform: 'linux',
  run, env, destinationParent: output })
const archives = readdirSync(product).filter(name => name.endsWith('-x86_64-unknown-linux-gnu.tar.gz'))
if (archives.length !== 1) throw new Error('Expected exactly one packaged Linux CLI archive')
const [name] = archives
const archive = join(product, name)
const cliPayload = join(output, 'cli-payload')
mkdirSync(cliPayload)
run('python3', [join(scripts, 'unpack-hosted-product.py'), archive, cliPayload])
const cli = join(cliPayload, 'idrive/idrive')
if (!statSync(cli).isFile()) throw new Error('Prepared CLI is not a regular file')
accessSync(cli, constants.X_OK)

const catalog = join(output, 'iris-sites')
const sourceConfig = join(output, 'source-config')
const sourceData = join(output, 'source-data')
mkdirSync(sourceConfig)
writeFileSync(join(sourceConfig, 'config.toml'), [
  '[storage]', `data_dir = ${JSON.stringify(sourceData)}`,
  '[nostr]', 'relays = ["wss://temp.iris.to", "wss://nos.lol", "wss://relay.snort.social"]',
  '[blossom]', 'servers = []',
  'read_servers = ["https://cdn.iris.to", "https://upload.iris.to", "https://blossom.primal.net"]',
  'write_servers = []', 'force_upload = false', '',
].join('\n'))
run('git', ['clone', irisSitesSource, catalog], { timeout: 25 * 60_000, env: releaseToolsEnv({ env: {
  ...env, HTREE_CONFIG_DIR: sourceConfig, HTREE_DATA_DIR: sourceData,
  HTREE_PREFER_LOCAL_DAEMON: '0', NOSTR_PREFER_LOCAL: '0',
} }) })
run('git', ['checkout', '--detach', irisSitesCommit], { cwd: catalog })
if (git(catalog, 'rev-parse', 'HEAD') !== irisSitesCommit
    || git(catalog, 'status', '--porcelain', '--untracked-files=all')) {
  throw new Error('Iris Sites checkout must match the pinned clean source')
}
run('pnpm', ['install', '--frozen-lockfile'], { cwd: catalog, timeout: 300000 })
// Dedicated lab images already provide Chromium's OS libraries.
const systemDependencies = env.IRIS_DRIVE_CATALOG_SYSTEM_DEPS_PREINSTALLED === '1' ? [] : ['--with-deps']
run('pnpm', ['exec', 'playwright', 'install', ...systemDependencies, 'chromium'], { cwd: catalog, timeout: 300000 })

const execCatalog = file => import(pathToFileURL(join(catalog, 'scripts', file)))
const { withFreshGateway } = await execCatalog('native-release-gate.mjs')
const { runNativeSmoke } = await execCatalog('native-launcher-smoke.mjs')
const results = await withFreshGateway(port => runNativeSmoke({ port }), cli)
requirePassingCatalog(results)
writeFileSync(join(output, 'receipt.json'), JSON.stringify({
  schema: 1, status: 'passed', commit, toolingCommit: git(scripts, 'rev-parse', 'HEAD'),
  catalogSource: irisSitesSource, catalogCommit: irisSitesCommit,
  linuxCliArchive: { name, ...await archiveDigest(archive) },
  linuxCliSha256: (await archiveDigest(cli)).sha256, results,
}, null, 2) + '\n', { flag: 'wx' })
