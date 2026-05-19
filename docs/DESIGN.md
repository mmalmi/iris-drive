# Hashdrive design

End-user file sync with Google-Drive-/Dropbox-style UX, built on
content-addressed storage (hashtree) with a Nostr-based identity and
discovery layer. P2P as far as each OS allows.

## Goals

- One **"My Drive"** per user — a hashtree root they own and edit, presented as
  a folder in the OS file manager.
- Sync that root across all of the user's devices automatically.
- Each device seeds its blocks P2P to other peers (initially WebRTC; transport
  layer eventually delegated to `~/src/fips`, see below).
- **Share** folders with specific other Nostr pubkeys; shares appear as
  additional drives on the recipient's device.
- No DNS, SSL, CDNs, or centralized servers. Identity = Nostr keypair.
- Native shells per platform, single shared Rust core.

## Architecture

Single in-process hashtree daemon (`hashtree-embedded`) inside the hashdrive
app holds the blocks, maintains the index, publishes the user's mutable root
over Nostr, and seeds those blocks to peers. The user's "drive" is **one
hashtree root they own**, served to the OS through whichever presentation
backend the platform supports (FileProvider / WinFsp / FUSE / DocumentsProvider).

Multiple devices owned by the same user reconcile their drive root via Nostr;
shares are extra roots from other pubkeys mounted as additional drives.

No separate daemon process, no IPC — `hashdrive-app-core` links
`hashtree-embedded` as a library and the app extensions / shells link
`hashdrive-app-core` via UniFFI.

```
┌─────────────────────────────────────────────────────────────┐
│  OS file surface (Finder / Explorer / Files app)            │
└──────┬───────────────┬─────────────────┬────────────────────┘
       │ FileProvider  │ WinFsp / FUSE   │ DocumentsProvider
       ▼               ▼                 ▼
┌────────────────────────────────────────────────────────────┐
│  Native shell (Swift / WPF / GTK / Compose)                 │
│  ⇄ hashdrive-app-core (UniFFI)                              │
└──────────────────────────┬─────────────────────────────────┘
                           ▼
┌────────────────────────────────────────────────────────────┐
│  hashdrive-core                                             │
│   - drive model, indexer, sync engine, share manager        │
│   - reconciler + conflict resolution                        │
└──────────────────────────┬─────────────────────────────────┘
                           ▼
┌────────────────────────────────────────────────────────────┐
│  hashtree-embedded                                          │
│   - block store, root publish, mutable-root subscriptions   │
│   - peer seeding (WebRTC today; fips later)                 │
└────────────────────────────────────────────────────────────┘
```

## External project relationships

- **`~/src/hashtree`** — supplies the storage primitive and all OS-mount
  adapters (see "Where adapter crates live" below). hashdrive does not fork
  hashtree; it consumes it and contributes upstream.
- **`~/src/fips`** — peer-to-peer transport. WebRTC and other transports will
  be migrated to fips's concern. Hashdrive should consume the transport
  layer through hashtree, not pin a specific transport. Treat any direct
  WebRTC reference as a temporary measure pending the fips transition.
- **`~/src/squirreldisk`** — disk-usage pie chart analyzer. Reference only for
  the "what's using space" UI idea in Phase 7; not a code dependency. Look at
  it for visualization inspiration, no obligation to extract or reuse.

## Where adapter crates live

Rust adapters live in **hashtree**. App-specific shells (with bundle IDs,
installer wiring, sidebar labels) live in **hashdrive**.

```
hashtree/rust/crates/
├── hashtree-fuse              Linux + (legacy) macOS-FUSE
├── hashtree-winfsp            Windows (new)
├── hashtree-fileprovider      macOS + iOS shared Rust core (new)
└── hashtree-saf               Android DocumentsProvider Rust core (new)

hashdrive/
├── linux/                            systemd user unit, mount at ~/Hashdrive
├── windows/                          installer bundles WinFsp
├── macos/HashdriveFileProvider/      Swift extension target
├── ios/HashdriveFileProvider/        Swift extension (shares code w/ macos)
└── android/.../HashdriveDocumentsProvider.kt
```

