import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'

export const labRevision = '9bb0e9361afa6b2b031c243af21cebbdefa1de4e'

export function validateMeshReceipt(receipt, commit) {
  if (!/^[0-9a-f]{40}$/.test(commit) || receipt?.schema_version !== 1
      || receipt.status !== 'passed' || receipt.lab_revision !== labRevision
      || receipt.release_gate !== true || receipt.lab_worktree_clean !== true) {
    throw new Error('Mesh gate receipt must come from the pinned clean release lab')
  }
  const expected = {
    drive: { source: 'htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-drive', rev: commit },
    chat: { source: 'https://github.com/irislib/iris-chat-rs', rev: 'f3ec6dff066508cb891dfdda7b21b088b1898076' },
    hashtree: { source: 'crates.io', version: '0.3.0' },
  }
  for (const [name, source] of Object.entries(expected)) {
    const product = receipt.products?.[name]
    if (!product || Object.entries(source).some(([key, value]) => product[key] !== value)
        || !/^[0-9a-f]{64}$/.test(product.sha256)) {
      throw new Error(`Mesh gate receipt does not match the ${name} release source`)
    }
  }
  const metrics = receipt.metrics
  if (metrics?.functional_recovery_complete !== true || metrics.wire_budget_passed !== true
      || metrics.anonymous_reader_adjacency !== true || metrics.private_blob_probe?.disclosed !== false) {
    throw new Error('Mesh gate receipt is missing recovery or anonymous reader privacy checks')
  }
  const samples = metrics.idle
  const phases = ['before partition', 'after recovery']
  if (!Array.isArray(samples) || samples.length !== phases.length || samples.some((sample, index) =>
    sample?.phase !== phases[index] || sample.wire_budget_passed !== true
      || sample.cpu_required !== true || sample.cpu_budget_percent !== 5
      || sample.wire_budget_bytes_per_second !== 4096
      || !Number.isFinite(sample.seconds) || sample.seconds < 65
      || !Array.isArray(sample.cpu_percent) || sample.cpu_percent.length !== 4
      || sample.cpu_percent.some(cpu => !Number.isFinite(cpu) || cpu < 0 || cpu > 5)
      || !Number.isFinite(sample.combined_bytes_per_second)
      || sample.combined_bytes_per_second < 0 || sample.combined_bytes_per_second >= 4096)) {
    throw new Error('Mesh gate receipt is missing passing CPU or bandwidth measurements')
  }
}

export function requireMeshRelease({ repoRoot, tag, commit, receiptPath }) {
  const git = (...args) => execFileSync('git', args, {
    cwd: repoRoot, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
  }).trim()
  if (git('rev-parse', 'HEAD') !== commit || git('rev-parse', `refs/tags/${tag}^{commit}`) !== commit) {
    throw new Error('Release tag, HEAD and mesh gate must identify the same commit')
  }
  if (git('cat-file', '-t', `refs/tags/${tag}`) !== 'tag') {
    throw new Error('Final releases require an annotated tag')
  }
  if (git('status', '--porcelain', '--untracked-files=all')) {
    throw new Error('Final release sources must be clean')
  }
  if (!receiptPath) {
    throw new Error('Set IRIS_STACK_GATE_RECEIPT to the pinned Iris Stack gate receipt for this public candidate')
  }
  validateMeshReceipt(JSON.parse(readFileSync(receiptPath, 'utf8')), commit)
}
