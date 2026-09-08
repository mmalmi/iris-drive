# Experiments

Performance and integration experiments log. Omit identifying information
(pubkeys, secrets, IPs, private hostnames, exact repo names, raw hashes)
unless the user explicitly asks otherwise.

## 2026-09-08 desktop approval daemon handoff

- A five-daemon run reached GTK approval submission, then failed the existing
  15-second authorization/ACK barrier. The owner daemon was stopped and the
  Windows joiner had no roster. The smoke exited after observing a queued
  receipt and killed the process named by the config lock.
- Cleanup now stops the GUI and its children without treating a config lock
  as process ownership. The primary approval flow restores its configured
  harness daemon after the shipped GTK action restarts its own daemon. This
  handoff counts against the same 15-second limit. The timer marker is now
  flushed immediately before accessibility activation, with successful action
  acknowledgment still separate, so synchronous approval work is included.
- Focused process regressions reproduced termination of unrelated lock holders
  and the missing daemon handoff, then passed for existing/replacement lock
  holders, GUI children, action/start failures, missing successful submission, marker ordering, and unchanged
  deadline binding.
  The three focused workflow tests, including explicit remote Cargo settings,
  passed in 0.876 seconds. This does not measure a full native gate speedup.

## 2026-09-08: Deterministic iOS approval release check

The iOS release check retains a 15-second budget from starting the owner approval command through receiver authorization and the owner consuming its acknowledgement. Its receiver was already running before approval; restarting it inside that budget interrupted the exchange and could prevent timely observation. The harness now observes the running receiver.

A subsequent native run failed because the approval command took 33.14 seconds. The receiver had persisted its approval within one second, but command completion still violated the gate. That long command was not reproduced in the controlled follow-up. With the existing application binary, isolated real owner/receiver daemons, and public Blossom storage, approval took 6.574 seconds with default plus local relays and 6.172 seconds with only the local relay. Three sequential public-storage uploads dominated both runs; after the final upload, publication and shutdown together took approximately 0.706 seconds with default plus local relays and 0.013 seconds with only the local relay. The owner persisted acknowledgement at 1.809 and 1.112 seconds, with no measured configuration-lock wait.

The Files-app scenario also resets native state, restoring public network defaults. The harness reapplies its local fixture settings at the subsequent stopped-app boundary, preserving the owner profile and roster, before testing reverse approval.

The iOS integration harness now uses the existing real local Blossom protocol fixture and local relay for fresh test-owned profiles, configured before their app or daemon starts. An actual CLI approval through these fixtures completed in 0.1611 seconds, publishing all six approval events and the root, and uploading all three reported blocks. HTTP reads verified the exact announced root ciphertext and fixture block bytes against their SHA-256 hashes. The corrected CLI-owner-to-iOS GUI flow completed the command, observed authorization, and observed the owner's persisted acknowledgement in less than one second under the unchanged 15-second budget. The reverse universal-link and manual approval flows also satisfied that budget; manual receiver authorization was observed in approximately 4.82 seconds, with acknowledgement verified by the owner's durable audit. These directions were measured separately.

These measurements establish a deterministic local integration check; they do not establish a production latency percentile or a production upload speedup. The normal upload-before-publication barrier, event/root/upload-report checks, real native UI, and acknowledgement requirement remain. The fixture tests HTTP storage behavior and bytes, not TLS or upload-auth signature validation; public release checks remain separate. Root completeness is reported by the production upload traversal, while the fixture checks bind the announced root to actual readable storage; aggregate blob counts are not an independent closure proof.

### Native idle CPU investigation (release candidate 0.1.35)

The iOS simulator's ordinary idle gate failed at 6.53% average CPU, then failed at 6.33% after a controlled fresh-process launch with the same retained profile. Both used the original 30-second warmup, 60-second measurement, twelve samples and 5% limit. A separate short profiling observation of 3.69% was diagnostic only and did not authorize release.

The retained authorized owner had no pending approval receipts or outbound approval acknowledgment work. Its stopped test relay and storage fixtures caused a background roster-publication failure about once per second. A complete-window profile captured 80 such failures over 79.104 seconds. The profiled CPU result was 5.29%; it is attribution evidence rather than a release gate.

