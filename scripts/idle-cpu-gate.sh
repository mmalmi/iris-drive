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
  IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH=<substring required in process command>
  IRIS_DRIVE_IDLE_CPU_APP_MAX=5
  IRIS_DRIVE_IDLE_CPU_DAEMON_MAX=10
  IRIS_DRIVE_IDLE_CPU_PROVIDER_MAX=3
  IRIS_DRIVE_IDLE_CPU_ANDROID_PACKAGE=to.iris.drive
  IRIS_DRIVE_IDLE_CPU_IOS_BUNDLE_ID=fi.siriusbusiness.drive
  IRIS_DRIVE_IDLE_CPU_IOS_DEVICE=<device name or UDID>
  IRIS_DRIVE_IDLE_CPU_IOS_HOST_APP_MAX=10
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
    "daemon": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_DAEMON_MAX", "10")),
    "provider": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_PROVIDER_MAX", "3")),
}
default_required = {
    "macos": {"app", "daemon", "provider"},
    "linux": {"app", "daemon"},
}[platform]
required_env = os.environ.get("IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES", "").strip()
required = {item for item in required_env.replace(",", " ").split() if item} or default_required
command_match = os.environ.get("IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH", "").strip()

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

def parse_cpu_time(value: str) -> float:
    days = 0
    if "-" in value:
        day_value, value = value.split("-", 1)
        days = int(day_value)
    parts = value.split(":")
    if len(parts) == 3:
        hours, minutes, seconds = parts
    elif len(parts) == 2:
        hours = "0"
        minutes, seconds = parts
    else:
        return float(value)
    return days * 86400 + int(hours) * 3600 + int(minutes) * 60 + float(seconds)

def snapshot():
    output = subprocess.check_output(["ps", "-axo", "pid=,ppid=,time=,command="], text=True)
    processes = {}
    for raw in output.splitlines():
        parts = raw.strip().split(None, 3)
        if len(parts) < 4:
            continue
        pid, _ppid, cpu_time, command = parts
        if command_match and command_match not in command:
            continue
        role = classify(command)
        if not role:
            continue
        try:
            value = parse_cpu_time(cpu_time)
        except ValueError:
            continue
        processes[int(pid)] = {"role": role, "cpu_seconds": value, "command": command}
    return processes

print(f"[idle-cpu] warmup {warmup}s, sample {duration}s every {interval}s", file=sys.stderr)
time.sleep(warmup)
samples = {role: [] for role in thresholds}
seen = {role: set() for role in thresholds}
deadline = time.monotonic() + duration
previous_time = time.monotonic()
previous = snapshot()
for process in previous.values():
    seen.setdefault(process["role"], set())
while time.monotonic() < deadline:
    time.sleep(interval)
    now = time.monotonic()
    current = snapshot()
    elapsed = max(now - previous_time, 0.001)
    totals = {role: 0.0 for role in thresholds}
    observed_roles = set()
    for pid, process in current.items():
        role = process["role"]
        observed_roles.add(role)
        seen.setdefault(role, set()).add(pid)
        previous_process = previous.get(pid)
        if previous_process and previous_process["role"] == role:
            delta = max(process["cpu_seconds"] - previous_process["cpu_seconds"], 0.0)
            totals[role] = totals.get(role, 0.0) + (delta / elapsed * 100.0)
    for role in observed_roles:
        samples.setdefault(role, []).append(totals.get(role, 0.0))
    if time.monotonic() >= deadline:
        break
    previous = current
    previous_time = now

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
    adb_cmd() {
      if [[ -n "$serial" ]]; then
        "$adb_bin" -s "$serial" "$@"
      else
        "$adb_bin" "$@"
      fi
    }
    required_env="${IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES:-app}"
    if [[ ! " ${required_env//,/ } " =~ [[:space:]]app[[:space:]] ]]; then
      echo "[idle-cpu] FAIL: Android exposes app/service CPU under package processes, not separate roles: $required_env" >&2
      exit 1
    fi
    echo "[idle-cpu] warmup ${warmup}s, sample ${duration}s every ${interval}s" >&2
    if [[ "${IRIS_DRIVE_IDLE_CPU_ANDROID_LAUNCH:-1}" != "0" ]]; then
      adb_cmd shell monkey -p "$package" -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
    fi
    sleep "$warmup"
    clk_tck="$(adb_cmd shell getconf CLK_TCK 2>/dev/null | tr -d '\r' | awk 'NR == 1 { print }')"
    if [[ -z "$clk_tck" || ! "$clk_tck" =~ ^[0-9]+$ ]]; then
      clk_tck=100
    fi
    python3 - "$adb_bin" "$serial" "$package" "$duration" "$interval" "$clk_tck" <<'PY'
