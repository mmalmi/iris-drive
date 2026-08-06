#!/usr/bin/env node

import { buildNumberFromVersion } from './local-release-lib.mjs'

const version = process.argv[2]
if (!version) {
  console.error('usage: scripts/release-build-number.mjs <version>')
  process.exit(2)
}

try {
  console.log(buildNumberFromVersion(version))
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  process.exit(2)
}
