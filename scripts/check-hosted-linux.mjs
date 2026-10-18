#!/usr/bin/env node
import { spawn, spawnSync, execFileSync } from 'node:child_process'
import { accessSync, constants, copyFileSync, existsSync, linkSync, mkdirSync, openSync, closeSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { join, resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'
import { stageHostedProduct } from './stage-hosted-product.mjs'

const root = resolve(process.env.SOURCE_ROOT)
const inputs = resolve(process.env.PRODUCT_INPUTS)
const output = resolve(process.env.CHECK_OUTPUT)
const scripts = dirname(fileURLToPath(import.meta.url))
mkdirSync(output, { recursive: true })
let env = { ...process.env }
function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit', ...options })
  if (result.status !== 0) throw new Error(`${command} failed: ${result.status ?? result.error}`)
}
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
if (commit !== env.SOURCE_COMMIT) throw new Error('Source candidate mismatch')
const product = stageHostedProduct({ repoRoot: root, productDirectory: inputs,
  platform: 'linux', run, env, destinationParent: output })
const receipt = JSON.parse(readFileSync(join(inputs, 'build-receipt.json'), 'utf8'))
const debs = readdirSync(product).filter(name => name.endsWith('-linux-x64.deb'))
const archives = readdirSync(product).filter(name => name.endsWith('-x86_64-unknown-linux-gnu.tar.gz'))
if (debs.length !== 1 || archives.length !== 1) throw new Error('Expected exactly one desktop and one CLI product')
const fixture = join(output, 'fixture')
const cliDir = join(fixture, 'target/debug')
const appDir = join(fixture, 'linux/target/debug')
mkdirSync(cliDir, { recursive: true })
mkdirSync(appDir, { recursive: true })
const cliPayload = join(output, 'cli-payload')
mkdirSync(cliPayload)
run('python3', [join(scripts, 'unpack-hosted-product.py'), join(product, archives[0]), cliPayload])
run('dpkg-deb', ['-x', join(product, debs[0]), join(output, 'unpacked')])
const cli = join(cliDir, 'idrive')
const app = join(appDir, 'iris-drive')
linkSync(join(cliPayload, 'idrive/idrive'), cli)
linkSync(join(output, 'unpacked/usr/bin/iris-drive'), app)
for (const file of [cli, app]) {
  if (!statSync(file).isFile()) throw new Error('Prepared executable is not a regular file')
  accessSync(file, constants.X_OK)
}
for (const relative of ['scripts/lib/linux-gui-status.sh', 'scripts/lib/linux-approve-device.py', 'scripts/idle-cpu-gate.sh']) {
  mkdirSync(dirname(join(fixture, relative)), { recursive: true })
  copyFileSync(join(root, relative), join(fixture, relative))
}
let smoke = readFileSync(join(root, 'scripts/desktop-gui-smoke.sh'), 'utf8')
const marker = '        echo "LINUX_GUI_SMOKE_OK state=$expected_state"'
if (smoke.split(marker).length !== 2) throw new Error('Canonical GUI readiness assertion changed')
smoke = smoke.replace(marker, `        cp "$screenshot" "$IRIS_DRIVE_HOSTED_GUI_SCREENSHOT"
        IRIS_DRIVE_IDLE_CPU_WARMUP_SECS=30 IRIS_DRIVE_IDLE_CPU_DURATION_SECS=65 \\
          IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH="$repo" bash "$repo/scripts/idle-cpu-gate.sh" --platform linux || exit $?
${marker}`)
const build = '  (cd "$repo/linux" && cargo build)'
if (smoke.split(build).length !== 2) throw new Error('Canonical GUI build section changed')
smoke = smoke.replace(build, '  die "The verified prepared GTK product is required"')
const smokePath = join(fixture, 'scripts/desktop-gui-idle.sh')
writeFileSync(smokePath, smoke)
run('bash', ['-n', smokePath])
const profile = join(output, 'profile')
env = { ...env, IRIS_DRIVE_REPO: fixture, IRIS_DRIVE_CONFIG_DIR: profile,
  IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR: profile, HTREE_DATA_DIR: join(output, 'hashtree'),
  IRIS_DRIVE_CLI: cli, IRIS_DRIVE_DEV_VM_LINUX_DISPLAY: ':97',
  IRIS_DRIVE_HOSTED_GUI_SCREENSHOT: join(output, 'gui.png') }
run(cli, ['--version'])
run(cli, ['--config-dir', profile, 'init', '--label', 'Linux release check'])
const log = openSync(join(output, 'daemon.log'), 'wx')
const daemon = spawn(cli, ['--config-dir', profile, 'daemon', '--watch-interval', '0', '--no-gateway'],
  { cwd: root, env, stdio: ['ignore', log, log] })
let daemonError
daemon.on('error', error => { daemonError = error })
const stopped = new Promise(resolve => daemon.once('close', resolve))
try {
  const deadline = Date.now() + 20000
  while (!existsSync(join(profile, 'daemon-status.json'))) {
    if (daemonError || daemon.exitCode !== null || Date.now() > deadline) throw new Error('Prepared daemon did not become ready', { cause: daemonError })
    await delay(200)
  }
  run('bash', [smokePath, 'linux', 'local'], { timeout: 240000 })
} finally {
  daemon.kill('SIGTERM')
  await Promise.race([stopped, delay(10000)])
  if (daemon.exitCode === null && daemon.signalCode === null) daemon.kill('SIGKILL')
  await stopped
  closeSync(log)
}
writeFileSync(join(output, 'receipt.json'), JSON.stringify({ schema: 1, status: 'passed', commit,
  gates: ['linux-canonical-gui', 'linux-release-idle-65s'], product: receipt }, null, 2))
