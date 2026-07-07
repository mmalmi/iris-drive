#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/idle-cpu-gate.sh [--platform auto|macos|linux|windows|android|ios]

Samples idle CPU for Iris Drive process roles and fails when any required role
stays above its budget. One full core is 100%.

Environment:
  IRIS_DRIVE_IDLE_CPU_WARMUP_SECS=30
  IRIS_DRIVE_IDLE_CPU_DURATION_SECS=60
  IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS=5
  IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES=app,daemon,provider
  IRIS_DRIVE_IDLE_CPU_APP_MAX=5
  IRIS_DRIVE_IDLE_CPU_DAEMON_MAX=5
  IRIS_DRIVE_IDLE_CPU_PROVIDER_MAX=3
  IRIS_DRIVE_IDLE_CPU_ANDROID_PACKAGE=to.iris.drive
  IRIS_DRIVE_IDLE_CPU_IOS_BUNDLE_ID=fi.siriusbusiness.drive
  IRIS_DRIVE_IDLE_CPU_IOS_DEVICE=<device name or UDID>
  ANDROID_SERIAL / IRIS_DRIVE_ANDROID_DEVICE / IRIS_DRIVE_ANDROID_SERIAL
USAGE
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
platform=auto

while [[ $# -gt 0 ]]; do
  case "$1" in
    --platform)
      platform="${2:-}"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$platform" == "auto" ]]; then
  case "$(uname -s)" in
    Darwin) platform=macos ;;
    Linux) platform=linux ;;
    MINGW* | MSYS* | CYGWIN*) platform=windows ;;
    *)
      echo "idle CPU gate does not support local platform $(uname -s)" >&2
      exit 2
      ;;
  esac
fi

warmup="${IRIS_DRIVE_IDLE_CPU_WARMUP_SECS:-30}"
duration="${IRIS_DRIVE_IDLE_CPU_DURATION_SECS:-60}"
interval="${IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS:-5}"

resolve_adb() {
  local sdk
  sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "$sdk" && -f "$ROOT/android/local.properties" ]]; then
    sdk="$(sed -n 's/^sdk\.dir=//p' "$ROOT/android/local.properties" | head -n 1)"
  fi
  if [[ -z "$sdk" && -d "$HOME/Library/Android/sdk" ]]; then
    sdk="$HOME/Library/Android/sdk"
  fi
  if [[ -n "$sdk" && -x "$sdk/platform-tools/adb" ]]; then
    printf '%s\n' "$sdk/platform-tools/adb"
    return
  fi
  if command -v adb >/dev/null 2>&1; then
    command -v adb
    return
  fi
  echo "adb not found; set ANDROID_HOME/ANDROID_SDK_ROOT or add adb to PATH" >&2
  exit 1
}

case "$platform" in
  macos | linux)
    python3 - "$platform" "$warmup" "$duration" "$interval" <<'PY'
import json
import os
import shlex
import subprocess
import sys
import time

platform, warmup, duration, interval = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
thresholds = {
    "app": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_APP_MAX", "5")),
    "daemon": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_DAEMON_MAX", "5")),
    "provider": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_PROVIDER_MAX", "3")),
}
default_required = {
    "macos": {"app", "daemon", "provider"},
    "linux": {"app", "daemon"},
}[platform]
required_env = os.environ.get("IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES", "").strip()
required = {item for item in required_env.replace(",", " ").split() if item} or default_required

def executable_basename(command: str) -> str:
    try:
        parts = shlex.split(command)
    except ValueError:
        parts = command.split()
    return os.path.basename(parts[0]) if parts else ""

def classify(command: str):
    executable = executable_basename(command)
    if platform == "macos":
        if "IrisDriveFileProvider" in command:
            return "provider"
        if "/Iris Drive.app/Contents/MacOS/idrive" in command and " daemon" in command:
            return "daemon"
        if executable == "idrive" and " daemon" in command:
            return "daemon"
        if "/Iris Drive.app/Contents/MacOS/Iris Drive" in command:
            return "app"
        return None
    if executable == "idrive" and " daemon" in command:
        return "daemon"
    if executable == "iris-drive":
        return "app"
    return None

def snapshot():
    output = subprocess.check_output(["ps", "-axo", "pid=,ppid=,pcpu=,command="], text=True)
    roles = {}
    for raw in output.splitlines():
        parts = raw.strip().split(None, 3)
        if len(parts) < 4:
            continue
        pid, _ppid, cpu, command = parts
        role = classify(command)
        if not role:
            continue
        try:
            value = float(cpu)
        except ValueError:
            continue
        roles.setdefault(role, []).append({"pid": int(pid), "cpu": value, "command": command})
    return roles

print(f"[idle-cpu] warmup {warmup}s, sample {duration}s every {interval}s", file=sys.stderr)
time.sleep(warmup)
samples = {role: [] for role in thresholds}
seen = {role: set() for role in thresholds}
deadline = time.monotonic() + duration
while True:
    roles = snapshot()
    for role, processes in roles.items():
        total = sum(process["cpu"] for process in processes)
        samples.setdefault(role, []).append(total)
        for process in processes:
            seen.setdefault(role, set()).add(process["pid"])
    if time.monotonic() >= deadline:
        break
    time.sleep(interval)

