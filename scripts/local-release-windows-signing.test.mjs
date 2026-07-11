import test from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

import { windowsPeHasAuthenticodeSignature } from './local-release-lib.mjs'

function fakeWindowsPe({ signed }) {
  const peOffset = 0x80
  const optionalHeaderOffset = peOffset + 24
  const dataDirectoryOffset = optionalHeaderOffset + 96
  const certificateDirectoryOffset = dataDirectoryOffset + 8 * 4
  const certificateOffset = 0x180
  const certificateSize = 0x20
  const bytes = Buffer.alloc(signed ? certificateOffset + certificateSize : certificateOffset)
  bytes.write('MZ', 0, 'ascii')
  bytes.writeUInt32LE(peOffset, 0x3c)
  bytes.write('PE\0\0', peOffset, 'ascii')
  bytes.writeUInt16LE(0x14c, peOffset + 4)
  bytes.writeUInt16LE(0xe0, peOffset + 20)
  bytes.writeUInt16LE(0x10b, optionalHeaderOffset)
  if (signed) {
    bytes.writeUInt32LE(certificateOffset, certificateDirectoryOffset)
    bytes.writeUInt32LE(certificateSize, certificateDirectoryOffset + 4)
  }
  return bytes
}

test('windowsPeHasAuthenticodeSignature detects the PE certificate table', () => {
  assert.equal(windowsPeHasAuthenticodeSignature(fakeWindowsPe({ signed: true })), true)
  assert.equal(windowsPeHasAuthenticodeSignature(fakeWindowsPe({ signed: false })), false)
  assert.equal(windowsPeHasAuthenticodeSignature(Buffer.from('not a pe')), false)
})

test('local-release final dry-run requires Windows signing inputs for Windows builds', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--final',
      '--dry-run',
      '--skip-zapstore',
      '--tag',
      'v9.9.9',
      '--only',
      'windows',
    ],
    {
      encoding: 'utf8',
      env: {
        PATH: process.env.PATH,
        HOME: process.env.HOME,
      },
    },
  )

  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /Windows Authenticode signing inputs/)
})
