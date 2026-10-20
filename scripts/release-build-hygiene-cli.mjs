#!/usr/bin/env node

import { lstatSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

import { assertNoPrivateBuildMetadata } from './release-build-hygiene.mjs'

function regularFiles(path) {
  const metadata = lstatSync(path)
  if (metadata.isSymbolicLink()) {
    throw new Error('Release payload must not contain symbolic links.')
  }
  if (metadata.isFile()) {
    return [path]
  }
  if (!metadata.isDirectory()) {
    return []
  }
  return readdirSync(path).flatMap((entry) => regularFiles(join(path, entry)))
}

try {
  const [repoRoot, payloadRoot, ...unexpected] = process.argv.slice(2)
  if (!repoRoot || !payloadRoot || unexpected.length > 0) {
    throw new Error('Invalid release payload privacy audit arguments.')
  }
  assertNoPrivateBuildMetadata(regularFiles(payloadRoot), process.env, { repoRoot })
  console.log('Release payload privacy audit passed.')
} catch {
  console.error('Release payload privacy audit failed.')
  process.exitCode = 1
}
