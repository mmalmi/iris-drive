// Small ustar writer for release test fixtures; no system tar involved.
import { gzipSync } from 'node:zlib'

function header({ name, size, type = '0', mode = 0o755, linkname = '' }) {
  const block = Buffer.alloc(512)
  block.write(name, 0, 100)
  block.write(`${mode.toString(8).padStart(7, '0')}\0`, 100)
  block.write('0000000\0', 108)
  block.write('0000000\0', 116)
  block.write(`${size.toString(8).padStart(11, '0')}\0`, 124)
  block.write('00000000000\0', 136)
  block.write('        ', 148)
  block.write(type, 156)
  block.write(linkname, 157, 100)
  block.write('ustar\0', 257)
  block.write('00', 263)
  let sum = 0
  for (const byte of block) sum += byte
  block.write(`${sum.toString(8).padStart(6, '0')}\0 `, 148)
  return block
}

// entries: [{ name, data?, type?, mode?, linkname? }]; data is a string or Buffer.
export function tarGz(entries) {
  const parts = []
  for (const entry of entries) {
    const data = Buffer.from(entry.data ?? '')
    parts.push(header({ ...entry, size: data.length }), data,
      Buffer.alloc((512 - (data.length % 512)) % 512))
  }
  parts.push(Buffer.alloc(1024))
  return gzipSync(Buffer.concat(parts))
}
