# Changelog

## Unreleased

- Save to Iris Drive on iPhone and the Files apps on iPhone and Android show
  your folders right away after another device changes the Drive, without
  waiting for Iris Drive to sync in the background.
- Large folders (such as photo libraries) open much faster in the Files app,
  Share Sheet, and CLI, and folders with more than 174 items now sync reliably
  across devices.
- Opening, saving, importing, and scrubbing large videos and files streams data
  chunk by chunk, reducing how much memory these operations need.
- Syncing edits to Blossom storage checks for unchanged blocks in batches and
  transfers blocks in parallel, making incremental syncs and initial
  downloads much faster with far less upload traffic.

## 0.1.52 - 2026-11-25

- Iris Drive for Mac is now published by Sirius Business Oy. After updating,
  macOS asks a few questions once: choose Open Anyway when it says Iris Drive
  differs from previously opened versions, and Allow when Iris Drive would like
  to access data from other apps. Your Drive is copied from the previous version,
  so you stay signed in with the same files. Macs on 0.1.50 or older need to
  download this version once by hand.
- Files listed in Drive open on Android and iPhone even when their contents have
  not synced yet; opening a file downloads it first.

## 0.1.51 - 2026-11-24

- Prepares Iris Drive for Mac to move to Sirius Business Oy as its publisher.
  This version can update to the first version published by Sirius Business Oy.
  That version copies your Drive from this one once; macOS asks whether
  Iris Drive may access data from other apps, and allowing it keeps you signed in.

## 0.1.50 - 2026-11-24

- Keep recent Calendar edits when the same calendar is changed in the browser
  and a native app.
- Show completed syncs correctly when several devices send files at the same time.
- Keep shared files ready to save on iPhone while destination folders sync,
  and resume interrupted shares without duplicating files already saved.
- Open the current Iris Sites catalog through the native Drive connection.
- Connect approved devices directly when a configured address works in only one direction.
- Open the pairing scanner from Devices on iPhone and identify the camera preview
  with Android accessibility tools.

## 0.1.49 - 2026-11-19

- Recover personal files from private Trash with `idrive trash`. Trash syncs
  across approved devices and keeps files for 30 days.
- A shared file stays in the folder until its owner has a verified copy on
  their Drive. Older shared files without a known owner are protected from
  deletion by this flow.

## 0.1.48 - 2026-11-18

- On iPhone and Android, Devices shows when a linked browser comes online or
  goes offline even if a relay stops responding to device-list updates.
- Keep keyed catalog reads on the native Hashtree connection.
- Update Hashtree CLI to 0.4.8 and nostr-pubsub to 0.1.22.

## 0.1.47 - 2026-11-17

- Fixed a crash that could occur when iOS suspended Drive during device discovery.
- On iPhone, “All files offline” appears beside the device’s connection status so the row keeps its height when syncing finishes.
- After you press Install, the Mac, Windows and Linux apps show each step:
  finding the release, downloading with a progress bar, checking the
  download, then installing and restarting or opening the installer. If it
  stops, they say why and offer Try Again; Install no longer just turns back
  on without a word. The wording comes from the shared core.

## 0.1.46 - 2026-11-16

- Your devices exchange Drive's own events (roots, roster changes and device
  storage reports) with your web devices over FIPS as well as through relays,
  so changes keep reaching every device when relays do not answer. Each device
  opens the same peer feed as the web app, only with the account's own
  devices, and Negentropy sends each side just the events it lacks.
- Uses Hashtree CLI 0.4.6, `nostr-pubsub-fips` 0.6.6 and FIPS 0.4.123.

## 0.1.45 - 2026-11-16

