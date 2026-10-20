import assert from 'node:assert/strict'
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { promisify } from 'node:util'
import { execFile } from 'node:child_process'
import test from 'node:test'

const execFileAsync = promisify(execFile)
const repoRoot = resolve(import.meta.dirname, '..')
const releaseGate = join(repoRoot, 'scripts', 'release-gate.sh')

const laneStub = `#!/usr/bin/env bash
set -Eeuo pipefail

command_name="$(basename "$0")"
arguments="$*"
lane=""
group=""
case "$command_name:$arguments" in
  "macos-vm-smoke-stub:")
    printf '%s\n' "\${IRIS_DRIVE_MACOS_SMOKE_LINK_JOURNEY-unset}" \
      >"$IRIS_DRIVE_GATE_TEST_EVENTS/macos-link-journey"
    touch "$IRIS_DRIVE_GATE_TEST_EVENTS/native-macos.started"
    for _ in $(seq 1 500); do
      [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-android.started" \
        && -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-ios.started" ]] && break
      sleep 0.01
    done
    [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-android.started" \
      && -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-ios.started" ]] || exit 98
    touch "$IRIS_DRIVE_GATE_TEST_EVENTS/macos-smoke.finished"
    exit 0
    ;;
  "just:android-gui-smoke")
    touch "$IRIS_DRIVE_GATE_TEST_EVENTS/native-android.started"
    for _ in $(seq 1 500); do
      [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-macos.started" \
        && -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-ios.started" ]] && break
      sleep 0.01
    done
    [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-macos.started" \
      && -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-ios.started" ]] || exit 98
    touch "$IRIS_DRIVE_GATE_TEST_EVENTS/android-gui-smoke.finished"
    exit 0
    ;;
  "just:ios-smoke --no-build")
    [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/ios-gui-smoke.finished" ]] || exit 96
    touch "$IRIS_DRIVE_GATE_TEST_EVENTS/ios-smoke.finished"
    exit 0
    ;;
  "just:ios-gui-smoke")
    touch "$IRIS_DRIVE_GATE_TEST_EVENTS/native-ios.started"
    for _ in $(seq 1 500); do
      [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-macos.started" \
        && -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-android.started" ]] && break
      sleep 0.01
    done
    [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-macos.started" \
      && -f "$IRIS_DRIVE_GATE_TEST_EVENTS/native-android.started" ]] || exit 98
    touch "$IRIS_DRIVE_GATE_TEST_EVENTS/ios-gui-smoke.finished"
    exit 0
    ;;
  "just:e2e-5devices") touch "$IRIS_DRIVE_GATE_TEST_EVENTS/e2e-5devices.finished"; exit 0 ;;
  "git:remote get-url macos") printf '%s\n' 'configured-macos:/repo'; exit 0 ;;
  "python3:scripts/test_release_workflows.py") lane=release-workflow-tests ;;
  node:*) lane=local-release-tests ;;
  "cargo:fmt --check") lane=fmt ;;
  "cargo:test --workspace --exclude idrive") lane=workspace-tests ;;
  cargo:*"--no-run") lane=idrive-compile ;;
  cargo:*"--test daemon_sync_matrix"*) lane=daemon-tests; group=rust ;;
  cargo:"test -p idrive"*) lane=cli-tests; group=rust ;;
  "cargo:build --workspace --release")
    for expected in release-workflow-tests local-release-tests fmt structure workspace-tests cli-tests daemon-tests; do
      [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/$expected.finished" ]] || exit 95
    done
    touch "$IRIS_DRIVE_GATE_TEST_EVENTS/workspace-release-build.finished"
    exit 0
    ;;
  "just:structure") lane=structure ;;
  uname:*) printf '%s\n' "\${IRIS_DRIVE_GATE_TEST_SYSTEM:-TestOS}"; exit 0 ;;
  *) printf 'unexpected release-gate command: %s %s\n' "$command_name" "$arguments" >&2; exit 97 ;;
esac

touch "$IRIS_DRIVE_GATE_TEST_EVENTS/$lane.started"
for _ in $(seq 1 500); do
  ready=1
  if [[ "$group" == rust ]]; then
    expected=cli-tests
    [[ "$lane" == cli-tests ]] && expected=daemon-tests
    [[ -f "$IRIS_DRIVE_GATE_TEST_EVENTS/$expected.started" ]] || ready=0
  else
  for expected in release-workflow-tests local-release-tests fmt structure workspace-tests idrive-compile; do
    if [[ ! -f "$IRIS_DRIVE_GATE_TEST_EVENTS/$expected.started" ]]; then
      ready=0
      break
    fi
  done
  fi
  [[ "$ready" == 1 ]] && break
  sleep 0.01
done
if [[ "$ready" != 1 ]]; then
  printf 'parallel release-gate rendezvous timed out in %s\n' "$lane" >&2
  exit 98
fi
touch "$IRIS_DRIVE_GATE_TEST_EVENTS/$lane.finished"
if [[ "\${IRIS_DRIVE_GATE_TEST_FAIL_LANE:-}" == "$lane" ]]; then
  printf 'intentional %s failure\n' "$lane" >&2
  exit 42
fi
`

