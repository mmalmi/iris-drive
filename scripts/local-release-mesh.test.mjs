import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { labRevision, requireMeshRelease, validateMeshReceipt } from './mesh-release-gate.mjs'

const sha = 'a'.repeat(40)
function receipt(revision = sha) {
  return {
    schema_version: 1, status: 'passed', release_gate: true, lab_revision: labRevision,
    lab_worktree_clean: true,
    products: {
      drive: { source: 'htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-drive', rev: revision, sha256: 'b'.repeat(64) },
      chat: { source: 'https://github.com/irislib/iris-chat-rs', rev: 'dc524968bab6b93d770cc1d7e8e81414293c37ad', sha256: 'b'.repeat(64) },
      hashtree: { source: 'crates.io', version: '0.2.150', sha256: 'b'.repeat(64) },
    },
    metrics: { idle: Array.from({ length: 2 }, () => ({
      seconds: 65, cpu_required: true, cpu_budget_percent: 5, wire_budget_bytes_per_second: 4096,
      cpu_percent: [0, 0.5, 5], combined_bytes_per_second: 3000,
    })) },
  }
}

test('mesh receipt binds candidate, lab, companions and measured budgets', () => {
  validateMeshReceipt(receipt(), sha)
  const mutations = [
    value => { value.lab_revision = sha },
    value => { value.release_gate = false },
    value => { value.lab_worktree_clean = false },
    value => { value.products.drive.rev = 'c'.repeat(40) },
    value => { value.products.chat.rev = 'c'.repeat(40) },
    value => { value.products.hashtree.version = '0.2.145' },
    value => { value.products.drive.source = 'local-binary' },
    value => { value.products.drive.sha256 = '' },
    value => { value.metrics.idle[0].cpu_budget_percent = 100 },
    value => { delete value.metrics.idle[0].seconds },
    value => { value.metrics.idle[0].seconds = 64.99 },
    value => { value.metrics.idle[1].seconds = 15 },
    value => { value.metrics.idle[0].seconds = Number.POSITIVE_INFINITY },
    value => { value.metrics.idle[0].seconds = '65' },
    value => { value.metrics.idle[0].cpu_percent[1] = null },
    value => { value.metrics.idle[0].cpu_percent[1] = true },
    value => { value.metrics.idle[0].cpu_percent[1] = 5.1 },
    value => { value.metrics.idle[0].combined_bytes_per_second = 4096 },
  ]
  for (const mutate of mutations) {
    const value = receipt()
    mutate(value)
    assert.throws(() => validateMeshReceipt(value, sha))
  }
  for (const value of [null, {}, []]) assert.throws(() => validateMeshReceipt(value, sha))
})

test('final release requires an annotated exact tag and unchanged sources', t => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-mesh-gate-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
  git('init', '-q')
  writeFileSync(join(root, 'source'), 'original')
  git('add', 'source')
  git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'Source')
  const commit = git('rev-parse', 'HEAD')
  git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'tag', '-a', 'v1.0.0', '-m', 'Release')
  git('tag', 'lightweight')
  const receiptPath = join(root, '.git', 'receipt.json')
  writeFileSync(receiptPath, JSON.stringify(receipt(commit)))
  const options = { repoRoot: root, tag: 'v1.0.0', commit, receiptPath }
  requireMeshRelease(options)
  assert.throws(() => requireMeshRelease({ ...options, tag: 'lightweight' }), /annotated/)
  assert.throws(() => requireMeshRelease({ ...options, commit: sha }), /commit/)
  assert.throws(() => requireMeshRelease({ ...options, receiptPath: undefined }), /IRIS_STACK_GATE_RECEIPT/)
  writeFileSync(receiptPath, 'broken JSON')
  assert.throws(() => requireMeshRelease(options))
  writeFileSync(receiptPath, JSON.stringify(receipt(commit)))
  writeFileSync(join(root, 'untracked'), 'unexpected')
  assert.throws(() => requireMeshRelease(options), /clean/)
  rmSync(join(root, 'untracked'))
  writeFileSync(join(root, 'source'), 'changed')
  assert.throws(() => requireMeshRelease(options), /clean/)
})

test('supported final publisher enforces the receipt before build and publication', () => {
  const source = readFileSync(new URL('./local-release.mjs', import.meta.url), 'utf8')
  const main = source.slice(source.indexOf('function main()'))
  assert.equal(main.match(/requireMeshRelease\(meshGate\)/g)?.length, 2)
  assert(main.indexOf('requireMeshRelease(meshGate)') < main.indexOf('buildReleaseArtifacts('))
  assert(main.lastIndexOf('requireMeshRelease(meshGate)') < main.indexOf('const published = publishRelease('))
})