- Devices lists each device once and shows how much of the Drive it keeps offline.
- Web deletions reach native devices even when a browser republishes the same file tree. Files on iPhone and Mac can reconcile an existing file without creating a numbered copy.
- Files on iPhone updates its listing during sync without rebuilding the whole Drive, and shows when the Files location has been turned off.
- iPhone shares keep the original movie, retry interrupted uploads and include the keys the other devices need to open it.
- Phone backups use one Photos folder. Matching camera filenames preserve different originals, and another phone backing up the same original can reuse the existing copy.
- The roster recognizes an active encrypted connection through another peer as well as a direct one.
- Device approvals, roster changes and foreground connection updates can proceed while background file work is running. The iOS browser gateway releases its file locks when the app goes into the background.

- Iris Drive on Windows shows its real version in Settings and now offers
  updates when a newer release is out. It used to call itself 1.0.0.0, so it
  never saw any 0.x release as newer.
- Releases are made from hosted builds with tested scripts in the repository:
  staging and Android signing of the hosted products, iOS export and upload,
  the release-gate lab environment, the Mac launch smoke on the lab VM when
  this Mac runs Iris Drive, Zapstore readback, and `haps` and `htree` pinned
  by SHA-256 for publishing.
- People in a shared folder are listed as in Google Drive: you first, then
  each person by name with their role under it, on every platform. Someone
  who gave no name is "Someone" rather than an id. On Linux the Share and
  Remove access dialogs open over the Iris Drive window instead of in a
  corner of the screen; on Windows My Drive says whether it is in File
  Explorer instead of "SAF provider root", and screen readers read each list
  row's name.

## 0.1.44 - 2026-11-16

- Drive asks before logging out on this device. Going back while waiting
  for a device approval also lets you keep the sign-in request open.

## 0.1.43 - 2026-11-15

- See, under Devices, how much of your Drive each of your devices holds, in
  which folders, how much room it has, whether it has the same version of the
  Drive as this one and whether it is connected directly. Native apps and
  Iris Drive web tell each other, so every device shows every other one. Each
  device measures itself now and then and sends its numbers to your other
  devices only, encrypted; relays cannot tell what they are or whose they are.
  `idrive storage` prints the same list.

## 0.1.42 - 2026-11-15

- Save large videos to Iris Drive from another app's share sheet on iPhone.
  The share extension used to read the whole file into memory, so iOS closed
  it on large videos and nothing was saved; a 380 MB video now saves using
  under 50 MB of memory. Files saved through the Files app, Android and photo
  backup are also stored in chunks instead of read in all at once.
- The Backup page on iPhone and Android groups where My Drive is backed up,
  with Sync Now and Check All beside that list, and keeps the form for
  adding a backup folded until you need it. Photo backup now says when it
  last backed up.

## 0.1.41 - 2026-11-15

- Share folders by link from every app. Anyone who opens a drive.iris.to share
  link can join from a browser without installing anything, or from the app's
  Join field. The sharer's Drive lets them in. Links made in Iris Drive web
  work in the apps, and links made in the apps work on the web.
- Open drive.iris.to share links in the app on iPhone and Mac, with the choice
  to open them in the browser instead. Other drive.iris.to pages the Mac app
  cannot show open in the browser.
- Shared folders sit on disk the way they do in Google Drive: once, where
  they are in My Drive. The separate "Shared with me" folder is gone from
  Finder, Files and Explorer, and sharing a folder no longer adds a second
  copy at the top of the owner's Drive. Take a joined folder out of My Drive
  and add it back from Shares without losing access; deleting or moving its
  folder on disk only changes your own Drive, never the shared files.
- Share a folder from Finder on Mac: right-click it in Iris Drive and choose
  Share with Iris Drive… to get its link. Sharing a folder that is already
  shared opens the same share instead of making a second one, and a folder
  inside or around a shared folder can no longer be shared again.
- Pick the folder to share from a list on Mac, iPhone and Android, and get
  the share link right after creating a shared folder on iPhone.
- Folders shared or joined while Drive is running now sync without a restart.
- Shared folders follow your profile, as in Google Drive. A folder you share
  or join on one device appears on your other devices, in the same place in My
  Drive; moving it, taking it out of My Drive, leaving it or stopping sharing
  applies on all of them. A device you link later is let into the folders you
  own. For a folder someone else owns it asks to join with the link you used.
