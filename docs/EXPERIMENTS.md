# Experiments

Performance and integration experiments log. Omit identifying information
(pubkeys, secrets, IPs, private hostnames, exact repo names, raw hashes)
unless the user explicitly asks otherwise.

## 2026-09-11 Android embedded gateway startup

An authorized Android profile had connected FIPS peers while its embedded
gateway failed to start. The social-graph transaction lock returned
`try_lock() not supported` with the pinned Rust toolchain. Healthy mesh status
alone therefore did not establish that the configured app services worked.

The existing Android native-state smoke class now creates a real authorized
profile with its resolver enabled, requires an error-free running embedded
gateway, and makes an actual loopback HTTP request. It checks the exact 307
redirect and portal location. The unchanged test APK failed against the old
native library in 1.217 seconds and passed against the Hashtree Android lock
correction in 1.607 seconds. Both runs stopped the app and removed only the
owned test package; the retained main profile's regular file contents matched
before and after. The native library packaged in the candidate APK matched the
compiled output exactly. This verifies startup and local HTTP, not WebView
content loading or final release acceptance.

The network policy also now permits the exact loopback address used by the
gateway readiness request. The existing hostname exceptions do not cover that
numeric address on older Android versions. The smoke test checks the effective
policy, but the device run does not establish coverage of every Android API.

The first candidate build exceeded its 512 MiB temporary-storage allowance and
was stopped with source and lockfile restored. A warm continuation passed with
a separately admitted 1 GiB allowance and the same free-space reserve. These
are build-resource results, not app CPU or bandwidth improvements. Idle app
acceptance must require the configured gateway to be healthy.

An early idle check of the corrected native candidate required that configured
gateway and at least one connected mesh peer throughout. After 90 seconds of
warmup, two 60-second windows averaged 3.77% and 3.92% CPU, below the unchanged
5% limit; their peaks were 6.93% and 5.97%. The app was stopped afterwards.
These unprofiled windows establish acceptable idle behavior for this working
candidate, not a causal CPU reduction or final shipping-artifact acceptance.

## 2026-09-11 Windows renderer compatibility

A self-contained Windows release app in a VM responded to its native GUI
assertions but displayed a blank white window. Captures verified the exact
launched process and window, foreground ownership, visible controls and their
accessibility bounds. Both screen-region and window captures were blank.

A process-only WPF software-rendering intervention made the unchanged app's
controls and layout visible. A matched startup-hook control performed the same
early loading and readback while keeping the default renderer; it remained
blank. The older running app and the release app used identical .NET 8.0.26
rendering binaries, and the UI markup was unchanged. This supports a rendering
path compatibility issue in this environment, without identifying a particular
driver or establishing a source regression. No render tier or graphics-device
error was recorded.

`IRIS_DRIVE_WINDOWS_SOFTWARE_RENDERING` provides an explicit process-only
compatibility option. It does not automatically change the renderer or global
graphics settings. The default graphics path still failed in this VM; software
rendering is a separate configuration, not a default-path pass. These short
functional captures do not establish CPU, bandwidth, or battery performance.

Validation built the WPF shell in Release mode and ran the existing Windows
profile tests against the real WPF process API. Enabled and disabled flags,
unchanged existing preferences, and profile pipe isolation passed. A fresh
owned GUI profile then passed the production UI assertions and rendered with
the product flag, with no startup hook. Its captured process matched the
software-rendering startup trace. This used a verification-only shell assembly
with the retained native core and runtime; it was not a packaged release test.
The fixture disabled peer discovery and configured no relay or blob servers;
the native provider was disabled. It does not establish mesh, provider, or
resource acceptance.

## 2026-09-10 unavailable service retry cost

The Android idle gate found excessive CPU with an authorized retained profile,
two connected WebSocket mesh peers, and unavailable test-service settings. The
Kotlin test shell used optimized Rust. Its original long-running process averaged
15.19% CPU. Restarting the identical APK with the same configuration and identity
still averaged 14.51%, above the unchanged 5% limit. Both measurements used the
normal 90-second warmup and 60-second window. Stored settings alone were not used
as proof of live connectivity: the control retained the same FIPS endpoint and
connected peer set.

A full-client transport fixture reproduced repeated connection attempts to a
service that rejected or immediately closed connections while its authenticated
FIPS link stayed stable. The original periodic path attempted seven connections
in 1.3 seconds; publication activity produced 27 accepted-and-closed streams in
the same interval. Healthy simultaneous connections remained stable, and retained
subscriptions recovered after the service became available. This was a service
retry defect; it did not establish that the peer was unsuitable for mesh transit.

