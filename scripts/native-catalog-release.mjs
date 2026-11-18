import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { basename } from 'node:path'
import { archiveDigest } from './hosted-product-receipt.mjs'

export const irisSitesSource = 'htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-sites'
export const irisSitesCommit = '60d045154af66810587a424f16ab2e2112209f92'

export function requirePassingCatalog(results) {
  assert.ok(Number.isSafeInteger(results?.passed) && results.passed > 0
    && results.failed === 0 && results.skipped === 2,
  'The native catalog must pass with only the two external services skipped')
}

export function readNativeCatalogReceipt(path, commit) {
  assert.ok(path, 'Set IRIS_DRIVE_CATALOG_GATE_RECEIPT to the successful hosted catalog receipt')
  const receipt = JSON.parse(readFileSync(path, 'utf8'))
  assert.ok(receipt?.schema === 1 && receipt.status === 'passed'
    && /^[a-f0-9]{40}$/.test(commit) && receipt.commit === commit
    && /^[a-f0-9]{64}$/.test(receipt.linuxCliSha256)
    && receipt.catalogSource === irisSitesSource && receipt.catalogCommit === irisSitesCommit,
  'The native catalog receipt must match the exact Drive candidate and pinned Iris Sites source')
  requirePassingCatalog(receipt.results)
  return receipt
}

export async function verifyNativeCatalogArchive(receipt, archive) {
  const actual = await archiveDigest(archive)
  assert.ok(receipt.linuxCliArchive?.name === basename(archive)
    && receipt.linuxCliArchive.bytes === actual.bytes && receipt.linuxCliArchive.sha256 === actual.sha256,
  'The staged Linux CLI archive does not match the candidate tested through the native catalog')
}