- The apps now tell Iris Drive web when they let a browser into a shared
  folder, so a browser linked to your profile can pick up the folders you
  share from an app, as one that joined by link already could.
- Save to Iris Drive from another app's share sheet on iPhone and Android
  asks first, as Google Drive does: check or change the name, pick the folder,
  then Save or Cancel. It used to save straight into My Drive without asking.
- Photo backup on iPhone shows its progress and when the last full backup
  finished, and says how to back up when it is waiting for Wi-Fi.

## 0.1.40 - 2026-11-15

- Add optional photo and video backup to My Drive on iOS and Android. Back up
  originals from the library you authorize, keep both files on a name collision,
  and retry pending uploads when connectivity returns.
- Add a photo folder shortcut to one-way folder mirroring on macOS. Removing a
  source file keeps the copy in Drive.
- Keep verified files available when a device block goes missing and a complete
  local snapshot covers the same device history. New files can be added safely
  while Drive retries the missing block. Edits to existing files wait for recovery.
- Restore My Drive from the recovery phrase and give newly linked devices the
  root that carries their key. Native updates can download through a running
  Drive service.

## 0.1.38 - 2026-10-19

### Security

- Update Negentropy to 0.5.2 to reject malformed synchronization messages before
  allocating memory (GHSA-jcg8-qg98-373g / RUSTSEC-2026-0365).

### Changed

- Update the compatible Hashtree and FIPS dependency set, retaining the iOS
  compatibility constraint for libc.

### Fixed

- Keep reopened device approval screens responsive during background sync and
  provider scans, and preserve new approvals when an older sync finishes.
- Keep device approval exchanges open long enough to deliver large rosters and
  acknowledgements, with bounded connection time, records and memory.
- Run native update checks and downloads on an owned thread with enough stack
  space for network startup, preventing crashes on native dispatch queues.
- Stop the macOS approval journey immediately when any required action fails.

## 0.1.37 - 2026-09-10

### Changed

- Use shared FIPS peer reputation and local transport observations when selecting
  mesh peers. External raters remain optional and are not trusted by default.
- Add `IRIS_DRIVE_FIPS_TRUSTED_RATERS` for explicit trusted `npub` or hex public
  keys, with invalid configuration rejected by the shared validator.
- Use the shared pubsub shutdown API consistently and remove the redundant
  ownership-unwrapping branch.

### Fixed

- Keep direct connections and own-store file transfers running when the optional
  same-user shared blob store cannot open, while reporting its error.
- Bound repeated connection attempts when a mesh peer does not provide the
  requested service, while retaining late-service recovery.
- Size shared FIPS cryptographic allocations for the admitted packet batch and
  remove repeated peer-selection key copies.
- Skip redundant approval-receipt verification when the signed roster already
  supplies usable authorization, preserving revocation precedence.
- Require exact-source CPU and bandwidth receipts before final publication,
  including sampling across the shared reputation maintenance interval.
- Support exact eligible iOS builds and resumable App Store review through the
  existing authenticated release helper.

## 0.1.35 - 2026-09-08

### Fixed

- Update embedded Hashtree components and networking dependencies with security
  fixes.
- Let slow valid peer downloads finish when backup storage has no copy, while
  retaining bounded reads and rejecting incomplete roots.
- Reduce repeated background retries while a mobile device is offline, while
  keeping pending device approvals responsive.
- Show Android device approval prompts immediately when a request arrives during
  native startup, while still requiring explicit confirmation.
- Recover missed desktop updates after a peer reconnects, while preserving the
  last complete file view until the newer signed root is available.
- Enforce mesh hop limits when forwarding missing blocks between peers,
  preventing repeated requests from circulating without consuming their budget.
- Reach additional available peers when retrying missing blocks, while keeping
  each read's concurrency and time limits bounded.
