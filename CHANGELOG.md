# Changelog

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
