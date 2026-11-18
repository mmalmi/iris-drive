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

### Back up photos and videos

On iOS or Android, open **Backup** and enable **Back up photos and videos** to copy originals into **My Drive → Photos**. Photo backup starts off. Choose which photos and videos Iris Drive may read in the system permission prompt. The app catches up when opened and can also run when the operating system grants background time. Removing a picture from the device or turning off backup keeps its Drive copy.

On Mac, open **Mirrors → Back up photo folder…** and select a folder of originals. Its contents go into **My Drive → Photos → [folder name]**. For Apple Photos, export originals into a regular folder first. On Windows or Linux, use the same mirror command shown below with a source of your choice and `--to "Photos/Camera"`.

### Mirror a folder into My Drive

On Mac, open **Mirrors → Mirror folder…**, choose a source, then enter a folder name or path inside My Drive. For iCloud Drive sources, use Finder’s **Keep Downloaded** on folders you want available for copying.

```bash
idrive mirror add "/path/to/source" --to "iCloud"
idrive mirror list
idrive mirror run "iCloud"
idrive mirror remove "iCloud"
```

Mirrors check every five minutes while Iris is running. The Mac app handles folders selected in its picker. The daemon handles folders registered from the CLI. Both use the same profile-local registrations and Rust copy engine. Use the same Iris profile for both interfaces (the CLI accepts `--config-dir`). New or changed files are verified against their stored bytes. Source deletions leave the Iris copies in place. An existing destination file with different contents, a symbolic link, or a cloud-only file appears under **need attention**. Resolve it and run the mirror again. Stopping a mirror keeps its contents in My Drive. The CLI supports `--json`; a manual run with pending items prints its report and exits nonzero.

Fresh clients use the independent authenticated FIPS first-adjacency services
on LNVPS (`wss://fips1.iris.to/fips`) and Osiris
(`wss://fips2.iris.to/fips`) by default. Override or disable them with
`IRIS_FIPS_WEBSOCKET_SEED_URLS`, using a comma-separated list of explicit
`wss://.../fips` URLs or an empty value. Normal Nostr relay settings remain
event and discovery/signaling routes; they do not carry FIPS packets.

With mesh pubsub enabled, Drive saves up to 256 signed FIPS peer announcements
in `fips-peer-announcements/` beside the device key. Each device and discovery
scope has its own JSON file. After restart, Drive restores valid announcements
for discovery and can answer a connected peer’s Nostr request from disk even
when the relays and the original announcing peer are offline.

Mesh peer selection uses the shared FIPS reputation policy and local transport
observations. No external rater is trusted by default. Optionally set
`IRIS_DRIVE_FIPS_TRUSTED_RATERS` to comma- or semicolon-separated `npub` or hex
public keys. These identities supply additional initial trust; neither friends
nor successful transport peers automatically gain rating authority. Invalid
explicit keys fail mesh startup. The shared adapter owns bounded rating
exchange and publication; Drive does not run a separate rating loop.

Useful CLI probes:

```bash
just run-cli stats
just run-cli status
just run-cli whoami
just run-cli list
just run-cli update --check
```

Common device-link and backup flows:

```bash
just run-cli app-keys invite
just run-cli app-keys request <device-invite-url> --label "Laptop"
just run-cli app-keys request <nostr-identity-uuid> --admin-app-key <admin-device-npub> --label "Laptop"
just run-cli app-keys approve <device-request-url-or-device-npub>
just run-cli app-keys list
just run-cli app-keys repair-wraps
just run-cli shares create "Projects/Alpha" --name "Alpha"
just run-cli shares link <share-id> --role reader
just run-cli shares accept <drive.iris.to-share-link>
just run-cli shares recipient-evidence --display-name "Alice" > recipient-profile.json
just run-cli shares invite <share-id> --recipient-evidence recipient-profile.json --role reader
just run-cli shares invite <share-id> --profile <nostr-identity-id> --app-key <recipient-device-npub> --role reader
just run-cli shares accept <share-invite-url>
just run-cli shares list
just run-cli shares members <share-id>
just run-cli shares role <share-id> <nostr-identity-id> editor
just run-cli shares revoke <share-id> <nostr-identity-id>
just run-cli shares shortcut <share-id> --parent "Projects"
just run-cli shares shortcut <share-id> --remove
just run-cli shares repair-wraps <share-id>
just run-cli shares list --diagnostics
just run-cli backups add fs:/path/to/encrypted-backup --label "External disk"
just run-cli backups sync
```

