import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

// Product archives remain immutable; signing always uses a fresh extraction.
export function stageHostedProduct({ repoRoot, productDirectory, platform, run, env, destinationParent }) {
  const git = (...args) => execFileSync('git', args, { cwd: repoRoot, encoding: 'utf8' }).trim()
  if (git('status', '--porcelain', '--untracked-files=all')) {
    throw new Error('Prepared products require a clean source checkout')
  }
  const archive = join(productDirectory, `${platform}-products.tar.gz`)
  run('node', [fileURLToPath(new URL('./hosted-product-receipt.mjs', import.meta.url)),
    join(productDirectory, 'build-receipt.json'), archive, git('rev-parse', 'HEAD'),
    git('rev-parse', 'HEAD^{tree}'), platform], { env })
  const work = destinationParent ?? join(repoRoot, 'work')
  mkdirSync(work, { recursive: true })
  const destination = mkdtempSync(join(work, `prepared-${platform}-`))
  run('python3', [fileURLToPath(new URL('./unpack-hosted-product.py', import.meta.url)),
    archive, destination], { env })
  return destination
}
