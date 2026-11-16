#!/usr/bin/env node
import { spawnSync, execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { join, resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { stageHostedProduct } from './stage-hosted-product.mjs'

const root = resolve(process.env.SOURCE_ROOT)
const inputs = resolve(process.env.PRODUCT_INPUTS)
const scripts = dirname(fileURLToPath(import.meta.url))
const output = resolve(process.env.CHECK_OUTPUT)
mkdirSync(output, { recursive: true })
let env = { ...process.env, CARGO_TARGET_DIR: join(root, 'target') }
function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit', ...options })
  if (result.status !== 0) throw new Error(`${command} failed: ${result.status ?? result.error}`)
}
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
if (commit !== env.SOURCE_COMMIT) throw new Error('Source candidate mismatch')
function stage(name, platform) {
  return stageHostedProduct({ repoRoot: root, productDirectory: join(inputs, name), platform, run, env, destinationParent: output })
}
function derived(name, payload) {
  const path = join(output, name)
  mkdirSync(path)
  renameSync(payload, join(path, 'Build'))
  return path
}
const product = stage('debug', 'ios-sim-debug')
run('python3', [join(scripts, 'diagnose-ios-files-location.py'), product])
