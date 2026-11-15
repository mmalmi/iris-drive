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
  member profile. As in Google Drive, a shared folder is on disk only where
  it sits in My Drive: at the owner's own path, or at the shortcut a member
  keeps. The app's **Shares** list is the "Shared with me" view; there is no
  such folder on disk.
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
are scoped by a share UUID and authorized by the share access snapshot, not by
an owner's Nostr pubkey.

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

- **Identity**: `idrive init` creates an IrisProfile UUID and a fresh
  per-install AppKey under `~/.config/iris-drive/key`.
  UUIDs are created randomly or learned from verified roster evidence; they are
  never derived from Nostr pubkeys, nsecs, recovery phrases, or other recovery
  secrets. Recovery phrase authority is optional explicit recovery material.
  Bare recovery phrase / `nsec` restore can create a fresh local
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
  Each My Drive root key is also wrapped for every active recovery phrase facet
  (not for shared folders), so a phrase restore with no other device online opens
  the devices' latest roots, then publishes its own merged root. Adding a phrase
  also has the phrase key sign its facet acceptance, which web sign-in looks up.
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
  internal share root, initializes a canonical share access snapshot, and
  records the owner IrisProfile as an admin member.
- **Send invite**: `idrive shares invite <share-id> --recipient-evidence
  recipient-profile.json --role reader`. The evidence bundle contains the
  selected representative npub/pubkey, signed IrisProfile roster ops, and
  self-signed facet acceptances. A recipient can export the local current
  AppKey proof bundle with `idrive shares recipient-evidence --display-name
  <name> > recipient-profile.json`; this does not grant authority by itself.
  Direct `--profile --app-key` remains a diagnostic/admin path. The selected
  representative npub/contact is only a
  discovery/display hint; access is granted to the resolved IrisProfile member,
  while concrete AppKeys receive key wraps and scoped signing/decryption
  capabilities. The recipient's IrisProfile roster and self-links prove
  key-to-UUID identity. The share access snapshot records signed
  UUID-to-role/status grants, concrete AppKey/device entries, key epochs, wraps,
  and tombstones. Projected member maps and any separate AppKey-to-IrisProfile
  map are derived/cache data, not authority. External contact indexes such as
  `nostr-social-graph` may rank/search representative npubs, but they are not
  share authority.
  If a share dialog only has a representative npub/contact and no signed
  IrisProfile evidence yet, Rust core may record a pending invite hint keyed by
  that npub. Pending hints are display/contact state; they do not add members,
  AppKey facets, key wraps, or write authority.
  Inviting rotates the share epoch and emits a compact invite bundle containing
  the signed canonical access snapshot for the share.
- **Accept invite**: `idrive shares accept <share-invite-url>`. The recipient
  imports the shared folder only if the invite names their IrisProfile.
- **CLI projection**: normal share JSON is entity/count oriented. `list`,
  `revoke`, and `repair-wraps` report IrisProfile members, roles/status, repair
  state, and AppKey counts without listing individual AppKeys. Raw missing,
  repaired, or revoked AppKeys are diagnostics only via explicit
  `--diagnostics` flags.
- **Receive invite**: app keeps an open Nostr subscription for DMs
  (no timed fetches — see CLAUDE.md rule), surfaces "X shared 'Photos'."
- **Receive share**: joining from the app adds a My Drive shortcut, the
  folder's only place on disk. Deleting that folder on disk, or **Remove from
  My Drive**, takes the shortcut away and keeps membership; **Add to My
  Drive** puts it back; moving or renaming it moves the shortcut. None of
  these touch the shared files. An owner's shared folder moves with them and
  cannot be deleted on disk until they stop sharing it. Team/shared
  DriveSpaces can come later; the first UX remains one Iris Drive.
