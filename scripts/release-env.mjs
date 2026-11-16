// Local release settings for scripts that run from a linked release worktree.
// The ignored .env.*.local files live in the main checkout, so read them from
// there as well as from the release checkout itself.
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import { parseEnvFile } from './local-release-lib.mjs'

export function mainCheckout(repoRoot) {
  const root = resolve(repoRoot)
  const marker = join(root, '.git')
  if (!statSync(marker, { throwIfNoEntry: false })?.isFile()) return root
  // A linked worktree points to its private Git directory. Its commondir is
  // relative to that directory, even when the worktree itself is elsewhere.
  const pointer = /^gitdir: (.+)$/.exec(readFileSync(marker, 'utf8').trim())
  if (!pointer) throw new Error('Invalid Git worktree pointer')
  const gitDir = resolve(root, pointer[1])
  const commonFile = join(gitDir, 'commondir')
  const common = existsSync(commonFile) ? resolve(gitDir, readFileSync(commonFile, 'utf8').trim()) : gitDir
  return basename(common) === '.git' ? dirname(common) : root
}

export function loadReleaseEnv({
  repoRoot, localFileNames = ['.env.release.local', '.env.zapstore.local'],
  provisioningFile, envFiles = [], env = process.env,
}) {
  const checkout = resolve(repoRoot)
  const primary = mainCheckout(checkout)
  const localFiles = root => localFileNames.map(name => join(root, name))
  const files = [
    ...(primary !== checkout ? localFiles(primary) : []),
    ...(provisioningFile ? [provisioningFile] : []),
    ...localFiles(checkout),
    ...envFiles,
  ]
  const loaded = {}
  for (const file of files) {
    if (existsSync(file)) Object.assign(loaded, parseEnvFile(readFileSync(file, 'utf8')))
  }
  return { ...loaded, ...env }
}

// Runs a tool and returns its stdout; stderr stays visible, failures throw.
export function runTool(command, args, { env = process.env, cwd, encoding = 'utf8' } = {}) {
  const result = spawnSync(command, args, {
    cwd, env, encoding, maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'inherit'],
  })
  if (result.status !== 0) {
    throw new Error(`${basename(command)} exited ${result.status ?? result.error?.code ?? result.signal}`)
  }
  return result.stdout
}

export function gitHubRepository(repoRoot, remote = 'github') {
  const result = spawnSync('git', ['remote', 'get-url', remote], { cwd: repoRoot, encoding: 'utf8' })
  const url = result.status === 0 ? result.stdout.trim() : ''
  return url.match(/github\.com[:/]([\w.-]+\/[\w.-]+?)(?:\.git)?$/)?.[1] ?? ''
}