Sharing is link-first. **Share** on a shared folder gives a
`https://drive.iris.to/#/share-join/...` link for viewers or editors. Anyone
who opens it can ask to join, in a browser with no install or from the app's
**Join** field. The sharer's Drive lets them in when it sees the request: the
desktop daemon at once, mobile apps on their next sync, the web app while it
is open. As in Google Drive, a shared folder is on disk only where it sits in
My Drive: the owner's own path, or the shortcut a member keeps (added when
they join from the app). There is no "Shared with me" folder on disk; the
Shares list is that view. **Remove from My Drive**, or deleting the shortcut
folder on disk, keeps membership; **Add to My Drive** puts it back; moving the
folder moves the shortcut.
Shared folders belong to the profile, not to one device. Each device sends
its list of them (what it shared or joined, where they sit in My Drive, what
it left) to the profile's other devices, encrypted for them, and they follow
it. A device linked later is let into the folders the profile owns by the
device that shared them. For a folder someone else owns it asks to join with
the link the profile used, so that link has to be still on; a folder joined
from a signed invite instead of a link has to be shared with the new device
again.
`idrive shares link <share-id> --disable` stops a folder's links from letting
anyone else in; people already in keep access. The link format is shared with
Iris Drive web (`src/drive/shareJoinLink.ts`).

Native apps expose the same share operations from the **Shares** tab: create a
shared folder, share it by link, join from a link, change member
roles, revoke a member, add a shortcut, and repair missing key wraps. The UI
shows people/profile members first; invite dialogs prefer signed
recipient-evidence JSON. `idrive shares recipient-evidence` or the app-core
`export_share_recipient_evidence` action exports the local NostrIdentity/device
proof bundle another user can pass to `idrive shares invite
--recipient-evidence` or `invite_share_member_from_evidence`; native share
screens expose this as **Copy my share identity**. Direct device-key entry
remains an admin fallback.

CLI share JSON follows the same shape by default: members are NostrIdentity
entities and repair/revocation output reports counts. Raw device-key lists are
available only from explicit diagnostics output such as
`idrive shares list --diagnostics`,
`idrive shares revoke <share-id> <nostr-identity-id> --diagnostics`, and
`idrive shares repair-wraps <share-id> --diagnostics`.

OS share/open integrations do not grant access by themselves. They open the app
share dialog for a selected Iris Drive folder using
`iris-drive://share?path=<folder>&name=<optional-name>` or the equivalent
`https://drive.iris.to/share?...` route; the user still confirms through the
normal Shares tab create flow. Web/contact handoff may include
`recipient_npub`, `recipient_name`, and `recipient_profile` query hints to
prefill the invite form, but those hints are display/discovery data only. Signed
recipient evidence and roster ops establish the IrisProfile member that receives
access.

Share invites include a signed roster checkpoint that summarizes the current
entity members, the materialized AppKey-to-IrisProfile participant mapping,
compact AppKey/key-epoch and member-roster heads, key epoch, and missing-wrap
state. IrisProfile roster proofs establish which AppKeys belong to a profile;
the share member roster records the IrisProfile UUID-to-role/status grants.
Projected `members` are cache/display state; the signed append-only share logs
remain the source of truth. The checkpoint is an invite-time proof for
recipients and web preview surfaces.

When `idrive daemon` is running it starts a loopback browser gateway on port
`17321` by default. The current primary drive can be opened at:

```text
http://main.drive.iris.localhost:17321/
```

The hand-typed root `http://iris.localhost:17321/` redirects to the canonical
hashtree apps portal at `sites.<publisher-npub>.iris.localhost`.

Immutable hashtree roots are served from per-root hosts under
`*.sites.iris.localhost`, and nhash links can be opened through:

```text
http://nhash.iris.localhost:17321/<nhash>/...
```

Non-HTML files from mutable published htree refs can be opened through the
portal host with any `npub1...` owner. Encode slash-containing tree names as
one path segment:

```text
http://iris.localhost:17321/<npub>/releases%2Fhashtree/latest/install.sh
```

HTML apps should use an isolated origin such as
`<tree>.<npub>.iris.localhost`; the shared portal path refuses `text/html`.

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
  app installs through the CLI and desktop control panels.
- Maintains open Nostr subscriptions for IrisProfile roster ops and mutable
  drive-root events while the iris-drive daemon is running.
- Imports local source trees into the persistent htree block store and exposes a
  merged virtual primary drive view through native provider bridges.