- Fix unsafe initialization in optional FUSE mounts while preserving Finder
  write compatibility.

## 0.1.34 - 2026-09-05

### Changed

- Apply authorized Drive-root events received from relays to the native
  provider view and fetch their Hashtree blocks from configured Blossom
  servers, so Web, iOS, Android, desktop, and CLI devices converge through the
  same durable protocol path.
- Make device approval hand off a complete logical `main` root: materialize an
  explicit empty root when necessary, upload every live block, publish the
  full roster, and publish a root newly wrapped for every active AppKey before
  emitting the encrypted approval receipt.
- Project file/directory path conflicts losslessly across native and Web
  clients. Causal replacements own the canonical path, concurrent conflicts
  keep the directory canonical and expose the file as a conflict copy, and a
  newer file relocates the complete replaced directory subtree.

### Fixed

- Retry staged provider writes after missing sync blocks arrive, releasing the
  daemon event loop and config lock between attempts so sync can recover.
- Reject incomplete provider directories instead of importing them as empty.
- Package the exact newly built Linux Debian artifact so older build outputs
  cannot be published under a new release version.
- Reserve enough filesystem capacity when creating the macOS installer image.
- Remove private build paths from iOS debug symbols while retaining symbolication.
- Reject unavailable directory blocks during merge and preserve the last
  accepted sync cache when a replacement tree cannot be read completely.
- Recognize shared NostrIdentity device-link URLs and `nostr:` wrappers, and
  ignore query/fragment hints when checking whether an invite is complete.
- Encode new profile content-key wraps as interoperable hexadecimal NIP-44
  plaintext while continuing to read legacy native raw-byte wraps. This keeps
  encrypted device names visible across native and Web clients and lets native
  devices acknowledge approvals created by Web.
- Preserve the native owner's device name and the joining device's requested
  name in approval roster facts instead of showing a generic linked-device
  label.
- Delay authorization publication when Blossom upload fails, preventing an
  approval receipt from activating a device whose current Drive root cannot be
  resolved.
- Treat encrypted and hash-only references to the same Hashtree root as the
  same causal identity, allowing Web observations at an equal sequence to
  establish ancestry without disclosing a root key.
- Preserve file bytes, whole-file hashes, modification metadata, nested empty
  directories, and accurate top-level counts when file and directory kinds
  replace one another.
- Keep an explicit file/directory replacement canonical when another device
  later publishes an unrelated edit, without duplicating or hiding the losing
  subtree's conflict copy.
- Serialize approval, live relay-root, and native-provider config mutations
  across app processes, while releasing the config lock before Blossom and
  relay network I/O.
- Let an outbound pending device initiate its one-shot FIPS link regardless of
  public-key ordering, without widening authorized-peer or block access.

## 0.1.33 - 2026-08-07

### Changed

- Exercise device approval, durable receipt cleanup, restarts, and post-link
  Drive sync through the shipped iOS, Android, Linux, Windows, macOS, and CLI
  paths. The optional two-phone gate additionally exercises QR and manual
  entry in both directions through the shipped UI, both physical cameras, and
  system file providers, and reports an explicit skip when hardware is not
  available.
- Consolidate redundant live-daemon scenarios while enforcing an eight-second
  approval ceiling throughout the production-like sync matrix, and avoid
  recursively rerunning workflow tests in the release-gate scheduler tests.
- Send approval receipts and roster events over FIPS first, then make one
  bounded relay attempt, so linking stays fast without losing relay durability.
- Prefer the configured remote macOS runner for release UI journeys, keeping
  native dialogs off the developer workstation while retaining the same
  shipped-app assertions.
- Record unsigned Windows artifacts as the project release policy while still
  requiring the expected CLI archive and installer in content-addressed final
  releases.

### Fixed

- Require explicit user confirmation before iOS, Android, Linux, macOS, or
  Windows approval links can add a device.
- Wake the mobile app-key exchange immediately after an owner approves a
  device, avoiding the idle maintenance delay before receipt delivery.
