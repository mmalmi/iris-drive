# Experiments

Performance and integration experiments log. Omit identifying information
(pubkeys, secrets, IPs, private hostnames, exact repo names, raw hashes)
unless the user explicitly asks otherwise.

## 2026-08-06 device-link and release-gate latency

- A real two-daemon approval completed in about 0.64s; the complete
  approval-plus-bidirectional-file scenario passed in 8.63s. The shared live
  matrix now applies an eight-second approval ceiling to every newly linked
  daemon instead of maintaining a second timing-only setup.
- The one-shot CLI/relay regression completed receipt application, durable ACK
  publication, owner restart, exact ACK consumption, and pending-receipt
  cleanup in 3.07s.
- A relay-and-Blossom-only regression linked an already-running unbound daemon,
  rebound its drive-root subscription, and exposed a file created before
  approval in 7.24s with zero direct FIPS connections.
- The provider stale-root retry regression passed five consecutive contention
  runs plus a clean follow-up. Approval in those runs completed in about
  0.23-0.52s while concurrent local and incoming roots both remained visible.
- The four-lane browser link matrix passed in about 1.4 minutes. Its two
  browser/browser flows took 22.8s and 22.4s end to end; a restarted browser
  joined a native owner with 24 prior key rotations in 36.5s, and the reverse
  native-joiner flow took 1.3s. Both measured approval phases stayed below
  ten seconds.
- Stubbing the workflow-contract lane inside the release-gate scheduler test,
  rather than recursively running that suite for each scheduler case, reduced
  the harness from about 11.25s to 2.38s. Removing two redundant live-daemon
  setups also shortens the real release gate while retaining the stronger
  combined timing and post-link assertions.
- The physical iOS/Android camera suite was structurally and build verified,
  but its runtime correctly skipped because no paired iOS device was online;
  no physical delivery timing is claimed.
- The shipped macOS UI journey passed on a remote macOS runner. Its two direct
  route preconditions took 2.74s and 2.20s; approval through authorization,
  direct FIPS, and durable ACK drain took 2.22s and 2.81s. The post-link
  provider write became visible in 4.00s.
- A short remote idle sample passed: the macOS app averaged 0.38% CPU and
  peaked at 0.97%; its daemon averaged 4.23% and peaked at 8.61%.
- The physical macOS/Android manual-entry matrix is not release-green. The
  earlier shipped confirmation submitted successfully, but a macOS-owner to
  Android-joiner run exceeded the strict 15s ceiling. After the delivery fixes,
  final reruns stopped before approval submission: first because the isolated
  macOS runner exhausted its build volume, then because Accessibility trust
  failed at harness preflight. Cleanup restored the Android device to direct
  connectivity and removed both test packages and all ADB mappings. No
  post-fix physical delivery timing is claimed.

## 2026-07-22 release-gate critical path

- A warm serial `verify-fast` profile took about 142s. Rust tests accounted
  for about 123s: workspace tests took 79s and the focused idrive command took
  44s. Running those independent Cargo lanes together passed in 27s warm wall
  time, versus about 46s warm serial time. The run shared Cargo artifacts and
  reported brief artifact/package-cache lock waits.
- The measurements were intentionally treated as directional: background VM,
  backup, and Rust activity made the host noisy. The release workflow now
  records grouped lane output so failures remain attributable even when work
  overlaps. After the scheduling change, the same fast tier passed in 31.3s
  wall time on the noisy host, about 78% below the measured serial baseline;
  a later fully warm validation passed in 23.8s, about 83% below the baseline.
- The full-gate topology also repeated native work: three macOS app builds,
  two iOS builds, three Android instrumentation launches plus a separate
  assemble, serial platform preflights, and serial five-host setup. The gate
  now builds each local native app once, overlaps independent device/host
  lanes, and reuses same-invocation local iOS/Android functional checks while
  retaining physical-iOS, Android-provider, idle-CPU, and cross-device sync
  coverage.

## 2026-06-23 macOS FileProvider provider-list CPU

- Reproduced the Finder CPU spike as FileProvider-triggered
  `idrive provider list` helper processes. The installed helper repeatedly
  measured about 1.54-1.55s real time and 1.40-1.42s user CPU for a read-only
  list of the live provider surface.
- The hot path built the merged provider view twice: once while materializing
  the visible root and again while building the timestamp index for list
  entries. Added a core helper that materializes the provider root from an
  already-computed merged view, then switched CLI and native provider list
  callers to reuse that view.
- Added macOS FileProvider cache invalidation keyed by the daemon
  `summary.provider_refresh_key`, with a local `updated_at` freshness check, so
  unchanged Finder enumerations do not spawn another helper after the old 1s
  TTL.
- Patched optimized helper timing on the same provider surface: three runs at
  about 0.08-0.10s real time and 0.03-0.04s user CPU. One debug-logging run
  retired about 0.87B instructions versus about 12.1B for the installed helper.

## 2026-06-23 provider write viewer-to-viewer latency

- Added an e2e latency probe that measures from a completed provider/viewer
  write on device A to the file becoming visible through device B's provider
  viewer. The first run showed the source daemon waiting for the old
  provider-root safety cadence: roughly 50-60 seconds across the matrix.
- Tightened provider-root notice handling so provider mutations ping a daemon
  loopback wake endpoint, with the config/provider filesystem watcher still
  active as an event source. The old 30s+ sweep remains only for the degraded
  case where the watcher cannot start.
- Verification command:
  `cargo test -p idrive --test daemon_sync_matrix live_daemons_provider_write_viewer_to_viewer_latency_probe -- --exact --nocapture`.
  Passing run measured about 0.13s, 0.14s, and 0.14s from source viewer completion
  to target viewer visibility across the three client hops.
- Isolated count-reaction probe used a throwaway `/tmp/iris-drive-latency.*`
  config/data dir, not the user's FileProvider/CloudStorage directory. After
  skipping the full provider-tree placeholder preflight for normal non-empty
  writes, one provider write completed in 58 ms and the temp daemon status
  file count changed in 69 ms.

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