import json
import os
import shlex
import subprocess
import sys
import time

adb_bin, serial, package = sys.argv[1], sys.argv[2], sys.argv[3]
duration, interval, clk_tck = int(sys.argv[4]), int(sys.argv[5]), int(sys.argv[6])
limit = float(os.environ.get("IRIS_DRIVE_IDLE_CPU_APP_MAX", "5"))

def adb_shell(script: str) -> str:
    cmd = [adb_bin]
    if serial:
        cmd.extend(["-s", serial])
    cmd.extend(["shell", script])
    return subprocess.check_output(cmd, text=True, stderr=subprocess.DEVNULL).replace("\r", "")

def snapshot():
    quoted_package = shlex.quote(package)
    script = (
        f"for p in $(pidof {quoted_package} 2>/dev/null); do "
        'if [ -r "/proc/$p/stat" ]; then '
        "awk -v p=\"$p\" '{print p, $14 + $15}' \"/proc/$p/stat\"; "
        "fi; "
        "done; "
        "awk '{print \"uptime\", $1}' /proc/uptime"
    )
    ticks_by_pid = {}
    uptime = None
    try:
        output = adb_shell(script)
    except subprocess.SubprocessError:
        return ticks_by_pid, uptime
    for raw in output.splitlines():
        parts = raw.split()
        if len(parts) != 2:
            continue
        if parts[0] == "uptime":
            try:
                uptime = float(parts[1])
            except ValueError:
                uptime = None
            continue
        try:
            ticks_by_pid[int(parts[0])] = float(parts[1])
        except ValueError:
            continue
    return ticks_by_pid, uptime

values = []
seen_process = False
previous_ticks, previous_uptime = snapshot()
if previous_ticks:
    seen_process = True
deadline = time.monotonic() + duration
while time.monotonic() < deadline:
    time.sleep(interval)
    current_ticks, current_uptime = snapshot()
    if current_ticks:
        seen_process = True
    if previous_uptime is not None and current_uptime is not None:
        elapsed = max(current_uptime - previous_uptime, 0.001)
    else:
        elapsed = float(interval)
    delta_ticks = 0.0
    for pid, current in current_ticks.items():
        previous = previous_ticks.get(pid)
        if previous is not None:
            delta_ticks += max(current - previous, 0.0)
    if current_ticks or previous_ticks:
        values.append(delta_ticks / clk_tck / elapsed * 100.0)
    previous_ticks, previous_uptime = current_ticks, current_uptime