- Backfill profile roster events when an awaiting device becomes authorized,
  and acknowledge only after the roster contains both the approving and
  joining devices, so events published just before its encrypted receipt are
  not missed or mistaken for an unrelated actor.
- Make one-shot native sync persist an applied approval before publishing its
  exact acknowledgment, and let the owner consume that acknowledgment and
  clear the matching pending receipt after either side restarts.
- Preserve and acknowledge multiple concurrent approvals for the same profile,
  while rejecting receipts from a different profile and durably syncing the
  config directory after atomic writes.
- Filter bound approval receipts by their expected signer without trusting
  either device's wall clock, and query acknowledgments only from pending
  device signers for their receipt event IDs, preventing clock skew, malformed
  events, or unrelated relay traffic from crowding out valid handshakes.
- Serialize short config mutations across native actions and background FIPS
  delivery so a stale refresh cannot overwrite a newly applied device roster.
- Serialize FIPS peer reconfiguration, cache it only after every ACL update
  succeeds, and refresh immediately after receipt backfill so concurrent device
  approvals cannot leave one device on an older peer allowlist.
- Refresh the drive-root subscription after an unbound request adopts its
  profile, then download pre-existing roots after the durable approval ACK so
  an already-running linked device exposes existing files immediately.
- Retry provider writes from the latest on-disk root after a concurrent block
  arrival, preventing a stale provider snapshot from dropping either edit.
- Ignore Android-only test actions in release builds and queue approval links
  received before the app finishes starting.
- Advance version-derived native build numbers beyond the existing TestFlight
  sequence so a new app version cannot reuse an already accepted Apple build.
- Remap private builder paths in every Rust release target and in bundled
  Clang/GCC dependencies, and normalize Unix archive ownership and timestamps.
- Strip Xcode debug paths from shipped macOS executables and audit every
  bundled file before signing.
- Resolve the installed Linux CLI beside the desktop app or from `PATH`, while
  keeping checkout probing debug-only so release binaries cannot embed the
  builder's source directory.
- Require all nine canonical release assets to be regular files before final
  publication, rejecting missing CLI or Android bundle artifacts, symlinks,
  directories, unsigned intermediates, and other tag-matching build residue.
- Build and require both Windows Rust payloads, omit managed debug symbols,
  and remap native dependency paths before auditing the complete unsigned
  installer payload.
- Include the project's existing MIT terms in native release packages and
  correct the excluded Linux crate's incomplete license metadata.
- Declare generated Android license assets as inputs to release lint and
  packaging tasks so clean signed builds pass Gradle validation.
- Avoid false fast-gate failures when workflow contract tests share a busy
  builder with the parallel Rust checks.

## 0.1.32 - 2026-07-27

### Changed

- Add Apple privacy manifests, in-app Privacy and Support links, and
  deterministic App Store screenshot fixtures for the iOS app and extensions.

### Fixed

- Preserve the signed, notarized macOS app bundle during self-update so its
  FileProvider entitlement remains valid and Iris Drive stays visible in
  Finder.
- Keep ad-hoc macOS development installs under `macos/.build` by default
  instead of overwriting a signed release in `/Applications`.
- Stop playing the system failure sound when FileProvider is unavailable.
- Let the mobile browser page remain visible behind its compact controls
  instead of painting an unnecessary dark footer or safe-area strip.
- Build native iOS dependencies for the app's iOS 17 deployment target so
  release binaries do not accidentally require the current Xcode SDK.

## 0.1.31 - 2026-07-27

### Fixed

- Publish authenticated FIPS presence transitions to the native mobile status
  file immediately, so iOS and Android Devices views no longer wait for the
  15-second idle maintenance tick before showing a peer online or offline.
- Require bidirectional online presence signals within two seconds in the real
  FIPS control runtime regression.
- Disable Nostr direct-path upgrades in the routed-WebSocket regression lane,
  ensuring it deterministically verifies mesh-only device presence.