- Retrieves hash-verified blocks through Hashtree's adaptive `BlobRouter`: the
  configured Drive store, the same-user shared LMDB store, and one bounded FIPS
  provider set over reliable TCP streams. Drive keeps its own configured
  outbound links, and Blossom remains a configured remote/cache path. Shared
  access covers immutable blob bytes only; Drive retains ownership of roots,
  pins, garbage collection, and other mutable application metadata. An
  unavailable shared store is reported and omitted from retrieval routes;
  Drive's own store and direct networking remain available.
- Owns and stops its Nostr relay carrier with the embedded FIPS endpoint, so
  authenticated WebRTC signaling works without replacing UDP or other direct
  links. Only roster members enter the Drive data ACL; explicit pending-link
  identities are limited to bootstrap topics, and ambient same-host blob
  providers remain retrieval routes.
- Supports encrypted backup targets for Blossom, filesystem, and LMDB endpoints.
- Supports private friend backups: exchange backup links, add each
  other, and choose total and per-friend space in Backups. These contacts remain
  local and do not create public follows. Backups use identities separate
  from social profiles and other Drive activity. The daemon transfers encrypted
  snapshots over Hashtree/FIPS, checks random blocks, and uses friends as retrieval
  sources. Both apps must be running for a transfer or check.
- Serves local browser views for `*.iris.localhost` and `nhash.iris.localhost`.
- Provides release-update plumbing through signed hashtree manifests.

Friend backup controls are available in the macOS Backups panel and the CLI:

```bash
idrive backups friends identity
idrive backups friends capacity --bytes 107374182400
idrive backups friends add '<friend-backup-link>' --bytes 53687091200 --label 'Friend'
idrive backups friends list
idrive backups friends export-recovery --output backup-recovery.json
idrive backups friends restore --recovery-file backup-recovery.json --friend '<friend-backup-link>'
```

Save the recovery file somewhere safe outside this device. It contains the private
backup key and saved friend contacts to help recover after losing the original
install. You can also restore from a friend added later by supplying their backup
link. The normal profile recovery phrase does not recover that install's backup
key. Recovery creates a new folder in Drive, preserving existing files.
A zero-byte friend allocation requests space
without offering space locally. Retained history and interrupted transfers count
toward the allocation; removing friends or shrinking their allocation never
silently deletes their backups. A successful sample check verifies the sampled
bytes, not the entire backup or physically independent storage.

## Platform Status

| Platform | Status |
| --- | --- |
| macOS | SwiftUI/AppKit app, Shares/AppKeys/Backup control, File Provider domain, app-group/runtime wiring, local smoke fixture |
| Linux x64 | GTK/libadwaita app, Shares/AppKeys/Backup control, FUSE-backed provider path, desktop entry/deep links, native smoke coverage |
| Windows x64 | WPF app, tray control, Cloud Files placeholder/hydration path, self-contained publish script |
| Android arm64 | Compose shell with Shares/AppKeys/Backup plus SAF DocumentsProvider with create/read/write/rename/delete/list and folder share-dialog handoff support |
| iOS | SwiftUI shell with Shares/AppKeys/Backup plus File Provider extension, simulator smoke, multi-app harness peer |
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

`idrive init` creates a NostrIdentity and a fresh per-install device key. Recovery
phrases are explicit recovery material, not part of normal profile creation.
`idrive restore` creates a fresh device key for the restored install, and
`idrive link` creates a fresh device key that waits for admin approval.

By default, CLI config lives under the OS config directory with the
`iris-drive` suffix. Set `IRIS_DRIVE_CONFIG_DIR` or pass `--config-dir` for
isolated development and tests. Important files live under that directory:

- `config.toml`: app config, relays, drives, roster, backup targets
- `key`: this app install's device signing key
- `recovery_phrase`: 12-word NostrIdentity recovery authority, when available
- `blocks/`: Iris Drive htree block store
- `Hashtree/`: embedded hashtree daemon runtime state

Native apps may use app-group or app-owned runtime directories, but they still
pass an explicit config directory to `idrive`/`iris-drive-app-core`.

### Validation

Run the deterministic per-change tier without simulators, phones, VMs, or GUI:

```bash
just verify-fast
```

