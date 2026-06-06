# Iris Drive design

End-user file sync with Google-Drive-/Dropbox-style UX, built on
content-addressed storage (hashtree), IrisProfile identity, and Nostr/FIPS
discovery. P2P as far as each OS allows.

## Goals

- One **"My Drive"** per IrisProfile — a private hashtree root the profile's
  authorized AppKeys can edit, presented as a folder in the OS file manager.
- Sync that root across all of the user's app installs automatically.
- Each app install seeds its blocks P2P to authorized peers through
  hashtree-over-FIPS.
- **Share** folders with specific other IrisProfile members. A share has its
  own cryptographic root, entity roster, roles, key epochs, wraps, and
  core-derived key status (`available`, `repair_needed`, `key_unavailable`,
  etc.); AppKeys remain the concrete signing/decryption actors under each
  member profile. Recipients see shares under **Shared with me** and may add
  shortcuts into My Drive.
- No DNS, SSL, CDNs, or centralized servers. Identity = IrisProfile UUID plus
  signed AppKey/recovery/social facets, not a primary Nostr pubkey.
- Native shells per platform, single shared Rust core.

## Architecture

Single in-process hashtree daemon (`hashtree-embedded`) inside the iris-drive
app holds the blocks, maintains the index, publishes the user's mutable root
over Nostr, and seeds those blocks to peers. The user's "drive" is **one
hashtree root they own**, served to the OS through whichever presentation
backend the platform supports (FileProvider / WinFsp / FUSE / DocumentsProvider).

Multiple app installs owned by the same IrisProfile reconcile their drive root
through signed roster ops, AppKey-signed root events, direct FIPS messages, and
optional relay/Blossom caching. Shares reuse the same root-event machinery but
are scoped by a share UUID and authorized by the share roster, not by an owner's
Nostr pubkey.

No separate daemon process, no IPC — `iris-drive-app-core` links
`hashtree-embedded` as a library and the app extensions / shells link
`iris-drive-app-core` via UniFFI.

```
┌─────────────────────────────────────────────────────────────┐
│  OS file surface (Finder / Explorer / Files app)            │
└──────┬───────────────┬─────────────────┬────────────────────┘
       │ FileProvider  │ WinFsp / FUSE   │ DocumentsProvider
       ▼               ▼                 ▼
┌────────────────────────────────────────────────────────────┐
│  Native shell (Swift / WPF / GTK / Compose)                 │
│  ⇄ iris-drive-app-core (UniFFI)                              │
└──────────────────────────┬─────────────────────────────────┘
                           ▼
┌────────────────────────────────────────────────────────────┐
│  iris-drive-core                                             │
│   - drive model, indexer, sync engine, share manager        │
│   - reconciler + conflict resolution                        │
└──────────────────────────┬─────────────────────────────────┘
                           ▼
┌────────────────────────────────────────────────────────────┐
│  hashtree-embedded                                          │
│   - block store, root publish, mutable-root subscriptions   │
│   - peer seeding via FIPS                                   │
└────────────────────────────────────────────────────────────┘
```

## External project relationships

- **`~/src/hashtree`** — supplies the storage primitive and all OS-mount
  adapters (see "Where adapter crates live" below). iris-drive does not fork
  hashtree; it consumes it and contributes upstream.
- **`~/src/fips`** — peer-to-peer transport. Iris Drive consumes it through
  hashtree/FIPS for direct block replication and keeps Blossom as an optional
  remote cache.
- **`~/src/squirreldisk`** — disk-usage pie chart analyzer. Reference only for
  the "what's using space" UI idea in Phase 7; not a code dependency. Look at
  it for visualization inspiration, no obligation to extract or reuse.

## Where adapter crates live

Rust adapters live in **hashtree**. App-specific shells (with bundle IDs,
installer wiring, sidebar labels) live in **iris-drive**.

