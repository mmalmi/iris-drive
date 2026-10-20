#!/usr/bin/env node
import { spawnSync, execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { join, resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { stageHostedProduct } from './stage-hosted-product.mjs'

const root = resolve(process.env.SOURCE_ROOT)
const inputs = resolve(process.env.PRODUCT_INPUTS)
const scripts = dirname(fileURLToPath(import.meta.url))
const output = resolve(process.env.CHECK_OUTPUT)
mkdirSync(output, { recursive: true })
let env = { ...process.env, CARGO_TARGET_DIR: join(root, 'target') }
function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit', ...options })
  if (result.status !== 0) throw new Error(`${command} failed: ${result.status ?? result.error}`)
}
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
if (commit !== env.SOURCE_COMMIT) throw new Error('Source candidate mismatch')
function stage(name, platform) {
  return stageHostedProduct({ repoRoot: root, productDirectory: join(inputs, name), platform, run, env, destinationParent: output })
}
function derived(name, payload) {
  const path = join(output, name)
  mkdirSync(path)
  renameSync(payload, join(path, 'Build'))
  return path
}
const debug = derived('Debug', stage('debug', 'ios-sim-debug'))
const release = derived('Release', stage('release', 'ios-sim-release'))
const mac = stage('mac', 'macos-release')
const cli = join(mac, 'idrive')
run('codesign', ['--force', '--sign', '-', cli])
run('codesign', ['--verify', '--strict', cli])
run('python3', [join(scripts, 'prepare-hosted-ios-check.py'), root, output])
const devices = JSON.parse(execFileSync('xcrun', ['simctl', 'list', 'devices', 'available', '--json'], { encoding: 'utf8' })).devices
const device = Object.entries(devices).filter(([runtime]) => runtime.includes('iOS-26-2'))
  .flatMap(([, entries]) => entries).find(item => item.name.startsWith('iPhone'))
if (!device) throw new Error('Hosted runner has no iOS26.2 iPhone simulator')
env = { ...env, IRIS_DRIVE_IDRIVE_BIN: cli, IRIS_DRIVE_IOS_SIMULATOR_DEVICE: device.udid,
  IRIS_DRIVE_IOS_UI_BUILD_LOG: join(output, 'xcode-tests.log'),
  IRIS_DRIVE_PREPARED_IOS_DERIVED_DATA: debug }
run('bash', [join(output, 'ios-gui-linking-smoke.sh')], { timeout: 40 * 60 * 1000 })
run('xcrun', ['simctl', 'io', device.udid, 'screenshot', join(output, 'gui.png')])
run('bash', [join(output, 'ios-simulator-smoke.sh'), '--no-build'], { timeout: 15 * 60 * 1000 })
env = { ...env, IRIS_DRIVE_IOS_XCODE_CONFIGURATION: 'Release',
  IRIS_DRIVE_PREPARED_IOS_DERIVED_DATA: release, IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES: 'app',
  IRIS_DRIVE_IDLE_CPU_IOS_DEVICE: device.udid, IRIS_DRIVE_IDLE_CPU_DURATION_SECS: '65' }
run('bash', [join(output, 'ios-simulator-smoke.sh'), '--install-only'], { timeout: 180000 })
run('bash', [join(root, 'scripts/idle-cpu-gate.sh'), '--platform', 'ios'], { timeout: 240000 })
const products = Object.fromEntries(['debug', 'release', 'mac'].map(name => [name,
  JSON.parse(readFileSync(join(inputs, name, 'build-receipt.json'), 'utf8'))]))
writeFileSync(join(output, 'receipt.json'), JSON.stringify({ schema: 1, status: 'passed', commit,
  gates: ['ios-gui-linking', 'ios-simulator-provider', 'ios-release-idle-65s'], products }, null, 2))