Restoring the same real local services at their original addresses, without changing the profile or network settings, allowed all six original roster events to publish and eliminated failed ticks throughout warmup and measurement. The unprofiled normal gate nevertheless failed at 5.84% average CPU (10.86% peak). This establishes the fixture lifetime issue but does not establish it as the cause of the CPU threshold failure. The profiled and unprofiled trials do not quantify CPU savings from removing retries.

The complete trace showed distributed mesh session, dataplane, discovery and scheduler work. No UDP hard-error busy loop was established. Status snapshots were not time aligned with the measurement windows; the offline snapshot predates its measurement. They cannot establish measured traffic rates or a traffic difference between trials. Unchanged settings alone do not prove identical live network activity. The one-second retry of unchanged offline background work is independently inconsistent with the existing fifteen-second idle schedule, but any correction must preserve fast pending approval/ACK work and immediate user-action wakeups.

The background error schedule was corrected to preserve the existing pending-work classification: an idle authorized owner retries after fifteen seconds, while awaiting approval, pending receipts, an undelivered acknowledgment or unknown state keeps the fast one-second cadence. User actions still wake the exchange immediately, and publication barriers and durable tracking are unchanged. A production-outcome regression failed at 1,000 ms versus the expected 15,000 ms before the fix; all eight focused idle scheduling tests passed afterward.

The gate also exposed a build mismatch: the functional simulator app used Swift Debug and Rust Debug. The release gate now builds and installs matching optimized Swift and Rust profiles, with the simulator architecture aligned, before any CPU sampler starts. Debug functional tests retain their required hooks. With the unchanged retained profile and network settings, the optimized simulator app passed at 1.17% average CPU and 1.78% peak over twelve samples. The 30-second warmup, 60-second window and 5% limit are unchanged. The installed binary matches the build; production audit writes bind the run to the retained shared profile and show the corrected fifteen-second error cadence. The optimized build and retry fix were both present, so this experiment does not isolate their individual CPU effects. The simulator build is distinct from the signed device IPA.

Local fixtures exercise real relay/storage protocol and content hashes; they do not validate public TLS or upload-auth signatures.

### Android approval during native startup (release candidate 0.1.35)

The existing test for a device-approval link arriving during native startup exposed a path that queued the request until native initialization completed. The ordinary cold-start link path already displayed its confirmation before native initialization. The same early-prompt helper now handles a new link while startup is pending, preserving validation, explicit approval, cancellation, deduplication and deferred native dispatch.

The existing test was strengthened rather than duplicated: with a three-second injected native-start delay, it requires the prompt within one second of the actual new-intent callback, then verifies explicit approval and the resulting device roster. The old path failed the immediate-prompt assertion; the corrected path passed. Restoring the original ActivityScenario intent in a finally block also removed an unrelated forty-five-second failure-time teardown caused by the injected intent.

The final Android link and provider-sync check passed its unchanged fifteen-second authorization budget. The normal Android idle gate then passed at 2.94% average CPU with the original ninety-second warmup, sixty-second measurement, five-second interval and 5% limit. It collected eleven samples because polling takes time. The unsupported exact-count assertion was rejected; the successful production measurement and original failed private-wrapper evidence were retained without a repeat. This checks the instrumented Kotlin test shell with optimized Rust. The native library bytes matched the signed APK and app bundle, but this does not make the test shell identical to the signed production application.

## 2026-09-05 deterministic fixture waits

- Replaced fixed sleeps in the config-lock, keyed-task completion, and
  recursive file-rescan tests with explicit pending/completion checks and
  controlled filesystem modification times. No tests were removed.
- Five interleaved runs of the old and new CLI test binaries, running those
  three tests serially, reduced median wall time from 0.1212s to 0.0394s
  (67.5%). Median test-body time fell from 0.11s to 0.03s; these figures do not
  include compilation or claim a whole-suite speedup.
- Consolidated the release-workflow barrier fixture and replaced repeated
  `find`/`wc`/`tr`/`seq` processes with shell builtins. All 12 workflow tests
  pass; interleaved measurements did not establish a wall-time improvement.

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