if not values:
    message = "android app process was not observed" if not seen_process else "android app CPU samples were unavailable"
    print(f"[idle-cpu] FAIL: {message}", file=sys.stderr)
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
    launch_ios_app() {
      xcrun simctl launch "$ios_device" "$bundle_id" >/dev/null 2>&1 ||
        xcrun devicectl device process launch --device "$ios_device" "$bundle_id" >/dev/null 2>&1 ||
        true
    }
    run_ios_host_process_sampler() {
      if [[ "${IRIS_DRIVE_IDLE_CPU_IOS_XCTRACE_REQUIRED:-0}" == "1" ]]; then
        return 1
      fi
      if [[ "${IRIS_DRIVE_IDLE_CPU_IOS_LAUNCH:-1}" != "0" ]]; then
        launch_ios_app
      fi
      echo "[idle-cpu] warmup ${warmup}s, host process delta ${duration}s every ${interval}s on simulator ${ios_device}" >&2
      python3 - "$ios_device" "$warmup" "$duration" "$interval" <<'PY'
import json
import os
import subprocess
import sys
import time

ios_device, warmup, duration, interval = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
device_fragment = f"/CoreSimulator/Devices/{ios_device}/"
thresholds = {
    "app": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_IOS_HOST_APP_MAX", os.environ.get("IRIS_DRIVE_IDLE_CPU_APP_MAX", "5"))),
    "provider": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_PROVIDER_MAX", "3")),
}
required_env = os.environ.get("IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES", "").strip()
required = {item for item in required_env.replace(",", " ").split() if item} or {"app", "provider"}

def classify(command: str):
    if device_fragment not in command:
        return None
    if "/IrisDriveFileProvider.appex/IrisDriveFileProvider" in command:
        return "provider"
    if "/Iris Drive.app/Iris Drive" in command and "/PlugIns/" not in command:
        return "app"
    return None

def parse_cpu_time(value: str) -> float:
    days = 0
    if "-" in value:
        day_value, value = value.split("-", 1)
        days = int(day_value)
    parts = value.split(":")
    if len(parts) == 3:
        hours, minutes, seconds = parts
    elif len(parts) == 2:
        hours = "0"
        minutes, seconds = parts
    else:
        return float(value)
    return days * 86400 + int(hours) * 3600 + int(minutes) * 60 + float(seconds)

def snapshot():
    output = subprocess.check_output(["ps", "-axo", "pid=,time=,command="], text=True)
    processes = {}
    for raw in output.splitlines():
        parts = raw.strip().split(None, 2)
        if len(parts) < 3:
            continue
        pid, cpu_time, command = parts
        role = classify(command)
        if not role:
            continue
        try:
            value = parse_cpu_time(cpu_time)
        except ValueError:
            continue
        processes[int(pid)] = {"role": role, "cpu_seconds": value}
    return processes

time.sleep(warmup)
samples = {role: [] for role in thresholds}
seen = {role: set() for role in thresholds}
deadline = time.monotonic() + duration
previous_time = time.monotonic()
previous = snapshot()
while time.monotonic() < deadline:
    time.sleep(interval)
    now = time.monotonic()
    current = snapshot()
    elapsed = max(now - previous_time, 0.001)
    totals = {role: 0.0 for role in thresholds}
    observed_roles = set()
    for pid, process in current.items():
        role = process["role"]
        observed_roles.add(role)
        seen.setdefault(role, set()).add(pid)
        previous_process = previous.get(pid)
        if previous_process and previous_process["role"] == role:
            delta = max(process["cpu_seconds"] - previous_process["cpu_seconds"], 0.0)
            totals[role] = totals.get(role, 0.0) + (delta / elapsed * 100.0)
    for role in observed_roles:
        samples.setdefault(role, []).append(totals.get(role, 0.0))
    previous = current
    previous_time = now

summary = {}
failures = []
for role in sorted(required | set(samples)):
    values = samples.get(role, [])
    if not values:
        if role in required:
            failures.append(f"{role}: required simulator process role was not observed")
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

print(json.dumps({
    "platform": "ios",
    "method": "host-process-delta",
    "required_roles": sorted(required),
    "roles": summary,
}, indent=2, sort_keys=True))
if failures:
    for failure in failures:
        print(f"[idle-cpu] FAIL: {failure}", file=sys.stderr)
    sys.exit(1)
print("[idle-cpu] OK", file=sys.stderr)
PY
    }
    if [[ "${IRIS_DRIVE_IDLE_CPU_IOS_LAUNCH:-1}" != "0" ]]; then
      launch_ios_app
    fi
    trace_dir="$(mktemp -d -t iris-drive-ios-idle-cpu.XXXXXX)"
    trap 'rm -rf "$trace_dir"' EXIT
    start_trace="$trace_dir/start.trace"
    end_trace="$trace_dir/end.trace"
    start_xml="$trace_dir/start.xml"
    end_xml="$trace_dir/end.xml"
    snapshot_secs="${IRIS_DRIVE_IDLE_CPU_IOS_SNAPSHOT_SECS:-5}"
    record_ios_activity_snapshot() {
      local output="$1"
      rm -rf "$output"
      xcrun xctrace record --quiet --template 'Activity Monitor' --device "$ios_device" \
        --all-processes --time-limit "${snapshot_secs}s" --output "$output" >/dev/null
    }
    echo "[idle-cpu] warmup ${warmup}s, xctrace Activity Monitor delta ${duration}s on ${ios_device}" >&2
    sleep "$warmup"
    start_epoch="$(python3 -c 'import time; print(time.time())')"
    if ! record_ios_activity_snapshot "$start_trace"; then
      echo "[idle-cpu] WARN: xctrace Activity Monitor start snapshot failed for ${ios_device}; trying host process sampler" >&2
      run_ios_host_process_sampler
      exit $?
    fi
    sleep "$duration"
    if [[ "${IRIS_DRIVE_IDLE_CPU_IOS_LAUNCH:-1}" != "0" ]]; then
      launch_ios_app
    fi
    if ! record_ios_activity_snapshot "$end_trace"; then
      echo "[idle-cpu] WARN: xctrace Activity Monitor end snapshot failed for ${ios_device}; trying host process sampler" >&2
      run_ios_host_process_sampler
      exit $?
    fi
    end_epoch="$(python3 -c 'import time; print(time.time())')"
    if ! xcrun xctrace export --input "$start_trace" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="activity-monitor-process-ledger"]' >"$start_xml"; then
      echo "[idle-cpu] WARN: xctrace Activity Monitor start export failed for ${ios_device}; trying host process sampler" >&2
      run_ios_host_process_sampler
      exit $?
    fi
    if ! xcrun xctrace export --input "$end_trace" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="activity-monitor-process-ledger"]' >"$end_xml"; then
      echo "[idle-cpu] WARN: xctrace Activity Monitor end export failed for ${ios_device}; trying host process sampler" >&2
      run_ios_host_process_sampler
      exit $?
    fi
    if ! python3 - "$start_epoch" "$end_epoch" "$start_xml" "$end_xml" <<'PY'