A first correction spaced service attempts by at least one second through both
paths. It reduced each fixture to two attempts in 1.3 seconds. An in-place Android
update preserved the identity, configuration, endpoint and connected peers, and
reduced average CPU from the restarted control's 14.51% to 5.95%. That was a 58.99%
reduction, but the ordinary gate still failed. The original APK, both controls and
the failed candidate remained retained, and release stayed held.

Short native profiles used user-CPU sampling only; their symbol shares do not
account for kernel time in the total CPU gate. They identified connection work,
peer selection, identity conversion and some repeated authorization projection,
but did not establish that any one remaining helper alone explained the excess.
A second combined candidate deferred identity validation until a service attempt
was admitted, spaced repeated failed attempts by three seconds, and skipped
pending-receipt verification when the current roster already supplied usable
authorization. Tests kept first connections and established sends immediate,
retained the existing five-second late-service recovery deadline, and verified
that a retained approval receipt could not override a later signed revocation.

The same in-place update and unchanged 90-second warmup / 60-second Android gate
averaged 5.44% CPU, with the same stored identity/configuration and live endpoint
and connected peers. This was a 62.51% reduction from the matched restarted
control, but still exceeded 5%. The measurement supports only the combined
change; no separate saving is attributed to either small simplification. Release
remained held, and the prior artifacts and failed results were preserved.

A later 30-second diagnostic requested total task-clock sampling without changing
device permissions. Although its event attributes admitted kernel samples, all
773 retained samples were marked user mode and kernel symbols were unavailable.
It did not resolve kernel attribution. The separately reported
device run was conservatively proven reaped more than 36 seconds before the
profile began. The profile still is not an unprofiled CPU gate or evidence that
the fixed budget has been met.



A third combined candidate kept the prior changes, borrowed peer-selection
lookup keys, and sized initial cryptographic-run allocations from the admitted
batch. Production dispatch tests measured singleton reserved item storage falling
from 101,376 to 792 bytes on the test host. Full 128-packet runs kept the same
capacity, and a nine-packet fairness continuation grew safely while completing
cryptography and retiring plaintext in order.

The source-bound Android UiTest candidate passed the unchanged gate at 4.94%
average CPU (unrounded 4.9444%), with a 9.98% peak across 11 intervals. The prior
candidate's exact endpoint and peers matched before the update and after the
sample; stored configuration and identity remained unchanged. The helper retained
raw process ticks, clock frequency and device uptime, allowing the unchanged
unweighted interval mean to be independently recomputed. Connectivity was checked
before and after, rather than continuously traced.

The pass has only about 0.056 CPU percentage points of headroom. It supports the
combined candidate under this measured configuration, without attributing an
individual CPU saving to either allocation change or guaranteeing wider
headroom. All earlier failures remain retained. The candidate uses private
normalized patches and is not a shipping release; final published dependency
adoption and required product gates remain before distribution.

## 2026-09-09 desktop idle measurement continuity

Desktop idle samplers could accept a required process disappearing or restarting
after an early observation. POSIX also clamped a decreasing CPU counter to zero,
while Windows could treat absent or null counter data as zero. Controlled inputs
against the actual script entry points reproduced six POSIX false passes (three
cases on Linux and macOS) and four Windows false passes. The fixed samplers require
the original required process set throughout the window and reject missing or
reset readings. Stable idle still passes and excessive CPU still fails. Existing
thresholds, sampling durations and optional-role semantics are unchanged.

## 2026-09-09 routed native mesh

- Drive's existing authorized peer roster now supplies bounded identity hints
  to the shared Nostr pubsub client. A transport-only intermediary remains
  outside the application roster. The fixture calls the production event and
  immutable blob APIs; it adds no alternate protocol or authorization path.
- A three-process candidate experiment connected Chat and Drive only through
  a Hashtree intermediary with Nostr disabled. Public relays and LAN discovery
  were disabled. Four signed events and two 192 KiB blobs arrived with exact
  content hashes, including events queued while the intermediary was stopped.
  Three repeated runs passed; median recovery of the original queued event
  IDs was 12.943 seconds after restart. This measures known peers with an
  explicit private bootstrap address, not unknown-peer discovery.
- A controlled comparison used the same fixture binaries and changed only the
  shared pubsub peer budget. A budget of one used 2,627/2,788 bytes per second
  before/after the partition; the ordinary budget of 64 used 2,628/2,833.
  CPU stayed between 0.47% and 1.13% per process. Both passed the experiment's
  5% CPU and 4 KiB/s idle guards. There was no meaningful bandwidth benefit
  from excluding the intermediary from pubsub connection attempts, so the
  production peer budget remains unchanged. These are local debug-process
  observations, not mobile battery measurements or production percentiles.

