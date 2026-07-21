# Changelog

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