import json
import os
import re
import sys
import xml.etree.ElementTree as ET

start_epoch = float(sys.argv[1])
end_epoch = float(sys.argv[2])
start_xml = sys.argv[3]
end_xml = sys.argv[4]
elapsed = max(end_epoch - start_epoch, 0.001)
thresholds = {
    "app": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_APP_MAX", "5")),
    "provider": float(os.environ.get("IRIS_DRIVE_IDLE_CPU_PROVIDER_MAX", "3")),
}
required_env = os.environ.get("IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES", "").strip()
required = {item for item in required_env.replace(",", " ").split() if item} or {"app", "provider"}
app_match = re.compile(os.environ.get("IRIS_DRIVE_IDLE_CPU_IOS_APP_MATCH", r"^(Iris Drive|fi\.siriusbusiness\.drive)$"))
provider_match = re.compile(os.environ.get("IRIS_DRIVE_IDLE_CPU_IOS_PROVIDER_MATCH", r"^IrisDriveFileProvider$"))

def local_name(tag):
    return tag.rsplit("}", 1)[-1]

def process_identity(element):
    fmt = element.attrib.get("fmt", "")
    if fmt:
        pid_match = re.search(r"\((\d+)\)$", fmt)
        pid = int(pid_match.group(1)) if pid_match else 0
        return re.sub(r"\s+\(\d+\)$", "", fmt), pid
    return (element.text or "").strip(), 0

def classify(name):
    if provider_match.search(name):
        return "provider"
    if app_match.search(name):
        return "app"
    return None

def read_snapshot(path):
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError as error:
        print(f"[idle-cpu] FAIL: could not parse xctrace XML: {error}", file=sys.stderr)
        sys.exit(1)
    rows = []
    for table in root.iter():
        if local_name(table.tag) == "node":
            rows.extend([child for child in table if local_name(child.tag) == "row"])
    if not rows:
        print("[idle-cpu] FAIL: xctrace Activity Monitor table had no rows", file=sys.stderr)
        sys.exit(1)
    snapshot = {}
    for row in rows:
        cells = list(row)
        if len(cells) < 8:
            continue
        proc = cells[1]
        cpu = cells[7]
        if local_name(proc.tag) != "process":
            continue
        name, pid = process_identity(proc)
        role = classify(name)
        if not role:
            continue
        try:
            cpu_ns = float((cpu.text or "0").strip())
        except ValueError:
            cpu_ns = 0.0
        previous = snapshot.get(role)
        if previous is None or cpu_ns > previous["cpu_ns"]:
            snapshot[role] = {"name": name, "pid": pid, "cpu_ns": cpu_ns}
    return snapshot

start = read_snapshot(start_xml)
end = read_snapshot(end_xml)
summary = {}
failures = []
for role in sorted(required | set(start) | set(end)):
    if role not in start or role not in end:
        if role in required:
            failures.append(f"{role}: required process role was not observed in both snapshots")
        continue
    if start[role]["pid"] and end[role]["pid"] and start[role]["pid"] != end[role]["pid"]:
        failures.append(f"{role}: process restarted during idle sample")
        continue
    delta_ns = end[role]["cpu_ns"] - start[role]["cpu_ns"]
    if delta_ns < 0:
        failures.append(f"{role}: cumulative CPU counter decreased during idle sample")
        continue
    avg = delta_ns / 1_000_000_000.0 / elapsed * 100.0
    limit = thresholds.get(role, thresholds["app"])
    summary[role] = {
        "avg_cpu": round(avg, 2),
        "peak_cpu": round(avg, 2),
        "samples": 2,
        "elapsed_secs": round(elapsed, 2),
        "cpu_delta_ns": int(delta_ns),
        "pids": sorted({start[role]["pid"], end[role]["pid"]} - {0}),
        "limit": limit,
    }
    if avg > limit:
        failures.append(f"{role}: avg CPU {avg:.2f}% > {limit:.2f}%")

print(json.dumps({
    "platform": "ios",
    "method": "xctrace activity-monitor cumulative delta",
    "required_roles": sorted(required),
    "roles": summary,
}, indent=2, sort_keys=True))
if failures:
    for failure in failures:
        print(f"[idle-cpu] FAIL: {failure}", file=sys.stderr)
    sys.exit(1)
print("[idle-cpu] OK", file=sys.stderr)
PY
    then
      run_ios_host_process_sampler
      exit $?
    fi
    ;;
  *)
    echo "unsupported idle CPU gate platform: $platform" >&2
    exit 2
    ;;
esac
