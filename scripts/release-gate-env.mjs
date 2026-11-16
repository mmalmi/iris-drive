#!/usr/bin/env node
// Run a release-gate command with the lab environment derived from the local
// dev-lab file and git remotes. Host names stay in local configuration and are
// never printed; only the names of the variables that were set are.
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import os from 'node:os'
import { delimiter, dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { parseEnvFile } from './local-release-lib.mjs'

export const releaseGateSimulator = 'iPhone Iris Drive Release Gate'
export const releaseGateEmulator = 'emulator-5554'

export function devLabEnvPath(env = process.env) {
  return env.IRIS_DRIVE_DEV_LAB_ENV || join(os.homedir(), '.config', 'iris-drive', 'dev-lab.env')
}

// "user@host:path" remotes name an SSH destination; URL remotes do not.
export function sshDestinationForRemote(repoRoot, remote) {
  const result = spawnSync('git', ['remote', 'get-url', remote], { cwd: repoRoot, encoding: 'utf8' })
  const url = result.status === 0 ? result.stdout.trim() : ''
  return url.includes(':') && !url.includes('://') ? url.slice(0, url.indexOf(':')) : ''
}

function macJavaHome() {
  const found = spawnSync('/usr/libexec/java_home', ['-v', '17'], { encoding: 'utf8' })
  return found.status === 0 ? found.stdout.trim() : ''
}

export function releaseGateEnv({
  repoRoot, env = process.env, devLab = {}, fastPrechecked = false,
  remoteHost = remote => sshDestinationForRemote(repoRoot, remote),
  javaHome = () => (process.platform === 'darwin' ? macJavaHome() : ''),
}) {
  const lab = { ...devLab, ...env }
  const ubuntu = remoteHost(lab.IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE || 'ubuntu')
  const macos = remoteHost(lab.IRIS_DRIVE_DEV_VM_MACOS_REMOTE || 'macos')
  const windows = lab.IRIS_DRIVE_DEV_VM_WINDOWS_SSH_HOST || remoteHost(lab.IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE || 'windows')
  const derived = {
    IRIS_DRIVE_E2E_UBUNTU_HOST: ubuntu,
    IRIS_DRIVE_E2E_WINDOWS_HOST: windows,
    IRIS_DRIVE_E2E_MACOS_HOST: macos,
    IRIS_DRIVE_E2E_IOS_HOST: 'local',
    IRIS_DRIVE_E2E_ANDROID_HOST: 'local',
    IRIS_DRIVE_MACOS_SSH_HOST: macos,
    IRIS_DRIVE_RELEASE_GATE_MACOS_VM: macos ? '1' : '',
    IRIS_DRIVE_IOS_SIMULATOR_DEVICE: releaseGateSimulator,
    IRIS_DRIVE_ANDROID_SERIAL: lab.ANDROID_SERIAL || releaseGateEmulator,
    ANDROID_SERIAL: lab.IRIS_DRIVE_ANDROID_SERIAL || releaseGateEmulator,
    JAVA_HOME: lab.JAVA_HOME || javaHome(),
    CARGO_INCREMENTAL: '0',
    CARGO_PROFILE_DEV_DEBUG: '0',
    CARGO_PROFILE_TEST_DEBUG: '0',
    ...(fastPrechecked ? { IRIS_DRIVE_RELEASE_GATE_FAST_PRECHECKED: '1' } : {}),
  }
  const result = { ...devLab, ...env }
  const set = []
  for (const [name, value] of Object.entries(derived)) {
    if (!env[name] && value) {
      result[name] = value
      set.push(name)
    }
  }
  if (result.JAVA_HOME) result.PATH = [join(result.JAVA_HOME, 'bin'), env.PATH].filter(Boolean).join(delimiter)
  const missing = ['IRIS_DRIVE_E2E_UBUNTU_HOST', 'IRIS_DRIVE_E2E_WINDOWS_HOST', 'IRIS_DRIVE_MACOS_SSH_HOST', 'JAVA_HOME']
    .filter(name => !result[name])
  return { env: result, set, missing }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2)
  const separator = args.indexOf('--')
  const flags = separator < 0 ? args : args.slice(0, separator)
  const command = separator < 0 ? [] : args.slice(separator + 1)
  if (command.length === 0 || flags.some(flag => flag !== '--fast-prechecked')) {
    console.error('Usage: node scripts/release-gate-env.mjs [--fast-prechecked] -- <command> [args...]')
    process.exit(2)
  }
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
  const labFile = devLabEnvPath()
  const devLab = existsSync(labFile) ? parseEnvFile(readFileSync(labFile, 'utf8')) : {}
  if (!existsSync(labFile)) console.error(`release gate env: no dev-lab file at ${labFile}`)
  const { env, set, missing } = releaseGateEnv({ repoRoot, devLab, fastPrechecked: flags.includes('--fast-prechecked') })
  console.error(`release gate env: set ${set.join(', ') || 'nothing'}`)
  if (missing.length) console.error(`release gate env: not configured: ${missing.join(', ')}`)
  const result = spawnSync(command[0], command.slice(1), { cwd: repoRoot, env, stdio: 'inherit' })
  process.exit(result.status ?? 1)
}
