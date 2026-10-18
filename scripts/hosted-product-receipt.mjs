import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { stat } from 'node:fs/promises'
import { basename } from 'node:path'
import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

export async function archiveDigest(path) {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return { bytes: (await stat(path)).size, sha256: hash.digest('hex') }
}

export async function verifyProductReceipt(receipt, archive, { commit, tree, platform }) {
  if (receipt.schema !== 1 || !/^[0-9a-f]{40}$/.test(commit)
      || !/^[0-9a-f]{40}$/.test(tree) || receipt.commit !== commit
      || receipt.tree !== tree || receipt.platform !== platform
      || receipt.sourceCleanBeforeBuild !== true || receipt.signed !== false
      || receipt.testsExecuted !== false || receipt.archive !== basename(archive)) {
    throw new Error('Hosted product receipt does not match the requested clean source and platform')
  }
  const actual = await archiveDigest(archive)
  if (actual.bytes !== receipt.bytes || actual.sha256 !== receipt.sha256) {
    throw new Error('Hosted product archive does not match its receipt')
  }
  return actual
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [receiptPath, archive, commit, tree, platform] = process.argv.slice(2)
  await verifyProductReceipt(JSON.parse(readFileSync(receiptPath, 'utf8')), archive,
    { commit, tree, platform })
}
