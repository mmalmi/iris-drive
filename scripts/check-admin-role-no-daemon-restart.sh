#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT" <<'PY'
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])


def read(path: str) -> str:
    return (root / path).read_text(encoding="utf-8")


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    sys.exit(1)


linux = read("linux/src/render.rs")
for label, notice in [
    ("make-admin", "Device made admin"),
    ("remove-admin", "Admin removed"),
]:
    block_match = re.search(
        rf"connect_clicked\(move \|_\| match .*?\{{(?P<body>.*?)model\.ui\.notice\.set_text\(\"{re.escape(notice)}\"\);",
        linux,
        re.S,
    )
    if not block_match:
        fail(f"missing Linux {label} admin action block")
    if "restart_daemon" in block_match.group("body"):
        fail(f"Linux {label} admin action must not restart the daemon")

macos = read("macos/Sources/IrisDriveMacApp.swift")
role_match = re.search(
    r"private func setDeviceAdminRole\(_ device: String, makeAdmin: Bool\) \{(?P<body>.*?)\n    func createShare",
    macos,
    re.S,
)
if not role_match:
    fail("missing macOS setDeviceAdminRole block")
if "restartSyncAfterSuccess: true" in role_match.group("body"):
    fail("macOS admin role changes must not restart sync")

windows = read("windows/MainWindowDevices.cs")
for method in ["AppointAdmin_Click", "DemoteAdmin_Click"]:
    method_match = re.search(
        rf"private async void {method}\(.*?\n    \{{(?P<body>.*?)\n    \}}\n",
        windows,
        re.S,
    )
    if not method_match:
        fail(f"missing Windows {method} block")
    body = method_match.group("body")
    if "StopDaemon()" in body or "EnsureDaemonRunning" in body:
        fail(f"Windows {method} must not restart the daemon")

print("ADMIN_ROLE_NO_DAEMON_RESTART_OK")
PY