## Phases

Each phase deliverable validates the architecture before the next layer lands.
Weeks are rough.

### Phase 0 — Upstream prep in hashtree (weeks 1–2)

- **Async `ProviderFs` trait** in a new `hashtree-provider` crate. Lift the
  directory/file semantics out of `hashtree-fuse` so the trait is the source
  of truth and `hashtree-fuse` is one consumer.
- **Root-diff API**: `diff(old_root, new_root) -> [ItemChange]` + monotonic
  sync anchor. Required by every non-FUSE backend.
- **Per-entry metadata extension**: optional `mtime`, `content_version`.
- **Block-level streaming reads**: confirm `read_file_range` doesn't force
  whole-file materialization anywhere.

All four are hashtree's concern, not hashdrive's. No hashdrive feature code yet.

### Phase 1 — `hashdrive-core` brain, headless (weeks 2–4)

- **Identity**: `hdrive init` creates a Nostr keypair under
  `~/.config/hashdrive/key`.
- **Drive model**: `Drive { owner_pubkey, drive_id, key, role }`. Primary
  drive is `{ owner = self, drive_id = "main", role = Owner }`.
- **Embedded daemon**: link `hashtree-embedded`, start on app launch with the
  config dir as block store path.
- **Indexer**: maintain the htree directory tree from the present working set.
- **Publisher**: debounce + publish new root over Nostr after each mutation
  (mirrors `hashtree-cli/app/mount_publish.rs`).
- **Subscriber**: open Nostr subscription for `(owner_pubkey, drive_id)`
  mutable-root events. New root → fetch diff → apply non-conflicting
  changes → flag conflicts.
- **Conflict resolution**: last-writer-wins by published timestamp,
  conflicted local file renamed `file (conflict from <device>).ext`.

Deliverable: `hdrive` CLI can create a drive, add files, see another device's
edits appear, republish. No OS mount yet.

### Phase 2 — First desktop platform end-to-end (weeks 4–7)

Start with **Linux**: `hashtree-fuse` already exists, fewest unknowns, easiest
e2e in Docker. Validates the full app loop fastest. macOS follows knowing the
trait surface is right.

- `hashdrive-fuse` adapter consumes `ProviderFs`.
- `hdrive` daemon mode mounts `~/Hashdrive` on startup, runs sync engine,
  exposes status over unix socket.
- e2e: two Docker containers, same identity, file in container A appears in
  container B's mount.

### Phase 3 — macOS FileProvider extension (weeks 7–11)

Two parallel tracks:

- **Apple entitlement request** for App Store distribution: file day 1. Multi-week
  response. Development on own devices does **not** wait on this (use
  `com.apple.developer.fileprovider.testing-mode` or self-enable the dev
  capability in the developer portal).
- **Extension build**:
  - `macos/HashdriveFileProvider/` Swift target subclassing
    `NSFileProviderReplicatedExtension`.
  - Links `hashtree-fileprovider` as an xcframework via UniFFI.
  - Containing app `macos/Hashdrive.app` registers the provider via
    `NSFileProviderManager`.
  - Maps `item`, `fetchContents`, `createItem`, `modifyItem`, `deleteItem`,
    `enumerator`, `evict` to `ProviderFs` ops.
  - Sync anchor = htree root CID; changes-since-anchor = Phase 0 diff API.

Validation: drive mounts at `~/Library/CloudStorage/Hashdrive-<account>/`,
Finder shows sidebar entry, edits round-trip to the Linux peer.

### Phase 4 — Sharing (weeks 11–13)

- **Send invite**: `hdrive share ./Photos --with npub1xxx --role reader`.
  Creates a child htree root for `./Photos`, publishes under a derived
  `drive_id`, encrypts access key with NIP-44 to recipient.
- **Receive invite**: app keeps an open Nostr subscription for DMs
  (no timed fetches — see CLAUDE.md rule), surfaces "X shared 'Photos'."
