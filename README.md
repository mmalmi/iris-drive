# Iris Drive

<p align="center">
  <img src="linux/resources/iris-drive.svg" alt="Iris Drive logo" width="112">
</p>

> Canonical repository: `htree://self/iris-drive` · package name: `iris-drive`

Iris Drive is end-user file sync built on local `htree` storage and Nostr
identity. It includes the `idrive` CLI/daemon, a shared Rust core, a UniFFI app
core, and native shells for desktop and mobile platforms. Think Drive-style
sync, but content-addressed, peer-aware, and free of DNS/SSL/CDN dependencies.

OS-visible drives are virtual provider surfaces only: FileProvider, FUSE,
Windows Cloud Files/WinFsp, SAF, or the platform equivalent over htree/provider
roots. Iris Drive should not silently substitute a normal user folder.

## Downloads

Public binary downloads are not a stable channel yet. Build from source for
now:

```bash
just build
just run
```

The `idrive update` path is already wired for signed hashtree releases at:

```text
htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-drive/latest
```

Release staging consumes already-built files from `dist/`; see "Release" under
Maintainer Notes.

## Quick Start

Launch the native app for the current desktop platform:

```bash
just run
```

On macOS this builds and opens the SwiftUI/AppKit app, registers the File
Provider domain when signing allows it, and starts the bundled iris-drive
daemon. On Linux it starts the GTK/libadwaita shell. Use the platform READMEs
for platform-specific signing, packaging, and smoke-test details.

For a terminal-only flow:

```bash
just run-cli init --label "Laptop"
just run-cli import /path/to/seed-folder
just run-cli daemon
```

Useful CLI probes:

```bash
just run-cli stats
just run-cli status
just run-cli whoami
just run-cli list
just run-cli update --check
```

Common device and backup flows:

```bash
just run-cli devices invite
just run-cli devices request <owner-npub-or-invite-url> --label "Laptop"
just run-cli devices approve <device-request-url-or-npub>
just run-cli devices list
just run-cli backups add fs:/path/to/encrypted-backup --label "External disk"
just run-cli backups sync
```

When `idrive daemon` is running it starts a loopback browser gateway on port
`17321` by default. The current primary drive can be opened at:

```text
http://main.drive.iris.localhost:17321/
```

Immutable hashtree roots are served from per-root hosts under
`*.sites.iris.localhost`, and nhash links can be opened through:

```text
http://nhash.iris.localhost:17321/<nhash>/...
```

Toggle the resolver/gateway setting with:

```bash
just run-cli nhash-resolver enable
just run-cli nhash-resolver disable
```

## Native Apps

The native apps share the Rust app-core state/action contract and use platform
shells for macOS, Linux, Windows, Android, and iOS.

```bash
just run
just run-linux
just android-build
just ios-build
```

See the platform READMEs for focused instructions:

- [macOS](macos/README.md)
- [Linux](linux/README.md)
- [Windows](windows/README.md)
- [Android](android/README.md)
- [iOS](ios/README.md)

## What Works Today

- Creates, restores, links, approves, revokes, and lists Nostr-backed Iris Drive
  devices through the CLI and desktop control panels.
- Maintains open Nostr subscriptions for account roster and mutable drive-root
  events while the iris-drive daemon is running.
- Imports local source trees into the persistent htree block store and exposes a
  merged virtual primary drive view through native provider bridges.
- Replicates blocks directly over hashtree-over-[FIPS] between authorized
  devices when peers are reachable; Blossom remains a configured remote/cache
  path.
- Supports encrypted backup targets for Blossom, filesystem, and LMDB endpoints.
- Serves local browser views for `*.iris.localhost` and `nhash.iris.localhost`.
- Provides release-update plumbing through signed hashtree manifests.

## Platform Status

| Platform | Status |
| --- | --- |
| macOS | SwiftUI/AppKit app, menu-bar control, File Provider domain, app-group/runtime wiring, local smoke fixture |
| Linux x64 | GTK/libadwaita app, FUSE-backed provider path, desktop entry/deep links, native smoke coverage |
| Windows x64 | WPF app, tray control, Cloud Files placeholder/hydration path, self-contained publish script |
| Android arm64 | Compose shell plus SAF DocumentsProvider with create/read/write/rename/delete/list support and adb smoke |
| iOS | SwiftUI shell plus File Provider extension, simulator smoke, multidevice harness peer |
| CLI | `idrive` create/restore/link, daemon, provider bridge, FIPS sync, Blossom/cache, backups, updater |

See [Platform GUI parity](docs/PARITY.md) for the detailed cross-platform
matrix and current e2e target.

