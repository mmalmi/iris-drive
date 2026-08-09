import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { deflateRawSync } from 'node:zlib'

import { assertZipEntriesEqualFiles } from './release-zip.mjs'

const apkLicense = 'assets/LICENSE'
const aabLicense = 'base/assets/LICENSE'

function zip(entries) {
  const localParts = []
  const centralParts = []
  let localOffset = 0

  for (const [name, contents, method = 8, localName = name] of entries) {
    const nameBytes = Buffer.from(name)
    const localNameBytes = Buffer.from(localName)
    const bytes = Buffer.from(contents)
    const compressed = method === 0 ? bytes : deflateRawSync(bytes)
    const local = Buffer.alloc(30)
    local.writeUInt32LE(0x04034b50, 0)
    local.writeUInt16LE(20, 4)
    local.writeUInt16LE(method, 8)
    local.writeUInt32LE(compressed.length, 18)
    local.writeUInt32LE(bytes.length, 22)
    local.writeUInt16LE(localNameBytes.length, 26)
    localParts.push(local, localNameBytes, compressed)

    const central = Buffer.alloc(46)
    central.writeUInt32LE(0x02014b50, 0)
    central.writeUInt16LE(20, 4)
    central.writeUInt16LE(20, 6)
    central.writeUInt16LE(method, 10)
    central.writeUInt32LE(compressed.length, 20)
    central.writeUInt32LE(bytes.length, 24)
    central.writeUInt16LE(nameBytes.length, 28)
    central.writeUInt32LE(localOffset, 42)
    centralParts.push(central, nameBytes)
    localOffset += local.length + localNameBytes.length + compressed.length
  }

  const central = Buffer.concat(centralParts)
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(entries.length, 8)
  end.writeUInt16LE(entries.length, 10)
  end.writeUInt32LE(central.length, 12)
  end.writeUInt32LE(localOffset, 16)
  return Buffer.concat([...localParts, central, end])
}

function fixture(t, {
  apkContents = 'license',
  aabContents = 'license',
  apkEntry = apkLicense,
  apkEntries,
  apkLocalName = apkEntry,
  method = 8,
} = {}) {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-android-license-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const licensePath = join(root, 'LICENSE')
  const apkPath = join(root, 'app.apk')
  const aabPath = join(root, 'app.aab')
  writeFileSync(licensePath, 'license')
  writeFileSync(apkPath, zip(apkEntries ?? [[apkEntry, apkContents, method, apkLocalName]]))
  writeFileSync(aabPath, zip([[aabLicense, aabContents, method]]))
  return { root, licensePath, apkPath, aabPath }
}

test('Android acceptance requires exact root license bytes in APK and AAB', (t) => {
  const paths = fixture(t)
  const accept = ({ licensePath, apkPath, aabPath }) =>
    assertZipEntriesEqualFiles([
      [licensePath, apkPath, apkLicense],
      [licensePath, aabPath, aabLicense],
    ], 'Android release license validation failed.')
  assert.doesNotThrow(() => accept(paths))
  assert.doesNotThrow(() => accept(fixture(t, { method: 0 })))

  for (const invalid of [
    fixture(t, { apkEntry: 'assets/OTHER-LICENSE' }),
    fixture(t, { apkLocalName: 'assets/OTHER-LICENSE' }),
    fixture(t, { apkEntries: [[apkLicense, 'license'], [apkLicense, 'license']] }),
    fixture(t, { aabContents: 'different' }),
    { ...paths, licensePath: join(paths.root, 'missing-license') },
  ]) {
    assert.throws(
      () => accept(invalid),
      (error) => {
        assert.equal(error.message, 'Android release license validation failed.')
        assert.equal(error.message.includes(invalid.root), false)
        return true
      },
    )
  }
})

test('Android license acceptance precedes every dist mutation', () => {
  const source = readFileSync(new URL('./local-release.mjs', import.meta.url), 'utf8')
  const block = source.match(/function buildAndroidArtifacts[\s\S]*?\nfunction buildWindowsArtifacts/)?.[0] ?? ''
  const acceptance = block.indexOf('assertZipEntriesEqualFiles')
  const mkdir = block.indexOf('mkdirSync(distDir')
  const apkCopy = block.indexOf('copyFileSync(apkPath')
  const aabCopy = block.indexOf('copyFileSync(aabPath')

  assert.ok(acceptance >= 0, 'Android builder must accept both licenses')
  assert.ok(acceptance < mkdir, 'license acceptance must precede dist creation')
  assert.ok(acceptance < apkCopy, 'license acceptance must precede APK copy')
  assert.ok(acceptance < aabCopy, 'license acceptance must precede AAB copy')
})
