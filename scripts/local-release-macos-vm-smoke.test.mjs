import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { macosBundleId, macosReleaseSmokeCommand, runningAppsWithBundleId } from './macos-release-smoke-host.mjs'

const repoRoot = fileURLToPath(new URL('..', import.meta.url))
const localSmoke = join(repoRoot, 'scripts', 'macos-release-smoke.sh')
const vmSmoke = join(repoRoot, 'scripts', 'macos-vm-release-smoke.sh')
const running = pid => () => [{ pid, app: '/Applications/Iris Drive.app' }]

test('the release smoke moves to the Mac VM when this Mac runs the same app, and says why', () => {
  const logs = []
  const command = macosReleaseSmokeCommand({ env: {}, repoRoot, platform: 'darwin', running: running(4242),
    log: line => logs.push(line) })
  assert.equal(command, vmSmoke)
  assert.match(logs.join('\n'), /on the Mac VM, because to\.iris\.drive\.macos is running here \(pid 4242\)/)
  assert.match(logs.join('\n'), /IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST=local/)
  assert.equal(macosReleaseSmokeCommand({ env: {}, repoRoot, platform: 'darwin', running: () => [] }), localSmoke)
  assert.equal(macosReleaseSmokeCommand({ env: {}, repoRoot, platform: 'linux', running: running(1) }), localSmoke)
})

test('an explicit host or command overrides the automatic choice', () => {
  const quiet = () => {}
  const options = { repoRoot, platform: 'darwin', running: running(1), log: quiet }
  assert.equal(macosReleaseSmokeCommand({ ...options, env: { IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST: 'local' } }), localSmoke)
  assert.equal(macosReleaseSmokeCommand({ ...options, running: () => [], env: { IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST: 'vm' } }), vmSmoke)
  assert.equal(macosReleaseSmokeCommand({ ...options, env: { IRIS_DRIVE_MACOS_RELEASE_SMOKE_COMMAND: '/reserved/smoke' } }),
    '/reserved/smoke')
  assert.throws(() => macosReleaseSmokeCommand({ ...options, env: { IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST: 'phone' } }),
    /must be local or vm/)
})

test('running apps are matched by the bundle id of their app, not by name', () => {
  assert.equal(macosBundleId(repoRoot), 'to.iris.drive.macos')
  const processes = () => [
    '  101 /Applications/Iris Drive.app/Contents/MacOS/Iris Drive',
    '  102 /Applications/Iris Drive.app/Contents/MacOS/idrive',
    '  103 /tmp/Old Iris Drive.app/Contents/MacOS/Iris Drive',
    '  104 /Applications/Iris Drive.app/Contents/PlugIns/IrisDriveFileProvider.appex/Contents/MacOS/IrisDriveFileProvider',
    '  105 /usr/sbin/cfprefsd',
  ].join('\n')
  const ids = { '/Applications/Iris Drive.app': 'to.iris.drive.macos', '/tmp/Old Iris Drive.app': 'to.iris.drive.test' }
  assert.deepEqual(runningAppsWithBundleId('to.iris.drive.macos', { processes, readBundleId: app => ids[app] ?? '' })
    .map(app => app.pid), [101, 102])
})

function fakeLab({ smokeStatus = 0 } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-vm-release-smoke-'))
  const bin = join(root, 'bin')
  mkdirSync(bin)
  const log = join(root, 'calls.log')
  const tool = (name, body) => {
    writeFileSync(join(bin, name), `#!/bin/sh\nprintf '%s\\n' "${name} $*" >> "${log}"\n${body}\n`)
    chmodSync(join(bin, name), 0o755)
  }
  tool('pgrep', 'exit 1')
  tool('scp', 'exit 0')
  tool('ssh', `case "$*" in *pgrep*) exit 1 ;; *macos-release-smoke.sh*) exit ${smokeStatus} ;; esac; exit 0`)
  const app = join(root, 'Iris Drive.app')
  mkdirSync(app)
  const archive = join(root, 'iris-drive-v9.9.9-macos-arm64.app.tar.gz')
  const dmg = join(root, 'iris-drive-v9.9.9-macos-arm64.dmg')
  writeFileSync(archive, 'archive')
  writeFileSync(dmg, 'dmg')
  const run = (args = ['--app', app, '--archive', archive, '--dmg', dmg]) => spawnSync('bash', [vmSmoke, ...args], {
    encoding: 'utf8',
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, IRIS_DRIVE_DEV_LAB_ENV: join(root, 'none.env'),
      IRIS_DRIVE_MACOS_SSH_HOST: 'mac-lab.test', IRIS_DRIVE_MACOS_SKIP_GIT_SYNC: '1' },
  })
  return { run, calls: () => readFileSync(log, 'utf8') }
}

test('the VM runner copies the signed artifacts and runs the release smoke in the synced VM checkout', () => {
  const lab = fakeLab()
  const result = lab.run()
  assert.equal(result.status, 0, result.stderr)
  const calls = lab.calls()
  assert.match(calls, /^pgrep -f scripts\/release-gate\.sh$/m)
  assert.match(calls, /^scp -q -o BatchMode=yes .*macos-arm64\.app\.tar\.gz .*macos-arm64\.dmg mac-lab\.test:iris-drive-release-smoke-/m)
  assert.match(calls, /ssh .*mac-lab\.test .*iris-drive-release-gate.*\.\/scripts\/macos-release-smoke\.sh --app .*Iris Drive\.app.* --archive .* --dmg /)
  assert.match(calls, /ssh .*rm -rf/)
})

test('a failing VM smoke fails the release step and keeps its files on the VM', () => {
  const lab = fakeLab({ smokeStatus: 3 })
  const result = lab.run()
  assert.equal(result.status, 3)
  assert.match(result.stderr, /kept ~\/iris-drive-release-smoke-.* there for inspection/)
  assert.doesNotMatch(lab.calls(), /rm -rf/)
  assert.equal(fakeLab().run(['--app', '/missing']).status, 2)
})

test('local-release dry run uses the VM runner when asked and keeps the same smoke arguments', () => {
  const result = spawnSync(process.execPath, [fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
    '--build', '--dry-run', '--tag', 'v9.9.9', '--only', 'macos'], { encoding: 'utf8',
    env: { ...process.env, IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST: 'vm', IRIS_DRIVE_MACOS_RELEASE_SMOKE_COMMAND: '' } })
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /scripts\/macos-vm-release-smoke\.sh --app .*Iris Drive\.app"? --archive .* --dmg /)
})
