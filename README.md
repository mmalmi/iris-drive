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
| `macos` | SwiftUI/AppKit native shell over `iris-drive-app-core` |
| `linux` | GTK/libadwaita native shell over the shared app core |
| `windows` | WPF native shell + installer over the shared app core |
| `android` / `ios` | Native mobile shells over the same shared core |

## Status

Project scaffold. No working sync engine yet — the workspace compiles and the
CLI prints `idrive` help.

## Getting started

```bash
cargo run -p idrive -- --help
```

## Layout

```
crates/
  iris-drive-core/        shared library
  iris-drive-cli/         `idrive` CLI + daemon
  iris-drive-app-core/    UniFFI bridge + native app state/actions
macos/ linux/ windows/ android/ ios/   native shell placeholders
docs/                   protocol notes, experiments
scripts/                build / release helpers
```

## License

MIT.