## Further Reading

- [Design](docs/DESIGN.md): architecture, phases, provider boundaries, and risks
- [Platform GUI parity](docs/PARITY.md): native shell capability matrix and e2e
  targets
- [Snapshot sync implementation plan](docs/SNAPSHOT_SYNC_IMPLEMENTATION_PLAN.md):
  root reconciliation, miss/timeout semantics, and FIPS retrieval plan
- [Apple FileProvider entitlement notes](docs/APPLE_FILEPROVIDER_ENTITLEMENT.md):
  macOS/iOS distribution requirements
- [Experiments](docs/EXPERIMENTS.md): benchmark and integration notes

## Maintainer Notes

This section is intentionally compact and command-oriented. Keep user-facing
product detail above; keep agent/operator reference material here.

### Config Model

`idrive init` creates a fresh owner key and device key. `idrive restore` imports
an owner key onto a new device, and `idrive link` creates a secondary device
that waits for owner/admin approval.

By default, CLI config lives under the OS config directory with the
`iris-drive` suffix. Set `IRIS_DRIVE_CONFIG_DIR` or pass `--config-dir` for
isolated development and tests. Important files live under that directory:

- `config.toml`: app config, relays, drives, roster, backup targets
- `key`: this device's signing key
- `owner_key`: owner signing key, only on create/restore/admin-capable installs
- `blocks/`: Iris Drive htree block store
- `Hashtree/`: embedded hashtree daemon runtime state

Native apps may use app-group or app-owned runtime directories, but they still
pass an explicit config directory to `idrive`/`iris-drive-app-core`.

### Validation

Normal Rust gate:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Repository structure and parity checks:

```bash
just structure
```

Useful focused checks:

```bash
just smoke-macos
just docker-cli-e2e
just android-smoke
just android-gui-smoke
just ios-smoke
just ios-gui-smoke
```

Full configured lab checks:

```bash
just e2e-3vms
just e2e-4devices
just e2e-5devices
```

### Release

1. Bump `[workspace.package].version` in `Cargo.toml`.
2. Run the release gate:

```bash
just release-gate
IRIS_DRIVE_RELEASE_GATE_FULL=1 just release-gate --full
```

The full gate runs the five-platform lab (`just e2e-5devices`) and requires
the Linux, Windows, macOS, iOS, and Android hosts/devices configured in the
local environment.

3. Build platform artifacts into `dist/`:

```bash
node scripts/local-release.mjs --build --only macos
node scripts/local-release.mjs --build --only linux
node scripts/local-release.mjs --build --only windows
node scripts/local-release.mjs --build --only android
node scripts/local-release.mjs --build --only ios
```

Use `--dry-run` to print the planned artifact names and commands without
building. The iOS step runs `scripts/ios-build ios-testflight`; set
`IRIS_DRIVE_IOS_PUBLIC_TESTFLIGHT=1` for the public-TestFlight-capable export.
Before the first iOS upload for a bundle ID, run
`scripts/testflight-internal ensure-app` or create the App Store Connect app
record manually with the documented bundle ID and SKU if the API key cannot
create app records.
`just release` runs the same build entrypoint.

4. Stage the release tree:

```bash
node scripts/local-release.mjs --tag v0.1.0
```

Publish a draft or final signed hashtree release:

```bash
node scripts/local-release.mjs --tag v0.1.0 --publish --draft
node scripts/local-release.mjs --tag v0.1.0 --final
```

The default release tree is `releases/iris-drive`. Copy
`.env.release.example` and `.env.zapstore.example` to local `.env.*.local`
files for machine-specific signing, htree release, and Zapstore settings.

### Workspace Layout

- [`crates/iris-drive-cli`](crates/iris-drive-cli): `idrive` CLI and daemon
- [`crates/iris-drive-core`](crates/iris-drive-core): config, identity, htree,
  sync, gateway, device-link, and backup logic
- [`crates/iris-drive-app-core`](crates/iris-drive-app-core): native app
  state/action contract and UniFFI bridge
- [`crates/hashtree-provider`](crates/hashtree-provider): provider-facing tree
  trait used by virtual file surfaces
- [`crates/iris-drive-mac`](crates/iris-drive-mac): Rust macOS menu-bar dev
  wrapper
- [`macos`](macos), [`linux`](linux), [`windows`](windows), [`android`](android),
  [`ios`](ios): native platform shells
- [`scripts`](scripts): build, smoke, e2e, release, and lab helpers
- [`docs`](docs): design notes, parity matrix, experiments, and platform plans

## License

MIT.

[FIPS]: https://github.com/jmcorgan/fips