## 0.1.30 - 2026-07-27

### Changed

- Update `fips-core` and `fips-endpoint` to 0.4.44 and the Hashtree FIPS
  transport to 0.4.11.
- Keep one deterministic Drive peer responsible for each direct connection,
  avoiding duplicate simultaneous dials between authorized devices.

### Fixed

- Maintain an authenticated idle control channel between authorized devices
  and report established routed channels as mesh connectivity, so devices
  reachable through FIPS transit no longer appear offline merely because they
  are not direct physical neighbors.
- Let that authenticated control channel own application-peer reconnects so a
  healthy routed session does not continuously retry a redundant direct path.
- Request every peer's current root whenever a daemon starts, so a device that
  reconnects through a cached FIPS route catches up on changes made while it
  was stopped.
- Canonicalize simultaneous same-path edits from stable AppKey provenance so
  every device exposes the same original and conflict-copy filenames.
- Order concurrent roots from their signed embedded causal metadata rather
  than relay or direct-message republish time, which can differ by receiver.

## 0.1.29 - 2026-07-21

### Changed

- Update FIPS to 0.4.34 for reliable direct-path recovery during network
  changes, reconnects, handshakes, and rekeys.
- Update the FIPS adapter for `nostr-pubsub` to 0.4.7 while keeping
  `nostr-pubsub` on the newest 0.1.13 release.
- Disable ambient Android LAN multicast discovery by default to stay within
  the mobile idle-CPU budget; the explicit environment override remains.
- Back off mobile app-key maintenance after approval is stable while retaining
  the faster retry cadence during device approval.
- Avoid rebuilding unchanged peer policy and periodic direct-root work without
  a connected authorized peer.
- Throttle unchanged recent-peer cache refreshes so status polling does not
  rewrite the cache on every pass.
- Refresh unchanged mobile connectivity counters once per minute while still
  publishing peer and error changes immediately.

## 0.1.28 - 2026-07-19

### Changed

- Use the transport-neutral Nostr pubsub router and shared INV/WANT protocol
  through FIPS/TCP, with traditional Nostr relay support remaining a separate
  router source.
- Remove the retired Nostr-relay FIPS packet carrier from the consumed
  Hashtree/FIPS stack.
- Use the LNVPS and Osiris authenticated WebSocket gateways as the default
  FIPS first-adjacency entry points while preserving explicit overrides.
- Update the FIPS adapter for `nostr-pubsub` to 0.4.3.

## 0.1.27 - 2026-07-18

### Changed

- Route Drive blob reads adaptively across local, direct FIPS, and shared Hashtree paths.
- Reuse same-host Hashtree blobs and the released Hashtree transport substrate.
- Keep FIPS control and blob routing on the shared reliable carrier stack, with the hardened FIPS 0.4.8 stream and relay lifecycle.

### Fixed

- Avoid installing the managed macOS daemon for ad-hoc development builds, which cannot safely use the signed service lifecycle.
- Build the macOS Rust core for the app's declared macOS 14 deployment target.
- Keep macOS File Provider development signing and lifecycle checks aligned with ad-hoc app behavior.
- Compact long native device names safely when relaying app-key approval requests.
- Hedge Hashtree provider reads so a failed or slow first TCP/FIPS provider cannot starve a healthy peer.
- Back off inactive TCP/FIPS blob and control polling to keep mobile idle CPU within the release budget.
- Keep mobile FIPS startup inside the app sandbox instead of opening the desktop shared LMDB route.
- Clean up simulator app and File Provider processes after iOS idle checks so later platform gates remain isolated.
- Keep the daemon responsive when mesh pubsub is explicitly disabled instead of panicking in its receive loop.
- Share one native mobile runtime between foreground and background handles to avoid duplicate FIPS and gateway workers.
- Allow cold Windows peer builds enough setup time in the cross-platform release gate.
- Retry transient local root-resolution misses while opening Iris Apps on iOS.
