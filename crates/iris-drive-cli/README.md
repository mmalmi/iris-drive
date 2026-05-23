# idrive

CLI and daemon for Iris Drive end-user file sync.

The `idrive` binary supervises a local hashtree daemon, indexes user-chosen
sync roots, and exposes status/share commands the native app shells call.

Run `idrive --help` for the current command set.

Backup targets accept Blossom URLs, FIPS device npubs, `fs:/path`, and
`lmdb:/path`. Filesystem and LMDB targets receive only encrypted hashtree blobs;
root keys and account keys stay in the local Iris Drive config.

## Repository

`htree://self/iris-drive`