## 2026-09-08 published mesh dependency integration

- Drive now consumes the published transport retry-fairness correction and
  embedded CLI security fixes. Required companion dependency floors were
  aligned together, preserving the existing transport and storage features.
  Linux's direct FUSE dependency selects the published compatibility carrier,
  which retains checked Finder timestamps and backports initialized libfuse3
  session callbacks. The old FUSE package is absent from the resolved graph.
- On that exact graph, the real Drive hop-limit, provider-failure fallback and
  inbound authorization tests passed in 0.41, 0.46 and 0.09 seconds. All 213 CLI
  binary unit tests passed in 4.44 seconds, including missed-root recovery.
  Strict Clippy checks passed for the shared core, app core and CLI.
- An initial command sequence passed the three integrations, then stopped
  because its CLI command selected a nonexistent library target. That failed
  invocation is retained as harness evidence. The corrected command selected
  the actual binary target and ran only the remaining unit and lint checks;
  this continuation took 85.207 seconds including compilation. Source and lock
  bytes were unchanged across both runs.
- These checks validate the combined published dependency graph and existing
  security boundaries. They do not identify the original cause of the earlier
  multi-peer missing-child timeout or establish a CPU or throughput improvement.

## 2026-09-08 blob forwarding hop limits

- A bounded source review found that Drive's inbound blob service used its
  full read router, including a peer route that preserved the received hop
  budget. A local miss could therefore circulate between peers without
  consuming a hop. The existing mesh-forwarding adapter now wraps only that
  peer route; local and shared-store routes remain terminal, and authorization,
  provider limits and download deadlines remain unchanged.
- A regression uses two real Drive blob services and a real TCP/FIPS reader
  over isolated local endpoints. Before the fix, a zero-hop request returned
  remote data instead of a miss; this valid failing run took 5.92 seconds.
  An earlier invocation failed the peer-connection setup before reaching the
  assertions and is not evidence of the security defect.
- The corrected regression passed in 0.41 seconds: zero-hop local reads work,
  zero-hop misses do not query another peer, one-hop reads return and cache
  valid remote bytes, and a missing child takes a finite two-hop path without
  repeated fan-out. A subsequent valid read still succeeds. Existing real
  provider-failure fallback and inbound authorization tests also passed in
  0.50 and 0.09 seconds. Each run cleaned up its owned transports and endpoints.
- This establishes the forwarding boundary, not a CPU or saturation result.
  It does not establish that forwarding loops caused the separately observed
  multi-peer download timeout for a common missing child.

## 2026-09-08 missed desktop root update recovery

- A five-peer run passed both desktop approval directions and the earlier
  file and restart checks, then failed its unchanged 300-second baseline
  convergence limit. Three peers had the new file; two retained the previous
  source root. The actual concurrent writes had not started. Retained logs
  showed the newer signed announcement on a healthy peer and no corresponding
  received frame on a stale peer. The initial delivery loss was not isolated.
- An incidental announcement could consume the reconnect signal before the
  peer-refresh task requested current state. Also, the existing five-minute
  repair task skipped peers whose older known roots were already complete.
  Recovery now stays pending until a successful request send, and the same
  five-minute task reconciles complete known roots too. The 30-second peer
  refresh, 10-second request throttle and valid older data remain unchanged.
- Three focused regressions failed before the correction and passed afterward.
  The data-path fixture retains a hash-verified old tree, deliberately withholds
  the next announcement, then exercises the production request codec, fresh
  signed reply, recipient-key unwrap and newer-sequence selection. It reads the
  new bytes, retains the old bytes and rejects replay of the older root.
  Transport delivery and the approved roster are supplied by the fixture.
- All 213 CLI unit tests and strict CLI Clippy checks passed. A separate real
  daemon reconnect test executed both sender and receiver restarts with its
  default 4/8/8 file counts and passed in 41.27 seconds. This complements the
  deterministic recovery regression; it does not inject packet loss or measure
  a CPU or throughput improvement. A prior invocation selected zero tests and
  was rejected as validation evidence.

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
- A later synchronized run kept all five daemons alive but still missed the
  same authorization deadline. The GTK smoke accepted a receipt queued for an
  earlier device before its selected approval callback had completed. The
  check now requires the selected AppKey and key wrap in the durable roster,
  plus the completion notice from its newly launched GUI process, before
  cleanup. Acknowledgment may already have drained the selected receipt.
  Three focused tests passed in 0.740 seconds, including stale receipts,
  missing or wrong targets, incomplete actions and another GUI's notice.
  The native authorization/ACK and direct-mesh gates remain unchanged.