```
hashtree/rust/crates/
├── hashtree-fuse              Linux + (legacy) macOS-FUSE
├── hashtree-winfsp            Windows (new)
├── hashtree-fileprovider      macOS + iOS shared Rust core (new)
└── hashtree-saf               Android DocumentsProvider Rust core (new)

iris-drive/
├── linux/                            systemd user unit, mount at ~/Iris Drive
├── windows/                          installer bundles WinFsp
├── macos/Iris DriveFileProvider/      Swift extension target
├── ios/Iris DriveFileProvider/        Swift extension (shares code w/ macos)
└── android/.../Iris DriveDocumentsProvider.kt
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
- **Block-level streaming reads**: confirm `read_file_range` can serve only the
  requested bytes.

All four are hashtree's concern, not iris-drive's. No iris-drive feature code yet.

### Phase 1 — `iris-drive-core` brain, headless (weeks 2–4)

- **Identity**: `idrive init` creates an IrisProfile UUID, a fresh per-install
  AppKey under `~/.config/iris-drive/key`, and a recovery phrase authority.
  UUIDs are created randomly or learned from verified roster evidence; they are
  never derived from Nostr pubkeys, nsecs, recovery phrases, or other recovery
  secrets. Bare recovery phrase / `nsec` restore can create a fresh local
  IrisProfile with that secret as recovery authority; recovering an existing
  UUID requires roster ops, acceptance breadcrumbs, invites, or export data that
  carry the UUID and verify against the recovery key. If relays return multiple
  verified UUID candidates for the same recovery/NIP-46 pubkey, core returns all
  candidates with roster metadata; UI may auto-pick only an unambiguous single
  candidate. Otherwise the user chooses, or keeps the fresh fallback profile.
  Merging distinct UUID profiles is a later explicit import/migration flow, not
  automatic identity-log union.
  Roster ops are not lockstep multisig documents: an op is signed by the key
  authorized to make that change. Member keys may also sign join/acceptance
  breadcrumbs for their own facet so they can later rediscover candidate
  IrisProfile UUIDs. Roster ops tag mentioned facet pubkeys with `p` tags so
  a key can also search for roster evidence that names it. Both paths are only
  discovery hints until the client projects the authoritative roster log and
  confirms the facet is active and not tombstoned.
- **Drive model**: primary My Drive is scoped by `IrisProfileId` (`root_scope_id`
  in config) with per-AppKey roots. AppKeys are actors; recovery/NIP-46 facets
  can admit fresh AppKeys and optionally decrypt key epochs but do not sign drive roots.
- **Embedded daemon**: link `hashtree-embedded`, start on app launch with the
  config dir as block store path.
- **Indexer**: maintain the htree directory tree from the present working set.
- **Publisher**: debounce + publish new root over Nostr after each mutation
  (mirrors `hashtree-cli/app/mount_publish.rs`).
- **Subscriber**: keep open subscriptions/direct-message streams for profile
  roster ops and `(profile_id, drive_id)` root events. New root → fetch diff →
  apply non-conflicting changes → flag conflicts.
- **Conflict resolution**: last-writer-wins by published timestamp,
  conflicted local file renamed `file (conflict from <app install>).ext`.

Deliverable: `idrive` CLI can create a drive, add files, see another app install's
edits appear, republish. No OS mount yet.

### Phase 2 — First desktop platform end-to-end (weeks 4–7)

Start with **Linux**: `hashtree-fuse` already exists, fewest unknowns, easiest
e2e in Docker. Validates the full app loop fastest. macOS follows knowing the
trait surface is right.

- `iris-drive-fuse` adapter consumes `ProviderFs`.
- `idrive` daemon mode mounts `~/Iris Drive` on startup, runs sync engine,
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
  - `macos/Iris DriveFileProvider/` Swift target subclassing
    `NSFileProviderReplicatedExtension`.
  - Links `hashtree-fileprovider` as an xcframework via UniFFI.
  - Containing app `macos/Iris Drive.app` registers the provider via
    `NSFileProviderManager`.
  - Maps `item`, `fetchContents`, `createItem`, `modifyItem`, `deleteItem`,
    `enumerator`, `evict` to `ProviderFs` ops.
  - Sync anchor = htree root CID; changes-since-anchor = Phase 0 diff API.

Validation: drive mounts at `~/Library/CloudStorage/Iris Drive-<account>/`,
Finder shows sidebar entry, edits round-trip to the Linux peer.

### Phase 4 — Sharing (weeks 11–13)

- **Create share**: `idrive shares create Photos --name Photos`. Creates an
  internal share root, initializes an entity-oriented share roster, and records
  the owner IrisProfile as an admin member.
- **Send invite**: `idrive shares invite <share-id> --recipient-evidence
  recipient-profile.json --role reader`. The evidence bundle contains the
  selected representative npub/pubkey, signed IrisProfile roster ops, and
  self-signed facet acceptances. Direct `--profile --app-key` remains a
  diagnostic/admin path. The selected representative npub/contact is only a
  discovery/display hint; access is granted to the resolved IrisProfile member,
  while concrete AppKeys receive key wraps and scoped signing/decryption
  capabilities. The recipient's IrisProfile roster is the authority for
  key-to-UUID membership; the share roster only records UUID-to-role membership.
  External contact indexes such as `nostr-social-graph` may rank/search
  representative npubs, but they are not share authority.
  Inviting rotates the share epoch and emits a compact invite bundle containing
  a signed roster checkpoint/proof. The checkpoint summarizes entity members,
  compact roster heads, current key epoch, and missing-wrap state; the
  append-only roster op log remains authoritative.
- **Accept invite**: `idrive shares accept <share-invite-url>`. The recipient
  imports the shared folder only if the invite names their IrisProfile.
- **Receive invite**: app keeps an open Nostr subscription for DMs
  (no timed fetches — see CLAUDE.md rule), surfaces "X shared 'Photos'."
- **Receive share**: shared folders appear under `Shared with me/<name>`.
  Recipients can add shortcuts anywhere in My Drive. Team/shared DriveSpaces can
  come later; the first UX remains one Iris Drive.
- **GUI parity**: native control panels render app-core `UiShare` and
  `UiShareMember` state from the **Shares** tab and dispatch app-core actions
  for create/invite/accept/revoke/shortcut/repair. They do not reimplement share
  authority or key-wrap validation.
- **Revoke / leave**: share admins revoke an IrisProfile member, tombstone all
  known AppKeys for that profile in the share roster, rotate the share epoch,
  and publish the new key only to remaining active members. Prior content can't
  be recalled (content-addressed); matches Drive/Dropbox reality.

### Phase 5 — Windows + remaining desktop (weeks 13–16)

- `hashtree-winfsp` Rust crate using the `winfsp` Rust binding.
- `windows/Iris Drive/` WPF installer bundles the WinFsp runtime.
- Mount under `%USERPROFILE%\Iris Drive` for parity with mac/linux.
- WPF status UI + tray icon mirroring the macOS shell.

### Phase 6 — Mobile (weeks 16–22)

- **iOS** first (Rust crate shared with macOS): `ios/Iris DriveFileProvider/`
  reuses most macOS Swift code. Silent push for remote-change wake.
  Materialization budget honored. TestFlight beta.
- **Android**: `hashtree-saf` Rust crate, `Iris DriveDocumentsProvider` Kotlin
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

1. **Identity model**: IrisProfile UUID plus typed facets. Every app install
   has its own AppKey. There is no primary Nostr pubkey, and profile UUIDs are
   not derived from key material.
2. **Authority model**: signed append-only roster op logs with deterministic
   projection, tombstones, and key-wrap status/repair state derived in Rust
   core. Facet acceptance events are self-signed breadcrumbs, not roster
   authority. Signed checkpoints may summarize a projected roster for invites
   or transport, but they do not replace the op log. Do not add a general CRDT
   library unless it clearly simplifies this model.
3. **Drive granularity**: one user-facing My Drive, with internal share roots
   for shared folders. Recipients see Shared with me and optional shortcuts,
   not a pile of separate sidebar drives.
4. **Conflict resolution**: causal merge where available, conflict copies for
   concurrent edits/deletes. No document-level CRDT in v1.
5. **In-process vs out-of-process daemon**: in-process. Mobile sandboxes ban
   IPC anyway; `hashtree-embedded` exists.
6. **WebRTC / transport always-on**: yes on desktop; **opt-in or Wi-Fi-only
   on mobile**. Default to "sync over Wi-Fi only" with an "always" toggle.
7. **File visibility default**: private — only authorized AppKeys with key
   wraps can read current encrypted roots. Sharing is opt-in per folder. Never
   default-public; mirror the global rule "ONLY push PUBLIC repos to public
   hashtree endpoints."

## Risks

- **Apple FileProvider App Store entitlement** can be denied or take 6+ weeks.
  Development on own devices is unaffected (dev capability self-enables). Have
  an alternate distribution path: FUSE-T mount under `~/Iris Drive` on macOS
  shippable through Developer-ID signed + notarized distribution while waiting
  on App Store.
- **Background sync on iOS** is famously restrictive. Plan for "syncs when app
  is foreground or when iOS feels like waking the extension." Same constraint
  as Drive/Dropbox; not solvable.
- **WebRTC NAT traversal at scale** is hashtree's (later fips's) problem.
  Worth a separate audit before Phase 2 if there's any doubt about the
  current TURN/relay story.
- **Storage growth**: content-addressed + retained revisions = unbounded
  growth. Need a gc policy and a "what's using space" UI by Phase 5 at the
  latest.
- **Cold-start reads**: first open of a 10 GB file on a new device blocks
  `open(2)` until the needed bytes are fetched. Progress UI is required, not optional.

## Why "Iris Drive"

Trademark-safe: "Drive" is descriptive in cloud-storage (iCloud Drive,
OneDrive, pCloud, etc.); the distinctive element is "Hash-." Avoids tying the
brand to a centralized-cloud framing. Falls under the Iris family
distributionally without requiring "Iris X" prefix.
