import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { delimiter, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { fixtureCheckout } from './lib/release-fixture.mjs'
import { releaseGateEnv, sshDestinationForRemote } from './release-gate-env.mjs'

const devLab = {
  IRIS_DRIVE_DEV_VM_MACOS_REMOTE: 'mac-vm',
  IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE: 'linux-vm',
  IRIS_DRIVE_DEV_VM_WINDOWS_SSH_HOST: 'win-lab',
}

function labCheckout() {
  const checkout = fixtureCheckout()
  checkout.git('remote', 'add', 'mac-vm', 'admin@mac-lab.test:src/iris-drive')
  checkout.git('remote', 'add', 'linux-vm', 'dev@linux-lab.test:src/iris-drive')
  checkout.git('remote', 'add', 'github', 'https://github.com/owner/drive.git')
  return checkout
}

test('lab hosts come from the dev-lab remotes and phones from the reserved local devices', () => {
  const checkout = labCheckout()
  const { env, set, missing } = releaseGateEnv({ repoRoot: checkout.root, env: { PATH: '/usr/bin' }, devLab,
    javaHome: () => '/opt/jdk17' })
  assert.equal(env.IRIS_DRIVE_E2E_MACOS_HOST, 'admin@mac-lab.test')
  assert.equal(env.IRIS_DRIVE_MACOS_SSH_HOST, 'admin@mac-lab.test')
  assert.equal(env.IRIS_DRIVE_E2E_UBUNTU_HOST, 'dev@linux-lab.test')
  assert.equal(env.IRIS_DRIVE_E2E_WINDOWS_HOST, 'win-lab')
  assert.equal(env.IRIS_DRIVE_E2E_IOS_HOST, 'local')
  assert.equal(env.IRIS_DRIVE_RELEASE_GATE_MACOS_VM, '1')
  assert.equal(env.IRIS_DRIVE_IOS_SIMULATOR_DEVICE, 'iPhone Iris Drive Release Gate')
  assert.equal(env.ANDROID_SERIAL, 'emulator-5554')
  assert.equal(env.PATH, ['/opt/jdk17/bin', '/usr/bin'].join(delimiter))
  assert.equal(env.CARGO_PROFILE_DEV_DEBUG, '0')
  assert.equal(env.IRIS_DRIVE_RELEASE_GATE_FAST_PRECHECKED, undefined)
  assert.deepEqual(missing, [])
  assert.ok(set.includes('IRIS_DRIVE_MACOS_SSH_HOST') && !set.some(name => name.includes('lab.test')))
})

test('explicit settings win and fast-check reuse must be asked for', () => {
  const checkout = labCheckout()
  const { env, set } = releaseGateEnv({ repoRoot: checkout.root, devLab, fastPrechecked: true, javaHome: () => '',
    env: { IRIS_DRIVE_MACOS_SSH_HOST: 'other-mac', ANDROID_SERIAL: 'R5CT', IRIS_DRIVE_IOS_SIMULATOR_DEVICE: 'UDID-1' } })
  assert.equal(env.IRIS_DRIVE_MACOS_SSH_HOST, 'other-mac')
  assert.equal(env.IRIS_DRIVE_ANDROID_SERIAL, 'R5CT')
  assert.equal(env.IRIS_DRIVE_IOS_SIMULATOR_DEVICE, 'UDID-1')
  assert.equal(env.IRIS_DRIVE_RELEASE_GATE_FAST_PRECHECKED, '1')
  assert.ok(!set.includes('IRIS_DRIVE_MACOS_SSH_HOST'))
})

test('URL remotes and absent remotes are not treated as SSH hosts', () => {
  const checkout = labCheckout()
  assert.equal(sshDestinationForRemote(checkout.root, 'github'), '')
  assert.equal(sshDestinationForRemote(checkout.root, 'missing'), '')
  const { missing } = releaseGateEnv({ repoRoot: checkout.root, env: {}, devLab: {}, javaHome: () => '' })
  assert.deepEqual(missing, ['IRIS_DRIVE_E2E_UBUNTU_HOST', 'IRIS_DRIVE_E2E_WINDOWS_HOST', 'IRIS_DRIVE_MACOS_SSH_HOST', 'JAVA_HOME'])
})

test('the wrapper runs the command with the derived environment and prints names, not hosts', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-gate-env-'))
  const labFile = join(root, 'dev-lab.env')
  writeFileSync(labFile, 'IRIS_DRIVE_DEV_VM_WINDOWS_SSH_HOST=win-lab.test\n')
  const result = spawnSync(process.execPath, [fileURLToPath(new URL('./release-gate-env.mjs', import.meta.url)),
    '--', 'sh', '-c', 'printf "%s|%s" "$IRIS_DRIVE_E2E_WINDOWS_HOST" "$IRIS_DRIVE_E2E_IOS_HOST"'],
  { encoding: 'utf8', env: { ...process.env, IRIS_DRIVE_DEV_LAB_ENV: labFile } })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.stdout, 'win-lab.test|local')
  assert.match(result.stderr, /release gate env: set .*IRIS_DRIVE_E2E_WINDOWS_HOST/)
  assert.ok(!result.stderr.includes('win-lab.test'))
})
