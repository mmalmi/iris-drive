import { existsSync, readFileSync } from 'node:fs'
import os from 'node:os'
import { join } from 'node:path'

// The release-head provider is the always-on htree daemon that serves Drive's
// signed release head to mesh updaters after relays forget it. Running
// `htree release publish` with its config posts the head to that daemon over
// FIPS (and to its legacy relays) instead of to relays only.
export const providerConfigEnv = 'IRIS_DRIVE_RELEASE_PROVIDER_CONFIG_DIR'

export function providerConfigDir(env, home = os.homedir()) {
  const configured = String(env[providerConfigEnv] ?? '').trim()
  return configured || join(home, 'Library/Application Support/to.iris.drive.release-publisher/config')
}

export function releaseHeadProvider(env, home = os.homedir()) {
  const configDir = providerConfigDir(env, home)
  const configPath = join(configDir, 'config.toml')
  if (!existsSync(configPath)) {
    throw new Error(`Missing release-head provider config ${configPath}; set ${providerConfigEnv}`)
  }
  // htree generates a new identity when the key is missing; never publish as one.
  if (!existsSync(join(configDir, 'keys'))) {
    throw new Error(`Release-head provider config ${configDir} has no release key`)
  }
  const config = readFileSync(configPath, 'utf8')
  if (!/^\s*event_transport\s*=\s*"fips-local-only"\s*$/m.test(config)) {
    throw new Error(`${configPath} must set nostr.event_transport = "fips-local-only"`)
  }
  const bind = config.match(/^\s*bind_address\s*=\s*"([^"]+)"\s*$/m)?.[1]
  if (!bind) throw new Error(`${configPath} has no server.bind_address`)
  return { configDir, baseUrl: `http://${bind}` }
}

// Only the identity and head transport change: `htree release publish` must
// read the release directory from the store `htree add` wrote.
export function providerPublishEnv(env, provider, home = os.homedir()) {
  const dataDir = env.HTREE_DATA_DIR || join(env.HTREE_CONFIG_DIR || join(home, '.hashtree'), 'data')
  return { ...env, HTREE_CONFIG_DIR: provider.configDir, HTREE_DATA_DIR: dataDir }
}

export function providerResolveUrl(provider, npub, releaseTree) {
  return `${provider.baseUrl}/api/nostr/resolve/${npub}/${encodeURIComponent(releaseTree)}`
}

export function requireProviderHead(resolved, root, url) {
  if (!root || resolved?.cid !== root) {
    throw new Error(`Release-head provider ${url} serves ${resolved?.cid || 'no release head'}, expected ${root || 'a published root'}`)
  }
}