summary = {}
failures = []
for role in sorted(required | set(samples)):
    values = samples.get(role, [])
    if not values:
        if role in required:
            failures.append(f"{role}: required process role was not observed")
        continue
    avg = sum(values) / len(values)
    peak = max(values)
    limit = thresholds.get(role, thresholds["app"])
    summary[role] = {
        "avg_cpu": round(avg, 2),
        "peak_cpu": round(peak, 2),
        "samples": len(values),
        "pids": sorted(seen.get(role, set())),
        "limit": limit,
    }
    if avg > limit:
        failures.append(f"{role}: avg CPU {avg:.2f}% > {limit:.2f}%")

print(json.dumps({"platform": platform, "required_roles": sorted(required), "roles": summary}, indent=2, sort_keys=True))
if failures:
    for failure in failures:
        print(f"[idle-cpu] FAIL: {failure}", file=sys.stderr)
    sys.exit(1)
print("[idle-cpu] OK", file=sys.stderr)
PY
    ;;
  windows)
    powershell_bin="${POWERSHELL:-}"
    if [[ -z "$powershell_bin" ]]; then
      if command -v pwsh >/dev/null 2>&1; then
        powershell_bin="$(command -v pwsh)"
      elif command -v powershell.exe >/dev/null 2>&1; then
        powershell_bin="$(command -v powershell.exe)"
      elif command -v powershell >/dev/null 2>&1; then
        powershell_bin="$(command -v powershell)"
      else
        echo "PowerShell not found for Windows idle CPU gate" >&2
        exit 1
      fi
    fi
    "$powershell_bin" -NoProfile -NonInteractive -ExecutionPolicy Bypass \
      -File "$ROOT/scripts/idle-cpu-gate-windows.ps1"
    ;;
  android)
    adb_bin="$(resolve_adb)"
    package="${IRIS_DRIVE_IDLE_CPU_ANDROID_PACKAGE:-${IRIS_DRIVE_ANDROID_PACKAGE:-to.iris.drive}}"
    serial="${IRIS_DRIVE_ANDROID_DEVICE:-${IRIS_DRIVE_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}}"
    adb_args=()
    if [[ -n "$serial" ]]; then
      adb_args=(-s "$serial")
    fi
    required_env="${IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES:-app}"
    if [[ ! " ${required_env//,/ } " =~ [[:space:]]app[[:space:]] ]]; then
      echo "[idle-cpu] FAIL: Android exposes app/service CPU under package processes, not separate roles: $required_env" >&2
      exit 1
    fi
    echo "[idle-cpu] warmup ${warmup}s, sample ${duration}s every ${interval}s" >&2
    sleep "$warmup"
    tmp="$(mktemp -t iris-drive-android-idle-cpu.XXXXXX)"
    trap 'rm -f "$tmp"' EXIT
    end=$((SECONDS + duration))
    while (( SECONDS <= end )); do
      "$adb_bin" "${adb_args[@]}" shell dumpsys cpuinfo 2>/dev/null |
        tr -d '\r' |
        awk -v package="$package" '
          $0 ~ package {
            gsub("%", "", $1)
            if ($1 ~ /^[0-9.]+$/) { total += $1; seen = 1 }
          }
          END { if (seen) print total }
        ' >>"$tmp" || true
      sleep "$interval"
    done
    python3 - "$tmp" <<'PY'
import json
import os
import sys

values = []
for line in open(sys.argv[1], encoding="utf-8"):
    try:
        values.append(float(line.strip()))
    except ValueError:
        pass
limit = float(os.environ.get("IRIS_DRIVE_IDLE_CPU_APP_MAX", "5"))
if not values:
    print("[idle-cpu] FAIL: android app process was not observed", file=sys.stderr)
    sys.exit(1)
avg = sum(values) / len(values)
summary = {"platform": "android", "required_roles": ["app"], "roles": {"app": {"avg_cpu": round(avg, 2), "peak_cpu": round(max(values), 2), "samples": len(values), "limit": limit}}}
print(json.dumps(summary, indent=2, sort_keys=True))
if avg > limit:
    print(f"[idle-cpu] FAIL: android app avg CPU {avg:.2f}% > {limit:.2f}%", file=sys.stderr)
    sys.exit(1)
