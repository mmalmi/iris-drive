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

failures = []

if "model.startForegroundSyncLoop()" in app:
    failures.append("IrisDriveIOSApp must not start foreground sync directly from view appearance.")

if "reconcileForegroundWork(isActive:" not in app:
    failures.append("IrisDriveIOSApp should route scene changes through reconcileForegroundWork(isActive:).")

if "private var shouldRunForegroundWork" not in model:
    failures.append("IrisDriveMobileModel needs a single foreground-work predicate.")

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
