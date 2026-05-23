# Iris Drive

End-user file sync built on [hashtree](htree://self/hashtree). Think Google Drive,
but content-addressed, multi-transport, and free of DNS/SSL/CDN dependencies.

> Canonical repository: `htree://self/iris-drive` · package name: `iris-drive`

## Overview

Iris Drive is a Rust workspace plus per-platform native shells. The Rust core
wraps the local `htree` daemon and exposes a sync/share model the native UIs
render. Native shells follow the same Rust-core / native-front pattern used in
[nostr-vpn](https://github.com/mmalmi/nostr-vpn).

| Component | Purpose |
| --- | --- |
| `idrive` | CLI/daemon: sync engine, share controls, htree-daemon supervisor |
| `iris-drive-core` | Shared library: config, sync state, htree client, share model |
| `iris-drive-app-core` | Native app state/action contract + UniFFI bridge for the native shells |
| `iris-drive-mac` | Rust macOS menu-bar dev wrapper around `idrive daemon` |
| `macos` | SwiftUI/AppKit native shell over `iris-drive-app-core` |
| `linux` | GTK/libadwaita native shell over the shared app core |
| `windows` | WPF native shell over the shared sync engine |
| `android` / `ios` | Native mobile shells over the same shared core |

## Status

Early working sync engine with macOS, Linux, and Windows desktop control
panels. The CLI can initialize an account, import a working directory, publish
private drive roots, replicate blocks directly over FIPS between authorized
devices, fall back to Blossom, mirror encrypted backup blobs to Blossom,
filesystem, or LMDB targets, and run a long-lived daemon.

## Getting started

```bash
just run
```

That launches the Rust macOS menu-bar wrapper. On first launch it creates
`~/Iris Drive`, initializes the local account/device, starts `idrive daemon`,
publishes the private drive root, and uploads encrypted blocks to the default
Blossom server as a fallback/cache.

For a terminal-only daemon, initialize/import once first:

```bash
just run-cli init
mkdir -p "$HOME/Iris Drive"
just run-cli import "$HOME/Iris Drive"
just run-daemon
```

Useful CLI probes:

```bash
just run-cli status
just run-cli whoami
just run-cli list
```

When `idrive daemon` is running it also starts a loopback browser gateway on
port `17321` by default. Stock browsers treat `*.localhost` as a trustworthy
local origin, so the current primary drive can be opened at:

```text
http://main.drive.iris.localhost:17321/
```

Immutable hashtree roots are served from per-root hosts under
`*.sites.iris.localhost`; `idrive status` and `idrive import` print those local
gateway URLs when a root is available.

## Layout

```
crates/
  iris-drive-core/        shared library
  iris-drive-cli/         `idrive` CLI + daemon
  iris-drive-app-core/    UniFFI bridge + native app state/actions
  iris-drive-mac/         Rust macOS menu-bar dev wrapper
macos/ linux/ windows/   desktop native shells
android/ ios/            mobile shell placeholders
docs/                   protocol notes, experiments
```

## License

MIT.
