import { readFileSync } from 'node:fs'
import { inflateRawSync } from 'node:zlib'

function readEntry(archive, expectedName, expectedSize) {
  const earliestEnd = Math.max(0, archive.length - 65_557)
  let endOffset = -1
  for (let offset = archive.length - 22; offset >= earliestEnd; offset -= 1) {
    if (
      archive.readUInt32LE(offset) === 0x06054b50 &&
      offset + 22 + archive.readUInt16LE(offset + 20) === archive.length
    ) {
      endOffset = offset
      break
    }
  }
  if (endOffset < 0) throw new Error('invalid zip')

  const entryCount = archive.readUInt16LE(endOffset + 10)
  const centralSize = archive.readUInt32LE(endOffset + 12)
  const centralOffset = archive.readUInt32LE(endOffset + 16)
  if (centralOffset + centralSize !== endOffset) throw new Error('invalid zip')

  let found = null
  let offset = centralOffset
  for (let index = 0; index < entryCount; index += 1) {
    if (offset + 46 > archive.length || archive.readUInt32LE(offset) !== 0x02014b50) {
      throw new Error('invalid zip')
    }
    const flags = archive.readUInt16LE(offset + 8)
    const method = archive.readUInt16LE(offset + 10)
    const compressedSize = archive.readUInt32LE(offset + 20)
    const size = archive.readUInt32LE(offset + 24)
    const nameLength = archive.readUInt16LE(offset + 28)
    const extraLength = archive.readUInt16LE(offset + 30)
    const commentLength = archive.readUInt16LE(offset + 32)
    const localOffset = archive.readUInt32LE(offset + 42)
    const nameStart = offset + 46
    const nextOffset = nameStart + nameLength + extraLength + commentLength
    if (nextOffset > archive.length) throw new Error('invalid zip')

    const name = archive.subarray(nameStart, nameStart + nameLength).toString('utf8')
    if (name === expectedName) {
      if (found) throw new Error('duplicate zip entry')
      if ((flags & 1) !== 0 || size !== expectedSize || ![0, 8].includes(method)) {
        throw new Error('invalid zip entry')
      }
      if (localOffset + 30 > archive.length || archive.readUInt32LE(localOffset) !== 0x04034b50) {
        throw new Error('invalid zip entry')
      }
      const localFlags = archive.readUInt16LE(localOffset + 6)
      const localMethod = archive.readUInt16LE(localOffset + 8)
      const localNameLength = archive.readUInt16LE(localOffset + 26)
      const localExtraLength = archive.readUInt16LE(localOffset + 28)
      const localNameStart = localOffset + 30
      const dataStart = localOffset + 30 + localNameLength + localExtraLength
      const dataEnd = dataStart + compressedSize
      const localName = archive.subarray(localNameStart, localNameStart + localNameLength)
      if (
        localFlags !== flags ||
        localMethod !== method ||
        localNameStart + localNameLength > archive.length ||
        !localName.equals(archive.subarray(nameStart, nameStart + nameLength)) ||
        dataEnd > centralOffset
      ) {
        throw new Error('invalid zip entry')
      }
      const compressed = archive.subarray(dataStart, dataEnd)
      const contents = method === 0
        ? compressed
        : inflateRawSync(compressed, { maxOutputLength: Math.max(expectedSize, 1) })
      if (contents.length !== size) throw new Error('invalid zip entry')
      found = contents
    }
    offset = nextOffset
  }
  if (offset !== endOffset) throw new Error('invalid zip')
  if (!found) throw new Error('missing zip entry')
  return found
}

export function assertZipEntriesEqualFiles(entries, failureMessage) {
  try {
    for (const [expectedPath, archivePath, entry] of entries) {
      const expected = readFileSync(expectedPath)
      const actual = readEntry(readFileSync(archivePath), entry, expected.length)
      if (!actual.equals(expected)) throw new Error('entry mismatch')
    }
  } catch {
    throw new Error(failureMessage || 'Release ZIP entry validation failed.')
  }
}
