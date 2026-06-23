# Experiments

Performance and integration experiments log. Omit identifying information
(pubkeys, secrets, IPs, private hostnames, exact repo names, raw hashes)
unless the user explicitly asks otherwise.

## 2026-06-23 provider write viewer-to-viewer latency

- Added an e2e latency probe that measures from a completed provider/viewer
  write on device A to the file becoming visible through device B's provider
  viewer. The first run showed the source daemon waiting for the old
  provider-root safety cadence: roughly 50-60 seconds across the matrix.
- Tightened provider-root notice handling and capped the quiet-watcher provider
  root sweep at 1 second. The steady-state work is cheap because unchanged
  roots return through the config/root-key cache.
- Verification command:
  `cargo test -p idrive --test daemon_sync_matrix live_daemons_provider_write_viewer_to_viewer_latency_probe -- --exact --nocapture`.
  Passing run measured about 1.3s, 0.5s, and 0.5s from source viewer completion
  to target viewer visibility across the three client hops.

## 2026-06-22 macOS roster/FIPS status CPU check

- Reproduced high CPU in the macOS app/daemon after app-key approval and
  remote device offline states. Samples showed repeated profile roster
  projection/signature verification from UI refresh, direct-root subscription,
  provider-root polling, and app-key roster resend paths.
- Added config/projection caches, bounded app-key roster retries, and live
  transport filtering for FIPS online status. Rebuilt and relaunched the macOS
  app locally.
- Final live check after warmup: app mostly idle with short refresh work,
  daemon around single-digit CPU, user-facing roster showed only the local app
  online and remote devices offline. Focused core/app-core/idrive tests passed.
