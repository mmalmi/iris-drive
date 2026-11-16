// Choose where the notarized-release launch smoke runs. The smoke stops and
// restarts any running Iris Drive with the same bundle id, so when this Mac's
// own Iris Drive is running it goes to the lab Mac VM instead.
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

export function macosBundleId(repoRoot) {
  const project = readFileSync(join(repoRoot, 'macos', 'project.yml'), 'utf8')
  return project.match(/PRODUCT_BUNDLE_IDENTIFIER:\s*(\S+)/)[1]
}

function bundleIdOf(app) {
  const result = spawnSync('plutil', ['-extract', 'CFBundleIdentifier', 'raw', '-o', '-',
    join(app, 'Contents', 'Info.plist')], { encoding: 'utf8' })
  return result.status === 0 ? result.stdout.trim() : ''
}

export function runningAppsWithBundleId(bundleId, {
  processes = () => spawnSync('ps', ['-axo', 'pid=,comm='], { encoding: 'utf8' }).stdout ?? '',
  readBundleId = bundleIdOf,
} = {}) {
  const apps = []
  for (const line of processes().split('\n')) {
    const match = line.trim().match(/^(\d+)\s+(.+)\/Contents\/MacOS\/[^/]+$/)
    if (match && readBundleId(match[2]) === bundleId) apps.push({ pid: Number(match[1]), app: match[2] })
  }
  return apps
}

export function macosReleaseSmokeCommand({
  env, repoRoot, log = console.log, platform = process.platform, running = runningAppsWithBundleId,
}) {
  if (env.IRIS_DRIVE_MACOS_RELEASE_SMOKE_COMMAND) return env.IRIS_DRIVE_MACOS_RELEASE_SMOKE_COMMAND
  const local = join(repoRoot, 'scripts', 'macos-release-smoke.sh')
  const vm = join(repoRoot, 'scripts', 'macos-vm-release-smoke.sh')
  const host = String(env.IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST ?? '').trim().toLowerCase()
  if (host === 'local') return local
  if (host === 'vm') {
    log('macOS release smoke: on the Mac VM (IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST=vm)')
    return vm
  }
  if (host) throw new Error('IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST must be local or vm')
  const bundleId = macosBundleId(repoRoot)
  const apps = platform === 'darwin' ? running(bundleId) : []
  if (apps.length === 0) return local
  log(`macOS release smoke: on the Mac VM, because ${bundleId} is running here `
    + `(pid ${apps.map(app => app.pid).join(', ')}) and the smoke would stop and restart it. `
    + 'Set IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST=local to run it here.')
  return vm
}
