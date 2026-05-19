# hashdrive

End-user file sync built on [hashtree](htree://self/hashtree). Think Google Drive,
but content-addressed, multi-transport, and free of DNS/SSL/CDN dependencies.

> Canonical repository: `htree://self/hashdrive`.

## Overview

`hashdrive` is a Rust workspace plus per-platform native shells. The Rust core
wraps the local `htree` daemon and exposes a sync/share model the native UIs
render. Native shells follow the same Rust-core / native-front pattern used in
[nostr-vpn](https://github.com/mmalmi/nostr-vpn).

| Component | Purpose |
| --- | --- |
| `hdrive` | CLI/daemon: sync engine, share controls, htree-daemon supervisor |
| `hashdrive-core` | Shared library: config, sync state, htree client, share model |
| `hashdrive-app-core` | Native app state/action contract + UniFFI bridge for the native shells |
| `macos` | SwiftUI/AppKit native shell over `hashdrive-app-core` |
| `linux` | GTK/libadwaita native shell over the shared app core |
| `windows` | WPF native shell + installer over the shared app core |
| `android` / `ios` | Native mobile shells over the same shared core |

## Status

Project scaffold. No working sync engine yet — the workspace compiles and the
CLI prints `hdrive` help.

## Getting started

```bash
cargo run -p hdrive -- --help
```

## Layout

```
crates/
  hashdrive-core/        shared library
  hashdrive-cli/         `hdrive` CLI + daemon
  hashdrive-app-core/    UniFFI bridge + native app state/actions
macos/ linux/ windows/ android/ ios/   native shell placeholders
docs/                   protocol notes, experiments
scripts/                build / release helpers
```

## License

MIT.