- The corrected primary approval passed in 6.497 seconds. Reverse approval
  satisfied its checks at 16.132 seconds and correctly failed the unchanged
  15-second deadline. The approval-only Windows check now omits the full
  navigation journey already covered by the general GUI smoke, removing its
  three fixed 500 ms waits. Reverse readiness reuses the successful Windows
  status from the same poll for its direct-peer assertion. Fifteen focused
  status cases passed; these changes do not yet establish a native gate pass.

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

## Physical iOS debug probe completion deadline

The instrumented physical iOS probe installed and launched, but no fresh result file appeared within its existing host polling window. A later bounded read confirmed that the debug action had created its isolated profile and the app was still running. The initial cleanup assertion incorrectly matched an undecoded device executable URL; the raw receipt was preserved, rejected, and superseded by an exact decoded-path/PID cleanup check. These observations do not identify the WebKit or gateway operation that stalled.

Source review found that the debug WebView timeout itself awaited JavaScript and screenshot callbacks, so it could not guarantee completion when a callback remained pending. The debug probe now uses its configured 30-second application budget once for the complete result, retaining the host's existing 10-second copy grace. Expiry synchronously publishes one failed result, cancels the work, and closes the WebView. Late callbacks cannot publish a second result, reload the page, or save a screenshot. Fully completed results before the deadline retain the existing content checks. A page that becomes inspectable only at the former later WebView timeout is now a timeout failure; the deadline is not extended to salvage that partial result.

The existing gateway XCTest class gained four focused cases covering a withheld capture callback, repeated expiry and late completion, a valid completion before expiry, and the configured timer. Against the extracted no-overall-deadline behavior, eight tests ran and three new cases failed (six assertions); the guarded run took 39.085 seconds. The initial implementation passed all eight cases in a 10.329-second guarded run. After adding the late-callback side-effect guards, all eight cases passed again in 22.311 seconds; the test assertions themselves took 0.034 seconds. Both successful guards verified process cleanup. This verifies the completion mechanism used by the debug probe, not reproduction of the physical device's original stall. All runtime edits are within existing DEBUG sections; shipping source outside those sections, Rust dependencies, versions, and the signed release artifacts are unchanged.

## Five-daemon fixture storage and watchdog cleanup

The release workload used five daemon roles: Linux, Windows and macOS guests plus two local macOS CLI fixtures. An initial run completed manual approval but failed reverse-link readiness. The two local CLI fixtures reported a generic direct-FIPS startup error; their inner error chains were not retained. A separate read-only investigation found an invalid semaphore reference in the existing shared local storage environment. Using the supported data-directory selector, both local fixtures were moved to one fresh shared pool inside the last local role's owned temporary base. Normal configuration and identity selection stayed unchanged; the existing storage and its holder were preserved. This establishes a test setup correction, not the discarded error's exact cause.

With the isolated pool, both desktop approval directions, direct FIPS and durable acknowledgements passed. The reverse approval completed in 13.587 seconds within the existing 15-second bound. Merge, create/edit/rename/delete, nested changes, file-type replacement, restart recovery, concurrent edits, many small files, the large-file transfer and the final online roster all passed. Delete propagation took 23.081 seconds under the unchanged convergence budget. After the original 180-second settling period, each daemon provided twelve samples over 60 seconds. Average CPU values were 3.33%, 7.02%, 5.01%, 4.81% and 3.33%, below the existing 10% average limit. The two local roles are desktop CLI fixtures; these measurements do not substitute for physical mobile evidence.

The workload child exited successfully, but its outer guard returned 125 after 818.766 seconds because a process group remained. That raw result is retained. The guard reaped the group without escalation, and a separate 1.506-second scoped audit confirmed that all owned workload processes, directories and mounts were absent. The fresh shared pool's own absence receipt also passed.

A focused check of the actual timeout helper reproduced an orphaned watchdog sleep after a successful command, a command exiting with status 23, and a timeout. The failing guard took 2.038 seconds. The watchdog now terminates and reaps its own timer jobs while preserving command status, timeout escalation and prompt completion. The same three cases then passed in a 1.844-second guarded run with no survivors, and an unrelated sibling remained alive. The original full-run guard did not retain the leftover member's identity, so this is a proven cleanup defect and correction rather than retrospective proof of that exact member. The passed workload and idle measurements were not repeated for teardown.

The portable harness selects the last owned POSIX local role for its shared pool, applies the selector only to local child commands, and leaves host layouts without local roles unchanged. Four focused helper cases, shell syntax and diff checks passed in a separate 0.179-second guard. Only test scripts and this experiment record changed; the frozen shipping runtime and package provenance remain unchanged.