It includes the normal Rust gate, structure/contracts, release-script tests,
and local core/app-core/CLI contract tests:

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
just verify-health
just verify-full
```

`verify-full` reserves the five-platform lab, records a machine-readable result
under `artifacts/verification/`, and classifies unavailable hosts/devices as
`infrastructure_unavailable` (exit 75) rather than a product failure. It runs
the full GUI/physical-device matrix and is intended for nightly or release
boundaries, not every edit. Dedicated simulator/device reset is opt-in through
`IRIS_NATIVE_LAB_RESET=1`; see `docs/verification-tiers.md`.

### Release

Releases ship binaries built by GitHub Actions from an exact commit. They are
verified, signed and published from a clean checkout of that commit; every step
below refuses another commit, a changed tree or a product whose receipt does
not match.

1. Version: `node scripts/local-release.mjs --tag vX.Y.Z --build --only platform-versions`,
   then `cargo update --workspace --offline` for the root and for
   `--manifest-path linux/Cargo.toml`, retitle "Unreleased" in `CHANGELOG.md`,
   run `just verify-fast`, commit and push.
2. Hosted builds and the Iris Stack gate:

```bash
gh workflow run release-products.yml --ref master -f commit=<sha> -f platform=all
gh workflow run product-lab.yml -R irislib/iris-stack --ref master -f drive_rev=<sha> -f chat_rev=<pin> -f htree_version=<pin>
```

   The products run uploads unsigned `drive-<platform>-<sha>` archives with build
   receipts. It uses checksum-verified tool archives, a pinned Rust toolchain
   and a verified Ubuntu package snapshot. Android verifies Gradle artifacts
   against committed SHA-256 checksums. The App Store archive uses Xcode 27.2
   on the `xcode-27` hosted runner. Archive, export and upload checks require the
   iOS 27 SDK or later. macOS and Windows use the Xcode and Visual Studio tools
   installed on their hosted runners. The stack gate's
   companion pins are in `scripts/mesh-release-gate.mjs`.
   Its `iris-stack-product-gate` artifact holds the `receipt.json` used for
   `IRIS_STACK_GATE_RECEIPT`.
   The products run also opens the published Iris Sites catalog in Chromium
   through the exact packaged Linux CLI's native browser bridge with a fresh
   Drive profile. Its `catalog-check-<sha>` artifact contains the receipt for
   `IRIS_DRIVE_CATALOG_GATE_RECEIPT`. To check an existing hosted Linux product,
   dispatch `platform=catalog-check` with its candidate `commit` and `release_run`
   ID.
3. Stage, sign and upload from the products run:

```bash
node scripts/release-hosted-stage.mjs --platform linux --run <run-id>
node scripts/release-hosted-stage.mjs --platform windows --run <run-id>
node scripts/release-hosted-stage.mjs --platform android --run <run-id>
node scripts/release-hosted-stage.mjs --platform macos-release --run <run-id>
node scripts/release-hosted-stage.mjs --platform ios-release --run <run-id>
IRIS_DRIVE_MACOS_PREBUILT_DIR=<macos-release dir> node scripts/local-release.mjs --build --only macos
node scripts/release-ios-hosted.mjs ios-build ios-export --product <ios-release dir>
node scripts/release-ios-hosted.mjs ios-build ios-upload --product <ios-release dir>
node scripts/release-ios-hosted.mjs testflight-internal put
```

   Linux and Windows files are linked into `dist/` unchanged; the Windows app
   zip must equal the hosted publish directory. Android is signed with the
   upload key from `.env.release.local` and checked for package, version, ABI,
   license, 16 KiB alignment, signer certificate and an unchanged payload.
   Evidence goes to `artifacts/release-<version>/hosted/<sha>/`. The Mac step
   signs, notarizes and runs the launch smoke; when an Iris Drive with the same
   bundle id is running on this Mac, the smoke runs on the lab Mac VM instead
   (`IRIS_DRIVE_MACOS_RELEASE_SMOKE_HOST=local` or `vm` decides explicitly).
   iOS export records the IPA hash and upload refuses any other IPA. Notes come
   from `artifacts/release-<version>/notes/beta-notes.txt` and
   `store-notes.json`. Public TestFlight (`testflight-public put`) and App Store
   review (`app-store submit`) are separate decisions; `app-store submit`
   creates or resumes the exact version's review and stops on missing metadata
   or a different attached build.
4. Gate: `node scripts/release-gate-env.mjs -- just release-gate`. The wrapper
   derives lab hosts from the local dev-lab file and git remotes, selects the
   reserved release-gate simulator and Android emulator, and prints only the
   names of the variables it sets. `just verify-full` also needs the
   five-platform lab with physical phones.
5. Tag and publish:

```bash
node scripts/release-tools.mjs install
node scripts/local-release.mjs --tag vX.Y.Z --stage-dir <dir>
node scripts/release-tools.mjs run -- haps import-release <dir> --config haps-release.json --tag vX.Y.Z --check
IRIS_STACK_GATE_RECEIPT=<stack/receipt.json> IRIS_DRIVE_CATALOG_GATE_RECEIPT=<catalog/receipt.json> node scripts/local-release.mjs --tag vX.Y.Z --stage-dir <dir> --final
```

   Push the annotated tag to both remotes before `--final`. Publishing runs the
   `haps` and `htree` pinned by version and SHA-256 in
   `scripts/release-tools.lock.json`; the archives are verified before every
   use. The Haps key comes from `NOSTR_KEY_PATH` in `.env.zapstore.local`
   unless `HAPS_KEY_FILE` is set. Final publication requires the
   [pinned Iris Stack process-gate receipt](https://github.com/irislib/iris-stack/blob/9bb0e9361afa6b2b031c243af21cebbdefa1de4e/docs/integration-lab.md)
   for the exact annotated tag; it checks the source tuple, recovery, private
   blob protection and CPU/bandwidth budgets. The catalog receipt must match
   the same commit and the SHA-256 of the staged Linux CLI archive. Receipts
   record locally supplied provenance.
6. Read back: `--final` checks the public `release.json`; then run
   `node scripts/zapstore-readback.mjs --tag vX.Y.Z` (signed app, release and
   APK hash records) and `node scripts/release-ios-hosted.mjs testflight-internal status`.

Local builds remain for development: `node scripts/local-release.mjs --build
--only <macos|linux|windows|android|ios>` on the matching host, with `--dry-run`
to print the planned artifacts. Copy `.env.release.example` and
`.env.zapstore.example` to `.env.*.local` for signing and store settings; a
release worktree reads them from the main checkout. The default release tree
is `releases/iris-drive`.

### Workspace Layout

- [`crates/iris-drive-cli`](crates/iris-drive-cli): `idrive` CLI and daemon
- [`crates/iris-drive-core`](crates/iris-drive-core): config, identity, htree,
  sync, gateway, AppKey-link, and backup logic
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


### Haps distribution

[`haps.toml`](haps.toml) describes the `iris-drive` desktop package and the
separately installable `idrive` command-line package. Linux GUI builds require
the GTK 4, libadwaita, FUSE 3 and D-Bus development libraries used by Native CI;
CLI builds use the locked Rust dependencies and the selected host toolchain.
The Windows GUI recipe uses the same self-contained payload as the native
installer. It requires the .NET 8 SDK, Rust MSVC toolchain and Visual Studio C++
Clang component. The self-contained .NET runtime is version-pinned; restore
uses the committed package lock and verifies SDK download archive hashes before
compilation. Publishing cannot perform a second implicit restore. Windows desktop
packages require Haps 0.1.20 or later. Haps installs the Start Menu shortcut and
`iris-drive:` link handler; the app registers its Cloud Files sync root when used. The release
publishes this complete payload as a ZIP alongside the installer, and verifies
its files against the original build output before publication.
The release version step updates both Haps package versions.

The macOS GUI has package metadata and an explicit source-build limitation. Its
FileProvider requires the signing and app-group entitlements in
`macos/Release.entitlements` and `macos/FileProvider/Release.entitlements`.
The existing macOS release builder additionally requires Developer ID,
provisioning and notarization configuration; it is a maintainer workflow.
An ad-hoc GUI build is not a supported substitute. The `idrive` CLI source
recipe remains available on macOS.

Source review applies to the reviewed source and recipe. It does not approve an
unrelated downloaded binary or establish equality with the signed app release.

The final local release flow also publishes desktop/CLI packages using
`haps-release.json` and the same verified `release.json` assets. Drafts do not
publish Haps packages. Install Haps 0.1.22 or later and Python
3.9+ on the publisher. Set `HAPS_KEY_FILE` to an existing secret-key file matching
the public publisher pinned in the mapping (or provision the matching existing
Haps identity). Never generate a new release identity. The flow verifies
checksums and package layout, publishes and cold-fetches the complete source
history, and then uploads the release. Package publication requires Nostr acknowledgement.
A Haps failure fails the final release; byte-identical retries reuse the signed
package versions. Iris Git, native updaters and Haps consume the same release
artifacts; Android/iOS and native installer channels retain their existing gates.
