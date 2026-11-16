#!/usr/bin/env node
// Read back the Zapstore records for an Android release: the app, the release
// for this version and the APK asset with the staged APK's SHA-256, each
// signed by the publisher named in zapstore.yaml.
import { createHash } from 'node:crypto'
import { existsSync, readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { normalizeTag, readWorkspaceVersionTag } from './local-release-lib.mjs'
import { npubToHex, verifyEvent } from './lib/nostr-verify.mjs'

export const zapstoreRelay = 'wss://relay.zapstore.dev'
export const androidAppId = 'to.iris.drive'

export function zapstoreFilters({ publisher, appId = androidAppId, version, apkSha256 }) {
  const releases = [`${appId}@${version}`, `${appId}@${version}-android-arm64`]
  return [
    { authors: [publisher], '#d': [appId, ...releases], limit: 10 },
    { authors: [publisher], '#x': [apkSha256], limit: 10 },
  ]
}

export function evaluateZapstoreRecords({ events, publisher, appId = androidAppId, version, apkSha256 }) {
  const releases = [`${appId}@${version}`, `${appId}@${version}-android-arm64`]
  const valid = []
  const invalid = []
  for (const event of events) (verifyEvent(event) && event.pubkey === publisher ? valid : invalid).push(event)
  const tag = (event, name, values) => event.tags?.some(entry => entry[0] === name && values.includes(entry[1]))
  const found = {
    app: valid.some(event => tag(event, 'd', [appId])),
    release: valid.some(event => tag(event, 'd', releases)),
    asset: valid.some(event => tag(event, 'x', [apkSha256])),
  }
  return { ...found, invalid: invalid.length, ok: found.app && found.release && found.asset && invalid.length === 0 }
}

// Collects matching events until the relay's end-of-stored-events notice.
export function queryRelay({ url = zapstoreRelay, filters, WebSocketImpl = globalThis.WebSocket, timeoutMs = 25000 }) {
  return new Promise((resolvePromise, reject) => {
    const socket = new WebSocketImpl(url)
    const events = []
    const finish = (error) => {
      clearTimeout(timer)
      try { socket.close() } catch {}
      if (error) reject(error)
      else resolvePromise(events)
    }
    const timer = setTimeout(() => finish(new Error(`${url} did not answer within ${timeoutMs / 1000}s`)), timeoutMs)
    socket.onopen = () => socket.send(JSON.stringify(['REQ', 'iris-drive-readback', ...filters]))
    socket.onerror = () => finish(new Error(`${url} is unavailable`))
    socket.onmessage = ({ data }) => {
      const message = JSON.parse(String(data))
      if (message[0] === 'EVENT' && message[1] === 'iris-drive-readback') events.push(message[2])
      if (message[0] === 'EOSE') finish()
      if (message[0] === 'CLOSED') finish(new Error(`${url} closed the query: ${message[2] ?? ''}`))
    }
  })
}

export async function readbackZapstore({ repoRoot, tag, apkPath, query = queryRelay, log = console.log }) {
  const yaml = readFileSync(join(repoRoot, 'zapstore.yaml'), 'utf8')
  const npub = yaml.match(/^pubkey:\s*(npub1\w+)\s*$/m)?.[1]
  if (!npub) throw new Error('zapstore.yaml names no publisher pubkey')
  const publisher = npubToHex(npub)
  const version = normalizeTag(tag).slice(1)
  const apk = apkPath ?? join(repoRoot, 'dist', `iris-drive-${normalizeTag(tag)}-android-arm64.apk`)
  if (!existsSync(apk)) throw new Error(`Released APK not found: ${apk}`)
  const apkSha256 = createHash('sha256').update(readFileSync(apk)).digest('hex')
  const events = await query({ filters: zapstoreFilters({ publisher, version, apkSha256 }) })
  const result = evaluateZapstoreRecords({ events, publisher, version, apkSha256 })
  log(`Zapstore ${version}: app ${result.app}, release ${result.release}, APK ${apkSha256.slice(0, 12)} ${result.asset}`
    + `${result.invalid ? `, ${result.invalid} record(s) with bad signatures or another publisher` : ''}`)
  return result
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2)
  const option = name => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
  try {
    const tag = option('--tag') ?? readWorkspaceVersionTag(readFileSync(join(repoRoot, 'Cargo.toml'), 'utf8'))
    const apkPath = option('--apk') && resolve(option('--apk'))
    const result = await readbackZapstore({ repoRoot, tag, apkPath })
    process.exitCode = result.ok ? 0 : 1
  } catch (error) {
    console.error(`error: ${error.message}`)
    process.exitCode = 1
  }
}
