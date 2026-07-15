#!/usr/bin/env python3
"""Reserve native test resources and classify infrastructure failures."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

INFRASTRUCTURE_UNAVAILABLE = 75
RESERVED_HEALTH_KINDS = {"android", "ios-device", "ios-simulator", "local", "ssh"}


def run_probe(command: List[str], timeout: int = 15) -> Tuple[bool, str]:
    try:
        completed = subprocess.run(
            command,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return False, str(error)
    output = completed.stdout.strip()
    return completed.returncode == 0, output


def check_health(spec: str) -> Dict[str, Any]:
    kind, separator, value = spec.partition(":")
    if not separator or not value:
        return {"spec": spec, "available": False, "detail": "expected kind:value"}

    if kind == "command":
        path = shutil.which(value)
        return {"spec": spec, "available": bool(path), "allocation": path or ""}

    if kind == "path":
        path = Path(value).expanduser()
        return {"spec": spec, "available": path.exists(), "allocation": str(path)}

    if kind == "env":
        allocation = os.environ.get(value, "")
        return {"spec": spec, "available": bool(allocation), "allocation": allocation}

    if kind == "local":
        expected = value.lower()
        actual = platform.system().lower()
        aliases = {"macos": "darwin", "windows": "windows", "linux": "linux"}
        return {
            "spec": spec,
            "available": actual == aliases.get(expected, expected),
            "allocation": actual,
        }

    if kind == "docker":
        ok, detail = run_probe(["docker", "info", "--format", "{{.ServerVersion}}"])
        return {
            "spec": spec,
            "available": ok,
            "allocation": value,
            "detail": detail[-1000:],
        }

    if kind == "ssh":
        ok, detail = run_probe(
            [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                value,
                "exit 0",
            ],
            timeout=10,
        )
        return {"spec": spec, "available": ok, "allocation": value, "detail": detail[-1000:]}

    if kind == "ios-simulator":
        if platform.system() != "Darwin" or not shutil.which("xcrun"):
            return {"spec": spec, "available": False, "detail": "xcrun requires macOS"}
        ok, output = run_probe(["xcrun", "simctl", "list", "devices", "available", "--json"])
        if not ok:
            return {"spec": spec, "available": False, "detail": output}
        try:
            devices = [
                device
                for runtime_devices in json.loads(output).get("devices", {}).values()
                for device in runtime_devices
                if device.get("isAvailable") and "iPhone" in device.get("name", "")
            ]
        except (TypeError, ValueError) as error:
            return {"spec": spec, "available": False, "detail": str(error)}
        if value == "auto":
            matches = sorted(devices, key=lambda item: item.get("state") != "Booted")
        else:
            matches = [item for item in devices if value in (item.get("udid"), item.get("name"))]
        if not matches:
            return {"spec": spec, "available": False, "detail": "simulator not found"}
        selected = matches[0]
        return {
            "spec": spec,
            "available": True,
            "allocation": selected.get("udid", ""),
            "name": selected.get("name", ""),
            "state": selected.get("state", ""),
        }

    if kind == "android":
        if not shutil.which("adb"):
            return {"spec": spec, "available": False, "detail": "adb not found"}
        ok, output = run_probe(["adb", "devices"])
        if not ok:
            return {"spec": spec, "available": False, "detail": output}
        devices = [
            line.split()[0]
            for line in output.splitlines()[1:]
            if len(line.split()) >= 2 and line.split()[1] == "device"
        ]
        matches = devices if value == "auto" else [serial for serial in devices if serial == value]
        return {
            "spec": spec,
            "available": bool(matches),
            "allocation": matches[0] if matches else "",
            "detail": "" if matches else "no authorized Android device",
        }

    if kind == "ios-device":
        if platform.system() != "Darwin" or not shutil.which("xcrun"):
            return {"spec": spec, "available": False, "detail": "devicectl requires macOS"}
        ok, output = run_probe(["xcrun", "devicectl", "list", "devices"], timeout=20)
        if not ok:
            return {"spec": spec, "available": False, "detail": output}
        devices = []
        for line in output.splitlines()[2:]:
            columns = [column.strip() for column in line.split("  ") if column.strip()]
            if len(columns) >= 4 and columns[3].startswith("available"):
                devices.append({"name": columns[0], "identifier": columns[2], "state": columns[3]})
        matches = (
            devices
            if value == "auto"
            else [device for device in devices if value in (device["name"], device["identifier"])]
        )
        return {
            "spec": spec,
            "available": bool(matches),
            "allocation": matches[0]["identifier"] if matches else "",
            "name": matches[0]["name"] if matches else "",
            "detail": "" if matches else "no paired available iOS device",
        }

    return {"spec": spec, "available": False, "detail": "unknown health check kind"}


def health_report(specs: List[str]) -> List[Dict[str, Any]]:
    return [check_health(spec) for spec in specs]


def health_resource(check: Dict[str, Any]) -> Optional[str]:
    kind = str(check.get("spec", "")).partition(":")[0]
    allocation = str(check.get("allocation", ""))
    if not check.get("available") or kind not in RESERVED_HEALTH_KINDS or not allocation:
        return None
    if kind == "local":
        return f"host:local:{socket.gethostname()}"
    if kind == "ssh":
        return f"host:ssh:{allocation}"
    return f"{kind}:{allocation}"


def allocation_environment(
    checks: List[Dict[str, Any]], mappings: List[str]
) -> Dict[str, str]:
    environment = os.environ.copy()
    for mapping in mappings:
        kind, separator, variable = mapping.partition("=")
        if not separator or not kind or not variable.isidentifier():
            raise ValueError(f"invalid --allocation-env mapping: {mapping}")
        matches = [
            str(check.get("allocation", ""))
            for check in checks
            if str(check.get("spec", "")).partition(":")[0] == kind
            and check.get("available")
            and check.get("allocation")
        ]
        if len(matches) != 1:
            raise ValueError(
                f"--allocation-env {mapping} requires exactly one available {kind} health check"
            )
        environment[variable] = matches[0]
    return environment


def state_root() -> Path:
    configured = os.environ.get("IRIS_NATIVE_LAB_STATE_DIR")
    return Path(configured) if configured else Path(tempfile.gettempdir()) / "iris-native-lab"


def lock_path(resource: str) -> Path:
    digest = hashlib.sha256(resource.encode("utf-8")).hexdigest()[:12]
    readable = "".join(character if character.isalnum() else "-" for character in resource)[:40]
    return state_root() / "locks" / f"{readable}-{digest}"


def process_is_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    return True


def acquire(resource: str, stale_after: int) -> Tuple[Optional[Path], Optional[Dict[str, Any]]]:
    path = lock_path(resource)
    path.parent.mkdir(parents=True, exist_ok=True)
    owner_path = path / "owner.json"
    for _ in range(2):
        try:
            path.mkdir()
            owner = {
                "resource": resource,
                "pid": os.getpid(),
                "host": socket.gethostname(),
                "started_at": dt.datetime.now(dt.timezone.utc).isoformat(),
            }
            owner_path.write_text(json.dumps(owner, sort_keys=True) + "\n", encoding="utf-8")
            return path, None
        except FileExistsError:
            try:
                owner = json.loads(owner_path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                owner = {}
            age = time.time() - path.stat().st_mtime
            local_owner = owner.get("host") == socket.gethostname()
            stale = age >= stale_after or (local_owner and not process_is_alive(int(owner.get("pid", 0))))
            if stale:
                shutil.rmtree(path, ignore_errors=True)
                continue
            return None, owner
    return None, {"resource": resource, "detail": "could not acquire resource"}


def release(path: Optional[Path]) -> None:
    if path is not None:
        shutil.rmtree(path, ignore_errors=True)


def write_report(report: Dict[str, Any], result_path: Optional[str]) -> None:
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if result_path:
        destination = Path(result_path)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(rendered, encoding="utf-8")
    print(rendered, end="")


def run_command(args: argparse.Namespace) -> int:
    started = time.monotonic()
    lock, owner = acquire(args.resource, args.stale_after)
    if lock is None:
        write_report(
            {
                "status": "infrastructure_unavailable",
                "category": "resource_busy",
                "resource": args.resource,
                "owner": owner or {},
                "exit_code": INFRASTRUCTURE_UNAVAILABLE,
            },
            args.result,
        )
        return INFRASTRUCTURE_UNAVAILABLE
    locks = [lock]
    try:
        checks = health_report(args.health)
        if any(not check["available"] for check in checks):
            write_report(
                {
                    "status": "infrastructure_unavailable",
                    "category": "preflight",
                    "resource": args.resource,
                    "health": checks,
                    "exit_code": INFRASTRUCTURE_UNAVAILABLE,
                },
                args.result,
            )
            return INFRASTRUCTURE_UNAVAILABLE
        reserved_resources = sorted(
            {resource for check in checks if (resource := health_resource(check))}
        )
        for resource in reserved_resources:
            allocation_lock, allocation_owner = acquire(resource, args.stale_after)
            if allocation_lock is None:
                write_report(
                    {
                        "status": "infrastructure_unavailable",
                        "category": "resource_busy",
                        "resource": args.resource,
                        "busy_resource": resource,
                        "owner": allocation_owner or {},
                        "health": checks,
                        "exit_code": INFRASTRUCTURE_UNAVAILABLE,
                    },
                    args.result,
                )
                return INFRASTRUCTURE_UNAVAILABLE
            locks.append(allocation_lock)
        try:
            child_environment = allocation_environment(checks, args.allocation_env)
        except ValueError as error:
            write_report(
                {
                    "status": "product_failure",
                    "category": "configuration",
                    "resource": args.resource,
                    "health": checks,
                    "detail": str(error),
                    "exit_code": 2,
                },
                args.result,
            )
            return 2
        try:
            completed = subprocess.run(
                args.command,
                check=False,
                timeout=args.timeout or None,
                env=child_environment,
            )
            code = completed.returncode
            status = "passed" if code == 0 else "infrastructure_unavailable" if code == 75 else "product_failure"
            category = "verification" if code != 75 else "test_environment"
        except subprocess.TimeoutExpired:
            code, status, category = 124, "product_failure", "verification_timeout"
        write_report(
            {
                "status": status,
                "category": category,
                "resource": args.resource,
                "health": checks,
                "reserved_resources": reserved_resources,
                "command": args.command,
                "duration_seconds": round(time.monotonic() - started, 3),
                "exit_code": code,
            },
            args.result,
        )
        return code if code >= 0 else 1
    finally:
        for acquired_lock in reversed(locks):
            release(acquired_lock)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="subcommand", required=True)
    health = subparsers.add_parser("health", help="Check resources without reserving them")
    health.add_argument("--health", action="append", default=[], required=True)
    health.add_argument("--result")
    run = subparsers.add_parser("run", help="Reserve a resource, preflight, and run verification")
    run.add_argument("--resource", required=True)
    run.add_argument("--health", action="append", default=[])
    run.add_argument(
        "--allocation-env",
        action="append",
        default=[],
        help="Export the single allocation for KIND as KIND=ENV_VAR",
    )
    run.add_argument("--result")
    run.add_argument("--timeout", type=int, default=0)
    run.add_argument("--stale-after", type=int, default=6 * 60 * 60)
    run.add_argument("command", nargs=argparse.REMAINDER)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    if args.subcommand == "health":
        checks = health_report(args.health)
        available = all(check["available"] for check in checks)
        write_report(
            {
                "status": "available" if available else "infrastructure_unavailable",
                "category": "preflight",
                "health": checks,
                "exit_code": 0 if available else INFRASTRUCTURE_UNAVAILABLE,
            },
            args.result,
        )
        return 0 if available else INFRASTRUCTURE_UNAVAILABLE
    if not args.command or args.command[0] != "--" or len(args.command) == 1:
        print("native_lab.py run requires -- <command>", file=sys.stderr)
        return 2
    args.command = args.command[1:]
    return run_command(args)


if __name__ == "__main__":
    raise SystemExit(main())
