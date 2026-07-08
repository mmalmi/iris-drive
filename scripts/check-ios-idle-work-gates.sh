#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT" <<'PY'
from pathlib import Path
import re
import sys

root = Path(sys.argv[1])
app = (root / "ios/Sources/IrisDriveIOSApp.swift").read_text(encoding="utf-8")
model = (root / "ios/Sources/IrisDriveMobileModel.swift").read_text(encoding="utf-8")
root_view = (root / "ios/Sources/IrisDriveRootView.swift").read_text(encoding="utf-8")
background = (root / "ios/Sources/IrisDriveBackgroundSync.swift").read_text(encoding="utf-8")

failures = []

if "model.startForegroundSyncLoop()" in app:
    failures.append("IrisDriveIOSApp must not start foreground sync directly from view appearance.")

if "reconcileForegroundWork(isActive:" not in app:
    failures.append("IrisDriveIOSApp should route scene changes through reconcileForegroundWork(isActive:).")

if "private var shouldRunForegroundWork" not in model:
    failures.append("IrisDriveMobileModel needs a single foreground-work predicate.")

if "private var shouldRunDriveForegroundRefresh" not in model:
    failures.append("iOS foreground work must keep refreshing authorized drive state while sync is paused.")

foreground_work_match = re.search(
    r"private var shouldRunForegroundWork: Bool \{(?P<body>.*?)\n    \}",
    model,
    flags=re.S,
)
if not foreground_work_match:
    failures.append("shouldRunForegroundWork was not found.")
elif "shouldRunDriveForegroundRefresh" not in foreground_work_match.group("body"):
    failures.append("shouldRunForegroundWork must include paused foreground drive refreshes.")

start_match = re.search(
    r"func startForegroundSyncLoop\(\) \{(?P<body>.*?)\n    func stopForegroundSyncLoop\(\)",
    model,
    flags=re.S,
)
if not start_match:
    failures.append("IrisDriveMobileModel.startForegroundSyncLoop was not found.")
else:
    start_body = start_match.group("body")
    if "guard shouldRunForegroundWork else" not in start_body:
        failures.append("startForegroundSyncLoop must return without scheduling work when logged out/idle.")
    if "UIApplication.shared.applicationState == .active" not in start_body:
        failures.append("startForegroundSyncLoop must be gated to active scenes.")

if "reconcileForegroundWorkIfAppActive()" not in model:
    failures.append("state changes must re-check whether foreground work should start or stop.")

if "foregroundDriveSyncMinimumIntervalSeconds" not in background:
    failures.append("iOS foreground drive sync needs a separate minimum interval from UI status refresh.")
else:
    interval_match = re.search(
        r"foregroundDriveSyncMinimumIntervalSeconds\s*:\s*TimeInterval\s*=\s*(?P<value>[0-9_]+)",
        background,
    )
    if not interval_match:
        failures.append("foregroundDriveSyncMinimumIntervalSeconds must be a numeric TimeInterval constant.")
    elif int(interval_match.group("value").replace("_", "")) < 30:
        failures.append("foreground drive sync should not run more often than every 30 seconds.")

foreground_interval_match = re.search(
    r"foregroundSyncIntervalNanoseconds\s*:\s*UInt64\s*=\s*(?P<value>[0-9_]+)",
    background,
)
if not foreground_interval_match:
    failures.append("iOS foreground status refresh interval was not found.")
elif int(foreground_interval_match.group("value").replace("_", "")) < 30_000_000_000:
    failures.append("iOS foreground status refresh should not poll native state more often than every 30 seconds.")

sync_once_match = re.search(
    r"private func syncOnceIfRunning\(\) async \{(?P<body>.*?)\n    private var foregroundSyncDelayNanoseconds",
    model,
    flags=re.S,
)
if not sync_once_match:
    failures.append("syncOnceIfRunning was not found.")
else:
    sync_once_body = sync_once_match.group("body")
    if "foregroundDriveSyncIsDue()" not in sync_once_body:
        failures.append("syncOnceIfRunning must throttle full foreground drive sync attempts.")
    if 'await refreshInBackground()' not in sync_once_body:
        failures.append("foreground throttle should still refresh visible status between full sync attempts.")
    if "if !isRevoked, isSetupComplete" not in sync_once_body:
        failures.append("authorized foreground sessions must refresh visible state even when sync is paused.")
    if "if syncRunning, foregroundDriveSyncIsDue()" not in sync_once_body:
        failures.append("foreground drive sync throttle must only gate native sync, not visible refresh.")

approve_match = re.search(
    r"func approveDevice\(request: String, label: String\) \{(?P<body>.*?)\n    func rejectDevice",
    model,
    flags=re.S,
)
if not approve_match:
    failures.append("approveDevice(request:label:) was not found.")
else:
    approve_body = approve_match.group("body")
    for token in (
        'dispatch([',
        'scheduleBackgroundSyncIfNeeded()',
        'lastForegroundDriveSyncStartedAt = Date.distantPast',
        'syncOnceIfRunning()',
        'startSync()',
        'recordConfigMutation(action: "approve_device"',
    ):
        if token not in approve_body:
            failures.append(f"iOS approveDevice must wake approval publishing; missing: {token}")

setup_match = re.search(
    r"private struct SetupWelcomeView: View \{(?P<body>.*?)\nprivate struct ",
    root_view,
    flags=re.S,
)
if not setup_match:
    failures.append("SetupWelcomeView was not found.")
else:
    setup_body = setup_match.group("body")
    blocked = [
        token for token in (
            "while ",
            "Task.sleep",
            "refreshProfileStatusInBackground",
            "refreshInBackground",
            "startForegroundSyncLoop",
        )
        if token in setup_body
    ]
    if blocked:
        failures.append(
            "SetupWelcomeView must stay passive while logged out; found: "
            + ", ".join(blocked)
        )

if failures:
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    sys.exit(1)

print("IOS_IDLE_WORK_GATES_OK")
PY
