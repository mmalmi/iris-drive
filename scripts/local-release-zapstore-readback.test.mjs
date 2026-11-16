import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { eventId, npubToHex, verifyEvent } from './lib/nostr-verify.mjs'
import { evaluateZapstoreRecords, queryRelay, readbackZapstore, zapstoreFilters } from './zapstore-readback.mjs'

// Signed offline with a throwaway key by nostr-tools; checked here with the
// dependency-free verifier.
const fixture = JSON.parse(readFileSync(new URL('./fixtures/zapstore-readback-events.json', import.meta.url), 'utf8'))
const { pubkey, version, apkSha256, events } = fixture
const copy = value => JSON.parse(JSON.stringify(value))

test('npub decoding, event ids and BIP-340 signatures match an independent signer', () => {
  assert.equal(npubToHex(fixture.npub), pubkey)
  assert.throws(() => npubToHex(fixture.npub.slice(0, -1) + (fixture.npub.endsWith('q') ? 'p' : 'q')), /Not a valid npub/)
  for (const event of events) {
    assert.equal(eventId(event), event.id)
    assert.ok(verifyEvent(event), `kind ${event.kind} verifies`)
  }
  const forged = copy(events[2])
  forged.tags[1][1] = 'f'.repeat(64)
  assert.ok(!verifyEvent(forged), 'changed content no longer matches its id')
  forged.id = eventId(forged)
  assert.ok(!verifyEvent(forged), 'a recomputed id does not match the signature')
  const otherSig = copy(events[0])
  otherSig.sig = events[1].sig
  assert.ok(!verifyEvent(otherSig))
})

test('a release is read back only with app, version and APK records from the publisher', () => {
  const ok = evaluateZapstoreRecords({ events, publisher: pubkey, version, apkSha256 })
  assert.deepEqual(ok, { app: true, release: true, asset: true, invalid: 0, ok: true })
  assert.equal(evaluateZapstoreRecords({ events: events.filter(event => event.kind !== 3063), publisher: pubkey,
    version, apkSha256 }).ok, false)
  assert.equal(evaluateZapstoreRecords({ events, publisher: pubkey, version: '9.9.10', apkSha256 }).release, false)
  assert.equal(evaluateZapstoreRecords({ events, publisher: pubkey, version, apkSha256: '0'.repeat(64) }).asset, false)
  const stranger = evaluateZapstoreRecords({ events, publisher: '1'.repeat(64), version, apkSha256 })
  assert.equal(stranger.ok, false)
  assert.equal(stranger.invalid, events.length)
})

class FakeRelay {
  static script = []
  constructor(url) {
    this.url = url
    this.sent = []
    FakeRelay.last = this
    queueMicrotask(() => this.onopen?.())
  }
  send(text) {
    this.sent.push(JSON.parse(text))
    for (const message of FakeRelay.script) queueMicrotask(() => this.onmessage?.({ data: JSON.stringify(message) }))
  }
  close() { this.closed = true }
}

test('the relay query sends one subscription and stops at end of stored events', async () => {
  FakeRelay.script = [...events.map(event => ['EVENT', 'iris-drive-readback', event]), ['EOSE', 'iris-drive-readback']]
  const filters = zapstoreFilters({ publisher: pubkey, version, apkSha256 })
  const received = await queryRelay({ url: 'wss://relay.test', filters, WebSocketImpl: FakeRelay, timeoutMs: 1000 })
  assert.equal(received.length, events.length)
  assert.deepEqual(FakeRelay.last.sent, [['REQ', 'iris-drive-readback', ...filters]])
  assert.deepEqual(filters[1]['#x'], [apkSha256])
  assert.ok(FakeRelay.last.closed)
  FakeRelay.script = []
  await assert.rejects(queryRelay({ url: 'wss://relay.test', filters, WebSocketImpl: FakeRelay, timeoutMs: 20 }),
    /did not answer within/)
})

test('readback hashes the released APK and uses the publisher from zapstore.yaml', async () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-zapstore-readback-'))
  writeFileSync(join(root, 'zapstore.yaml'), `name: Iris Drive\npubkey: ${fixture.npub}\n`)
  mkdirSync(join(root, 'dist'))
  writeFileSync(join(root, 'dist', 'iris-drive-v9.9.9-android-arm64.apk'), 'fixture apk')
  const queries = []
  const query = async ({ filters }) => { queries.push(filters); return events }
  const lines = []
  const result = await readbackZapstore({ repoRoot: root, tag: 'v9.9.9', query, log: line => lines.push(line) })
  assert.equal(result.ok, true)
  assert.deepEqual(queries[0][0].authors, [pubkey])
  assert.match(lines[0], /Zapstore 9\.9\.9: app true, release true, APK afbe02bf06f5 true/)
  await assert.rejects(readbackZapstore({ repoRoot: root, tag: 'v9.9.8', query }), /Released APK not found/)
})