- **Mount the share** as a sibling drive —
  `~/Library/CloudStorage/Hashdrive-<account>/Shared/<owner-display>/Photos/`.
  Reuses every Phase 1 mechanism with a different `Drive { owner, role }`.
- **Revoke / leave**: owner publishes new key wrapped only to remaining
  members. Prior content can't be recalled (content-addressed); matches
  Drive/Dropbox reality.

### Phase 5 — Windows + remaining desktop (weeks 13–16)

- `hashtree-winfsp` Rust crate using the `winfsp` Rust binding.
- `windows/Hashdrive/` WPF installer bundles the WinFsp runtime.
- Mount under `%USERPROFILE%\Hashdrive` for parity with mac/linux.
- WPF status UI + tray icon mirroring the macOS shell.

### Phase 6 — Mobile (weeks 16–22)

- **iOS** first (Rust crate shared with macOS): `ios/HashdriveFileProvider/`
  reuses most macOS Swift code. Silent push for remote-change wake.
  Materialization budget honored. TestFlight beta.
- **Android**: `hashtree-saf` Rust crate, `HashdriveDocumentsProvider` Kotlin
  class, foreground service for sync engine. Picker integration via
  Files-by-Google.

### Phase 7 — Polish (ongoing)

- Status UI per platform: recent changes, conflicts to resolve, peers connected.
- Selective sync: per-folder "always available offline" vs "on-demand."
- Bandwidth limits (push upstream to hashtree-network or fips).
- File versioning UI surfaced via htree history (largely free — htree already
  tracks it).
- Background updater (pattern from nostr-vpn's `hashtree-updater` +
  `tauri-plugin-hashtree-updater`).
- Multi-account support.
- **Disk-usage view** (pie-chart style; squirreldisk for design inspiration).

## Decisions to lock in early

1. **Identity model**: device key vs user key. Recommendation — single user
   key copied to each device for v1 (matches Drive UX). Revisit with NIP-46 /
   hashdrive-specific delegation once multi-device threat model justifies it.
2. **Drive granularity**: one root per "drive" (My Drive + each share), not
   one root per user. Clean for sharing, mirrors Drive/Dropbox sidebar.
3. **Conflict resolution**: last-writer-wins + rename. No CRDT in v1.
4. **In-process vs out-of-process daemon**: in-process. Mobile sandboxes ban
   IPC anyway; `hashtree-embedded` exists.
5. **WebRTC / transport always-on**: yes on desktop; **opt-in or Wi-Fi-only
   on mobile**. Default to "sync over Wi-Fi only" with an "always" toggle.
6. **File visibility default**: private — only the user's own pubkey's
   devices see it. Sharing is opt-in per folder. Never default-public; mirror
   the global rule "ONLY push PUBLIC repos to public hashtree endpoints."

## Risks

- **Apple FileProvider App Store entitlement** can be denied or take 6+ weeks.
  Development on own devices is unaffected (dev capability self-enables). Have
  a fallback: FUSE-T mount under `~/Hashdrive` on macOS shippable through
  Developer-ID signed + notarized distribution while waiting on App Store.
- **Background sync on iOS** is famously restrictive. Plan for "syncs when app
  is foreground or when iOS feels like waking the extension." Same constraint
  as Drive/Dropbox; not solvable.
- **WebRTC NAT traversal at scale** is hashtree's (later fips's) problem.
  Worth a separate audit before Phase 2 if there's any doubt about the
  current TURN/relay story.
- **Storage growth**: content-addressed + retained revisions = unbounded
  growth. Need a gc policy and a "what's using space" UI by Phase 5 at the
  latest.
- **Cold-start materialization**: first open of a 10 GB file on a new device
  blocks `open(2)` until fetched. Progress UI is required, not optional.

## Why "Hashdrive"

Trademark-safe: "Drive" is descriptive in cloud-storage (iCloud Drive,
OneDrive, pCloud, etc.); the distinctive element is "Hash-." Avoids tying the
brand to a centralized-cloud framing. Falls under the Iris family
distributionally without requiring "Iris X" prefix.
