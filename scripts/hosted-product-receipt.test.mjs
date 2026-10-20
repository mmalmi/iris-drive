import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { archiveDigest, verifyProductReceipt } from './hosted-product-receipt.mjs'

test('hosted products require exact source, platform, and unchanged archive bytes', async () => {
  const root = await mkdtemp(join(tmpdir(), 'drive-product-'))
  try {
    const archive = join(root, 'products.tar.gz')
    await writeFile(archive, 'product bytes')
    const expected = { commit: 'a'.repeat(40), tree: 'b'.repeat(40), platform: 'macos-release' }
    const receipt = { schema: 1, ...expected, sourceCleanBeforeBuild: true, signed: false,
      testsExecuted: false, archive: 'products.tar.gz', ...await archiveDigest(archive) }
    await verifyProductReceipt(receipt, archive, expected)
    for (const change of [{ commit: 'c'.repeat(40) }, { tree: 'd'.repeat(40) },
      { platform: 'ios-release' }, { sourceCleanBeforeBuild: false }, { signed: true },
      { testsExecuted: true }, { archive: '../products.tar.gz' }]) {
      await assert.rejects(verifyProductReceipt({ ...receipt, ...change }, archive, expected))
    }
    await writeFile(archive, 'changed bytes')
    await assert.rejects(verifyProductReceipt(receipt, archive, expected), /archive does not match/)
  } finally { await rm(root, { recursive: true, force: true }) }
})
