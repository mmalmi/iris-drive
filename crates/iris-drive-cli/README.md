# idrive

CLI and daemon for Iris Drive end-user file sync.

The `idrive` binary supervises a local hashtree daemon, indexes user-chosen
sync roots, and exposes status/share commands the native app shells call.

Run `idrive --help` for the current command set.

Common GUI-parity flows:

```bash
idrive stats
idrive status
idrive devices invite
idrive devices request <owner-npub-or-invite-url> --label "Laptop"
idrive devices requests
idrive devices approve <device-request-url-or-npub>
idrive devices revoke <device-npub>
idrive backups add fs:/path/to/encrypted-backup --label "External disk"
idrive backups sync
idrive backups check
idrive daemon
```

The older top-level device commands (`link`, `approve`, `revoke`, `roster`)
remain available for scripts; `devices ...` is the discoverable operator group
that mirrors the native desktop control panels.

Backup targets accept Blossom URLs, FIPS device npubs, `fs:/path`, and
`lmdb:/path`. Filesystem and LMDB targets receive only encrypted hashtree blobs;
root keys and account keys stay in the local Iris Drive config.

## Repository

`htree://self/iris-drive`