async function makeHarness() {
  const root = await mkdtemp(join(tmpdir(), 'iris-drive-release-gate-test-'))
  const bin = join(root, 'bin')
  const events = join(root, 'events')
  await Promise.all([
    mkdir(bin),
    mkdir(events),
  ])
  for (const command of ['cargo', 'git', 'just', 'macos-vm-smoke-stub', 'node', 'python3', 'uname']) {
    const path = join(bin, command)
    await writeFile(path, laneStub)
    await chmod(path, 0o755)
  }
  return { root, bin, events }
}

async function runGate({ failLane = '', full = false, macosHost = 'test-macos-vm', system = 'TestOS' } = {}) {
  const harness = await makeHarness()
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    SOURCE_DATE_EPOCH: '0',
    IRIS_DRIVE_GATE_TEST_EVENTS: harness.events,
    IRIS_DRIVE_GATE_TEST_FAIL_LANE: failLane,
    IRIS_DRIVE_GATE_TEST_SYSTEM: system,
    IRIS_DRIVE_DEV_LAB_ENV: join(harness.root, 'missing-dev-lab.env'),
    IRIS_DRIVE_DEV_VM_MACOS_REMOTE: 'macos',
    IRIS_DRIVE_RELEASE_GATE_IDLE_CPU: '0',
    IRIS_DRIVE_MACOS_SSH_HOST: macosHost,
    IRIS_DRIVE_RELEASE_GATE_MACOS_SMOKE_COMMAND: 'macos-vm-smoke-stub',
  }
  try {
    const result = await execFileAsync(releaseGate, full ? ['--full'] : [], {
      cwd: repoRoot,
      env,
      timeout: 15_000,
    })
    return { ...result, ...harness }
  } catch (error) {
    Object.assign(error, harness)
    throw error
  }
}

async function cleanup(root) {
  await rm(root, { recursive: true, force: true })
}

test('release gate starts independent checks before joining', async () => {
  let result
  try {
    result = await runGate()
    for (const lane of [
      'release-workflow-tests',
      'local-release-tests',
      'fmt',
      'structure',
      'workspace-tests',
      'idrive-compile',
      'cli-tests',
      'daemon-tests',
      'workspace-release-build',
    ]) {
      await readFile(join(result.events, `${lane}.finished`))
    }
  } finally {
    if (result?.root) await cleanup(result.root)
  }
})

test('release gate drains parallel checks and labels a failed lane', async () => {
  let failure
  try {
    await runGate({ failLane: 'daemon-tests' })
    assert.fail('release gate unexpectedly passed')
  } catch (error) {
    failure = error
    assert.notEqual(error.code, 0)
    assert.match(error.stderr, /\[release-gate:idrive-tests\].*intentional daemon-tests failure/s)
    assert.match(error.stderr, /daemon-tests failed/)
    await assert.rejects(readFile(join(error.events, 'workspace-release-build.finished')))
  } finally {
    if (failure?.root) await cleanup(failure.root)
  }
})

test('release gate auto-routes macOS to the configured VM and overlaps native checks', async () => {
  let result
  try {
    result = await runGate({ full: true, macosHost: '', system: 'Darwin' })
    for (const marker of [
      'macos-smoke.finished',
      'ios-smoke.finished',
      'ios-gui-smoke.finished',
      'android-gui-smoke.finished',
      'e2e-5devices.finished',
    ]) {
      await readFile(join(result.events, marker))
    }
    assert.equal(await readFile(join(result.events, 'macos-link-journey'), 'utf8'), '1\n')
  } finally {
    if (result?.root) await cleanup(result.root)
  }
})
