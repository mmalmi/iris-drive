import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'
import test from 'node:test'
import { archiveDigest } from './hosted-product-receipt.mjs'
import { irisSitesCommit, irisSitesSource, readNativeCatalogReceipt, verifyNativeCatalogArchive } from './native-catalog-release.mjs'

test('native catalog receipt binds the real run to the candidate, gate source and staged CLI archive', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'drive-native-catalog-'))
  t.after(() => rm(dir, { recursive: true, force: true }))
  const commit = 'a'.repeat(40)
  const path = join(dir, 'receipt.json')
  const archive = join(dir, 'idrive-v1.0.0-x86_64-unknown-linux-gnu.tar.gz')
  await writeFile(archive, 'checked candidate')
  const receipt = {
    schema: 1, status: 'passed', commit, catalogSource: irisSitesSource, catalogCommit: irisSitesCommit,
    linuxCliArchive: { name: basename(archive), ...await archiveDigest(archive) },
    linuxCliSha256: 'b'.repeat(64), results: { passed: 18, failed: 0, skipped: 2 },
  }
  await writeFile(path, JSON.stringify(receipt))
  await verifyNativeCatalogArchive(readNativeCatalogReceipt(path, commit), archive)
  assert.throws(() => readNativeCatalogReceipt(undefined, commit), /IRIS_DRIVE_CATALOG_GATE_RECEIPT/)
  assert.throws(() => readNativeCatalogReceipt(path, 'c'.repeat(40)), /exact Drive candidate/)
  await writeFile(path, JSON.stringify({ ...receipt, catalogCommit: 'c'.repeat(40) }))
  assert.throws(() => readNativeCatalogReceipt(path, commit), /pinned Iris Sites source/)
  for (const results of [{ passed: 0, failed: 0, skipped: 2 }, { passed: 18, failed: 1, skipped: 2 },
    { passed: 18, failed: 0, skipped: 3 }]) {
    await writeFile(path, JSON.stringify({ ...receipt, results }))
    assert.throws(() => readNativeCatalogReceipt(path, commit), /native catalog must pass/)
  }
  await writeFile(archive, 'changed candidate')
  await assert.rejects(verifyNativeCatalogArchive(receipt, archive), /archive does not match/)
})

test('the final publisher checks the staged catalog-tested archive before Haps or Hashtree publication', async () => {
  const source = await readFile(new URL('./local-release.mjs', import.meta.url), 'utf8')
  const main = source.slice(source.indexOf('async function main()'))
  const verify = main.indexOf('await verifyNativeCatalogArchive(')
  assert(verify > main.indexOf('stageRelease({'))
  assert(verify < main.indexOf("run('haps', [...hapsArgs, '--publish-source']"))
  assert(verify < main.indexOf('const published = publishRelease('))
})
