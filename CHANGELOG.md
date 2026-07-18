# Changelog

## 0.1.27 - 2026-07-18

### Changed

- Route Drive blob reads adaptively across local, direct FIPS, and shared Hashtree paths.
- Reuse same-host Hashtree blobs and the released Hashtree transport substrate.
- Keep FIPS control and blob routing on the shared reliable carrier stack, with the hardened FIPS 0.4.6 stream lifecycle.

### Fixed

- Avoid installing the managed macOS daemon for ad-hoc development builds, which cannot safely use the signed service lifecycle.
- Build the macOS Rust core for the app's declared macOS 14 deployment target.
- Keep macOS File Provider development signing and lifecycle checks aligned with ad-hoc app behavior.
- Compact long native device names safely when relaying app-key approval requests.
- Hedge Hashtree provider reads so a failed or slow first TCP/FIPS provider cannot starve a healthy peer.
- Back off inactive TCP/FIPS blob and control polling to keep mobile idle CPU within the release budget.
- Keep mobile FIPS startup inside the app sandbox instead of opening the desktop shared LMDB route.
