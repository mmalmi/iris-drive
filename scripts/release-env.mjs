// Local release settings for scripts that run from a linked release worktree.
// The ignored .env.*.local files live in the main checkout, so read them from
// there as well as from the release checkout itself.
import { execFileSync, spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import { parseEnvFile } from './local-release-lib.mjs'

export function mainCheckout(repoRoot) {
  const common = execFileSync('git', ['rev-parse', '--path-format=absolute', '--git-common-dir'], {
    cwd: repoRoot, encoding: 'utf8',
  }).trim()
  return basename(common) === '.git' ? dirname(common) : resolve(repoRoot)
}

export function loadReleaseEnv({ repoRoot, envFiles = [], env = process.env }) {
  const roots = [...new Set([mainCheckout(repoRoot), resolve(repoRoot)])]
  const files = [
    ...roots.flatMap(root => ['.env.release.local', '.env.zapstore.local'].map(name => join(root, name))),
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
