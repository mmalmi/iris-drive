#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

function arMembers(path) {
  const archive = readFileSync(path)
  if (archive.subarray(0, 8).toString('ascii') !== '!<arch>\n') throw new Error('invalid ar')

  const members = new Map()
  let offset = 8
  while (offset < archive.length) {
    if (offset + 60 > archive.length || archive.subarray(offset + 58, offset + 60).toString('ascii') !== '`\n') {
      throw new Error('invalid ar header')
    }
    const rawName = archive.subarray(offset, offset + 16).toString('ascii').trim()
    const name = rawName.endsWith('/') ? rawName.slice(0, -1) : rawName
    const sizeText = archive.subarray(offset + 48, offset + 58).toString('ascii').trim()
    if (!/^[\w.-]+$/.test(name) || !/^\d+$/.test(sizeText) || members.has(name)) {
      throw new Error('invalid ar member')
    }
    const size = Number.parseInt(sizeText, 10)
    const start = offset + 60
    const end = start + size
    if (!Number.isSafeInteger(size) || end > archive.length) throw new Error('invalid ar size')
    members.set(name, archive.subarray(start, end))
    offset = end + (size % 2)
  }
  return members
}

function copyrightFromDataArchive(name, contents) {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-deb-license-'))
  try {
    const archivePath = join(root, name)
    writeFileSync(archivePath, contents)
    const listing = spawnSync('tar', ['-tf', archivePath], {
      encoding: 'utf8',
      maxBuffer: 16 * 1024 * 1024,
    })
    const copyrightName = 'usr/share/doc/iris-drive/copyright'
    const matches = listing.stdout
      .split('\n')
      .filter((entry) => entry.replace(/^\.\//, '') === copyrightName)
    if (listing.status !== 0 || matches.length !== 1) {
      throw new Error('invalid data archive')
    }
    const extracted = spawnSync('tar', ['-xOf', archivePath, matches[0]], {
      maxBuffer: 2 * 1024 * 1024,
    })
    if (extracted.status !== 0) throw new Error('invalid copyright')
    return extracted.stdout
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

export function assertLinuxDebLicense(debPath, licensePath) {
  try {
    const members = arMembers(debPath)
    const controlNames = [...members.keys()].filter((name) => /^control\.tar\.(?:gz|xz|zst|bz2)$/.test(name))
    const dataNames = [...members.keys()].filter((name) => /^data\.tar\.(?:gz|xz|zst|bz2)$/.test(name))
    if (
      members.size !== 3 ||
      !members.get('debian-binary')?.equals(Buffer.from('2.0\n')) ||
      controlNames.length !== 1 ||
      dataNames.length !== 1
    ) {
      throw new Error('invalid deb')
    }

    const license = readFileSync(licensePath)
    const copyright = copyrightFromDataArchive(dataNames[0], members.get(dataNames[0]))
    const text = copyright.toString('utf8')
    const requiredLines = [
      'Source: htree://self/iris-drive',
      'Copyright: 2026 Iris Drive contributors',
      'License: MIT',
    ]
    if (
      license.length === 0 ||
      text.toUpperCase().includes('UNLICENSED') ||
      requiredLines.some((line) => !text.split('\n').includes(line)) ||
      copyright.length < license.length ||
      !copyright.subarray(copyright.length - license.length).equals(license)
    ) {
      throw new Error('invalid license')
    }
  } catch {
    throw new Error('Linux package license validation failed.')
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [debPath, licensePath, ...unexpected] = process.argv.slice(2)
    if (!debPath || !licensePath || unexpected.length > 0) throw new Error('invalid arguments')
    assertLinuxDebLicense(debPath, licensePath)
    console.log('Linux package license validation passed.')
  } catch {
    console.error('Linux package license validation failed.')
    process.exitCode = 1
  }
}