- **Same on every device**: sharing is per profile, as it is per account in
  Google Drive, with no server to hold the list. Each AppKey publishes the
  profile's shared folders (`share_index`) as replaceable events, kind 30078
  with `l` = `iris-drive/share-index` and `i` = the profile id. A short list
  is one event at `d` = `iris-drive/<profile>/share-index`; a long one is
  split by folder over further events (`.../share-index/<n>`), so no event
  outgrows what relays accept, and a reader merges every part it is given.
  The content is AES-256-GCM under a fresh key per event that is NIP-44
  wrapped for each of the profile's AppKeys, the same recipients a drive root
  is wrapped for, and it travels the same ways: relays and the direct root
  exchange. Relays see the profile id, the device keys and the size, never
  the folders. An entry holds the share id, owner profile, the admin AppKey
  to trust, name, role, the key of the join link if one was used, and the
  place in My Drive (the owner's source path, a member's shortcuts).
  Entries merge per share and every device reaches the same result in any
  order: a later generation wins, then removal, then the later change; a
  device stamps its change after every change it has seen, so clocks do not
  decide. Devices that change different folders at once both keep their
  change. Leaving a folder and coming back to one the profile left are said
  outright by the action; a folder merely missing from a device's config is
  added again, and one merely present after the profile left is dropped
  again, so a stale config never makes the profile leave or rejoin. The list
  file is written only under the config transaction lock and belongs to one
  profile. The list grants nothing: a device adds a folder only from the
  access snapshot signed by the entry's admin, once that snapshot includes
  it. For a folder the profile owns, the admin AppKey named in the entry adds
  the profile's newly linked AppKeys and republishes; if the profile removed
  that AppKey, another one that manages the folder takes over. For a folder
  someone else owns, a device that is not in the snapshot asks with the
  profile's join link. **Remove from My Drive**, moving, leaving and **Stop
  sharing** on one device apply on the others.
- **Telling a device it was let in**: Iris Drive web does not read the
  profile's list. An admin AppKey that lets devices into a folder, its own
  profile's or those of someone who asked with a join link, sends each one
  the note Iris Drive web listens for (`share_added`): kind 30078 with `l` =
  `iris-drive/share-added`, one `p` tag naming the device, and `d` =
  SHA-256 of the label, share id and device key joined by newlines. The
  content is NIP-44 for that device alone: `v`, `share_id`,
  `owner_profile_id`, `name`, and `role` (`editor` or `reader`, a hint). The
  note grants nothing: the device waits for the access snapshot signed by the
  note's author and adds the folder once that snapshot includes it. Devices
  still to be told are kept in `share-added.json` until their notes are sent.
- **GUI parity**: native control panels render app-core `UiShare`,
  `UiShareMember`, and `UiPendingShareInvite` state from the **Shares** tab,
  including source path, role/status, pending contacts,
  repair-needed/missing-wrap detail, and shortcut state. They
  dispatch app-core actions for create, invite, accept, role changes, revoke,
  shortcut, and repair. Invite dialogs accept signed recipient evidence before
  falling back to direct AppKey entry. Native shells can dispatch
  `export_share_recipient_evidence` and copy app-core
  `last_share_recipient_evidence` from a share-screen **Copy my share identity**
  action instead of building identity proofs themselves. They do not
  reimplement share authority or key-wrap validation.
- **Revoke / leave**: share admins revoke an IrisProfile member, tombstone all
  known AppKeys for that profile in the share access snapshot, rotate the share
  epoch, and publish the new key only to remaining active members. Prior
  content can't be recalled (content-addressed); matches Drive/Dropbox reality.

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
2. **Authority model**: IrisProfile identity still uses signed roster op logs
   with deterministic projection. Share access uses one signed canonical
   replaceable snapshot per share, with UUID grants, concrete AppKeys/devices,
   tombstones, key epochs, and key-wrap status/repair state derived in Rust
   core. Facet acceptance events are self-signed breadcrumbs, not roster or
   share authority. Do not add a general CRDT library unless it clearly
   simplifies this model.
3. **Drive granularity**: one user-facing My Drive, with internal share roots
   for shared folders. Members see them in the Shares list and as My Drive
   shortcuts, not as a pile of separate sidebar drives.
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