print("[idle-cpu] OK", file=sys.stderr)
PY
    ;;
  ios)
    case "$(uname -s)" in
      Darwin) ;;
      *)
        echo "iOS idle CPU gate requires macOS/Xcode host tooling" >&2
        exit 2
        ;;
    esac
    command -v xcrun >/dev/null 2>&1 || { echo "xcrun not found" >&2; exit 1; }
    bundle_id="${IRIS_DRIVE_IDLE_CPU_IOS_BUNDLE_ID:-${IRIS_DRIVE_IOS_BUNDLE_ID:-fi.siriusbusiness.drive}}"
    ios_device="${IRIS_DRIVE_IDLE_CPU_IOS_DEVICE:-${IRIS_DRIVE_IOS_DEVICE:-${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-}}}"
    if [[ -z "$ios_device" ]]; then
      ios_device="$(
        xcrun simctl list devices booted --json |
          python3 -c 'import json,sys; data=json.load(sys.stdin); print(next((d["udid"] for runtime, devices in data.get("devices", {}).items() if "iOS" in runtime for d in devices if d.get("state") == "Booted"), ""))'
      )"
    fi
    if [[ -z "$ios_device" ]]; then
      echo "iOS idle CPU gate needs IRIS_DRIVE_IDLE_CPU_IOS_DEVICE or a booted iOS simulator" >&2
      exit 2
    fi
    if [[ "${IRIS_DRIVE_IDLE_CPU_IOS_LAUNCH:-1}" != "0" ]]; then
      xcrun simctl launch --terminate-running-process "$ios_device" "$bundle_id" >/dev/null 2>&1 ||
        xcrun devicectl device process launch --device "$ios_device" "$bundle_id" >/dev/null 2>&1 ||
        true
    fi
    trace_dir="$(mktemp -d -t iris-drive-ios-idle-cpu.XXXXXX)"
    trap 'rm -rf "$trace_dir"' EXIT
    trace="$trace_dir/idle.trace"
    echo "[idle-cpu] warmup ${warmup}s, xctrace Activity Monitor ${duration}s on ${ios_device}" >&2
    sleep "$warmup"
    xcrun xctrace record --quiet --template 'Activity Monitor' --device "$ios_device" \
      --all-processes --time-limit "${duration}s" --output "$trace" >/dev/null
    xcrun xctrace export --input "$trace" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="activity-monitor-process-ledger"]' |
      python3 - "$duration" <<'PY'
import json
import os
import re
import sys
import xml.etree.ElementTree as ET

duration = max(float(sys.argv[1]), 1.0)
xml = sys.stdin.read()
thresholds = {
    "app": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_APP_MAX", "5")),
    "provider": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_PROVIDER_MAX", "3")),
}
required_env = os.environ.get("IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES", "").strip()
required = {item for item in required_env.replace(",", " ").split() if item} or {"app", "provider"}
app_match = re.compile(os.environ.get("IRIS_DRIVE_IDLE_CPU_IOS_APP_MATCH", r"^(Iris Drive|fi\.siriusbusiness\.drive)$"))
provider_match = re.compile(os.environ.get("IRIS_DRIVE_IDLE_CPU_IOS_PROVIDER_MATCH", r"(IrisDriveFileProvider|FileProvider)"))

def local_name(tag):
    return tag.rsplit("}", 1)[-1]

def process_name(element):
    fmt = element.attrib.get("fmt", "")
    if fmt:
        return re.sub(r"\s+\(\d+\)$", "", fmt)
    return (element.text or "").strip()

def classify(name):
    if provider_match.search(name):
        return "provider"
    if app_match.search(name):
        return "app"
    return None

try:
    root = ET.fromstring(xml)
except ET.ParseError as error:
    print(f"[idle-cpu] FAIL: could not parse xctrace XML: {error}", file=sys.stderr)
    sys.exit(1)

tables = [node for node in root.iter() if local_name(node.tag) == "node"]
rows = []
for table in tables:
    rows.extend([child for child in table if local_name(child.tag) == "row"])
if not rows:
    print("[idle-cpu] FAIL: xctrace Activity Monitor table had no rows", file=sys.stderr)
    sys.exit(1)

totals = {}
pids = {}
for row in rows:
    cells = list(row)
    if len(cells) < 8:
        continue
    proc = cells[1]
    pid = cells[4]
    cpu = cells[7]
    if local_name(proc.tag) != "process":
        continue
    name = process_name(proc)
    role = classify(name)
    if not role:
        continue
    try:
        cpu_ns = float((cpu.text or "0").strip())
    except ValueError:
        cpu_ns = 0.0
    totals[role] = totals.get(role, 0.0) + cpu_ns
    try:
        pids.setdefault(role, set()).add(int((pid.text or "0").strip()))
    except ValueError:
        pass

summary = {}
failures = []
for role in sorted(required | set(totals)):
    if role not in totals:
        if role in required:
            failures.append(f"{role}: required process role was not observed")
        continue
    avg = totals[role] / 1_000_000_000.0 / duration * 100.0
    limit = thresholds.get(role, thresholds["app"])
    summary[role] = {
        "avg_cpu": round(avg, 2),
        "peak_cpu": round(avg, 2),
        "samples": 1,
        "pids": sorted(pids.get(role, set())),
        "limit": limit,
    }
    if avg > limit:
        failures.append(f"{role}: avg CPU {avg:.2f}% > {limit:.2f}%")

print(json.dumps({"platform": "ios", "required_roles": sorted(required), "roles": summary}, indent=2, sort_keys=True))
if failures:
    for failure in failures:
        print(f"[idle-cpu] FAIL: {failure}", file=sys.stderr)
    sys.exit(1)
print("[idle-cpu] OK", file=sys.stderr)
PY
    ;;
  *)
    echo "unsupported idle CPU gate platform: $platform" >&2
    exit 2
    ;;
esac
