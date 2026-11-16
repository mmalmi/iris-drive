import { execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export function requirePublishedHead(result, tag, root) {
  if (result.verified !== true || result.tag !== tag || !root || result.root_cid !== root) {
    throw new Error(`Expected the signed release head for ${tag}, observed ${result.tag || 'no release'}`)
  }
}

// A relay-only publisher can feed the signed announcement to a running Drive
// seed. Only a separate, empty-profile mesh reader can verify distribution.
export function verifyPublishedMeshUpdate({ env, npub, releaseTree, tag, root, dryRun }) {
  console.log(`Verify ${tag} through a fresh Drive mesh update check`)
  if (dryRun) return
  const idrive = env.IRIS_DRIVE_RELEASE_CHECK_IDRIVE || 'idrive'
  const reference = `htree://${npub}/${encodeURIComponent(releaseTree)}/latest`
  function check(directory, mesh) {
    const checkEnv = { ...env, IRIS_DRIVE_FIPS_ENABLE_MESH_PUBSUB: String(mesh), RUST_LOG: 'error' }
    delete checkEnv.IRIS_DRIVE_UPDATE_RELAYS
    const output = execFileSync(idrive, ['--config-dir', directory, 'update', '--reference', reference, '--check', '--json'], {
      env: checkEnv, encoding: 'utf8', timeout: 45000, stdio: ['ignore', 'pipe', 'pipe'],
    })
    requirePublishedHead(JSON.parse(output), tag, root)
  }
  const seed = env.IRIS_DRIVE_RELEASE_SEED_CONFIG_DIR
  if (seed) check(seed, false)
  // A running seed reads new announcements from disk at its next sync tick.
  const deadline = Date.now() + 65000
  for (;;) {
    const directory = mkdtempSync(join(tmpdir(), 'iris-drive-release-observer-'))
    try {
      check(directory, true)
      console.log(`Verified signed ${tag} on the Drive mesh`)
      return
    } catch (error) {
      if (Date.now() >= deadline) throw error
    } finally {
      rmSync(directory, { recursive: true, force: true })
    }
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 3000)
  }
}
