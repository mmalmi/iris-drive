// Read-only Nostr checks for release readback: npub decoding (BIP-173),
// event ids (NIP-01) and BIP-340 Schnorr signatures, without dependencies.
import { createHash } from 'node:crypto'

const sha256 = (...parts) => createHash('sha256').update(Buffer.concat(parts.map(part => Buffer.from(part)))).digest()
const P = 0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2fn
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n
const G = {
  x: 0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798n,
  y: 0x483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8n,
}
const mod = (value, modulus = P) => ((value % modulus) + modulus) % modulus

function pow(base, exponent) {
  let result = 1n
  for (let b = mod(base); exponent > 0n; exponent >>= 1n, b = (b * b) % P) {
    if (exponent & 1n) result = (result * b) % P
  }
  return result
}

function add(a, b) {
  if (!a) return b
  if (!b) return a
  if (a.x === b.x && mod(a.y + b.y) === 0n) return null
  const slope = a.x === b.x
    ? mod(3n * a.x * a.x * pow(2n * a.y, P - 2n))
    : mod((b.y - a.y) * pow(b.x - a.x, P - 2n))
  const x = mod(slope * slope - a.x - b.x)
  return { x, y: mod(slope * (a.x - x) - a.y) }
}

function multiply(point, scalar) {
  let result = null
  for (let addend = point; scalar > 0n; scalar >>= 1n, addend = add(addend, addend)) {
    if (scalar & 1n) result = add(result, addend)
  }
  return result
}

function liftX(x) {
  if (x >= P) return null
  const c = mod(x * x * x + 7n)
  const y = pow(c, (P + 1n) / 4n)
  if (mod(y * y) !== c) return null
  return { x, y: y % 2n === 0n ? y : P - y }
}

const toBig = bytes => BigInt(`0x${Buffer.from(bytes).toString('hex') || '0'}`)
const isHex = (value, bytes) => typeof value === 'string' && new RegExp(`^[0-9a-f]{${bytes * 2}}$`).test(value)

export function schnorrVerify(signatureHex, messageHex, publicKeyHex) {
  if (!isHex(signatureHex, 64) || !isHex(messageHex, 32) || !isHex(publicKeyHex, 32)) return false
  const signature = Buffer.from(signatureHex, 'hex')
  const publicKey = Buffer.from(publicKeyHex, 'hex')
  const point = liftX(toBig(publicKey))
  const r = toBig(signature.subarray(0, 32))
  const s = toBig(signature.subarray(32))
  if (!point || r >= P || s >= N) return false
  const tag = sha256('BIP0340/challenge')
  const e = mod(toBig(sha256(tag, tag, signature.subarray(0, 32), publicKey, Buffer.from(messageHex, 'hex'))), N)
  const R = add(multiply(G, s), multiply(point, mod(N - e, N)))
  return Boolean(R) && R.y % 2n === 0n && R.x === r
}

export function eventId(event) {
  return sha256(JSON.stringify([0, event.pubkey, event.created_at, event.kind, event.tags, event.content])).toString('hex')
}

export function verifyEvent(event) {
  return Boolean(event) && isHex(event.id, 32) && eventId(event) === event.id
    && schnorrVerify(event.sig, event.id, event.pubkey)
}

const CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l'
function polymod(values) {
  const generators = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
  let checksum = 1
  for (const value of values) {
    const top = checksum >> 25
    checksum = ((checksum & 0x1ffffff) << 5) ^ value
    for (let index = 0; index < 5; index += 1) if ((top >> index) & 1) checksum ^= generators[index]
  }
  return checksum
}

export function npubToHex(npub) {
  const text = String(npub).trim().toLowerCase()
  const separator = text.lastIndexOf('1')
  const prefix = text.slice(0, separator)
  const data = [...text.slice(separator + 1)].map(character => CHARSET.indexOf(character))
  const expanded = [...prefix].map(c => c.charCodeAt(0) >> 5).concat(0, [...prefix].map(c => c.charCodeAt(0) & 31))
  if (prefix !== 'npub' || data.length < 6 || data.includes(-1) || polymod([...expanded, ...data]) !== 1) {
    throw new Error('Not a valid npub')
  }
  const bytes = []
  let accumulator = 0
  let bits = 0
  for (const value of data.slice(0, -6)) {
    accumulator = ((accumulator << 5) | value) & 0xfff
    bits += 5
    while (bits >= 8) {
      bits -= 8
      bytes.push((accumulator >> bits) & 0xff)
    }
  }
  if (bytes.length !== 32) throw new Error('Not a valid npub')
  return Buffer.from(bytes).toString('hex')
}
