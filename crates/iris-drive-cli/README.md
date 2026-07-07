# idrive

CLI and daemon for Iris Drive end-user file sync.

The `idrive` binary supervises a local hashtree daemon, indexes user-chosen
sync roots, and exposes status/share commands the native app shells call.

Run `idrive --help` for the current command set.

Common GUI-parity flows:

```bash
idrive stats
idrive status
idrive app-keys invite
idrive app-keys request <device-invite-url> --label "Laptop"
idrive app-keys request <nostr-identity-uuid> --admin-app-key <admin-device-npub> --label "Laptop"
idrive app-keys requests
idrive app-keys approve <device-request-url-or-device-npub>
idrive app-keys reject <device-request-url-or-device-npub>
idrive app-keys revoke <device-npub>
idrive backups add fs:/path/to/encrypted-backup --label "External disk"
idrive backups sync
idrive backups check
idrive update --check
idrive daemon
```

The older top-level linking commands (`link`, `approve`, `revoke`, `roster`)
remain available for scripts; `app-keys ...` is the discoverable operator group
for linked app installs and their scoped device authority.

Backup targets accept Blossom URLs, FIPS device npubs, `fs:/path`, and
`lmdb:/path`. Filesystem and LMDB targets receive only encrypted hashtree blobs;
root keys and NostrIdentity recovery/device material stay in the local Iris Drive config.

## Repository

`htree://self/iris-drive`

## Updates

`idrive update` checks the signed hashtree release reference
`htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-drive/latest`
by default. It uses configured Iris Drive relays and Blossom servers, and when
the iris-drive daemon is already running it tries that daemon's embedded
hashtree endpoint first for cached release blocks.

Release staging starts from already-built files in `dist/`:

```bash
node scripts/local-release.mjs --tag v0.1.0
node scripts/local-release.mjs --tag v0.1.0 --publish --draft
node scripts/local-release.mjs --tag v0.1.0 --final
```
