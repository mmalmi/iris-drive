#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/macos/Sources/IrisDriveMacApp.swift"
SOURCES="$ROOT/macos/Sources"
IOS_MODEL="$ROOT/ios/Sources/IrisDriveMobileModel.swift"
ANDROID_MAIN="$ROOT/android/app/src/main/java/to/iris/drive/app/MainActivity.kt"
ANDROID_DEBUG="$ROOT/android/app/src/main/java/to/iris/drive/app/AndroidDebugSupport.kt"
WINDOWS_SERVICE="$ROOT/windows/IrisDriveService.cs"
WINDOWS_NATIVE_CORE="$ROOT/windows/IrisDriveServiceNativeCore.cs"
WINDOWS_NATIVE="$ROOT/windows/IrisDriveNativeCore.cs"

require_contains() {
  local needle="$1"
  if ! grep -Fqr "$needle" "$SOURCES"; then
    echo "missing '$needle' in macos/Sources" >&2
    exit 1
  fi
}

require_absent() {
  local needle="$1"
  if grep -Fq "$needle" "$APP"; then
    echo "unexpected '$needle' in macos/Sources/IrisDriveMacApp.swift" >&2
    exit 1
  fi
}

require_file_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "missing '$needle' in ${file#$ROOT/}" >&2
    exit 1
  fi
}

require_file_absent() {
  local file="$1"
  local needle="$2"
  if grep -Fq "$needle" "$file"; then
    echo "unexpected '$needle' in ${file#$ROOT/}" >&2
    exit 1
  fi
}

require_contains 'let nativeCoreQueue = DispatchQueue(label: "to.iris.drive.macos.native-core"'
require_contains "var nativeStatusRefreshInFlight = false"
require_contains "var nativeStatusRefreshPending = false"
require_contains "func scheduleNativeStatusRefresh()"
require_contains "func finishNativeStatusRefresh()"
require_contains "nativeCoreQueue.async { [weak self] in"
require_contains "self.scheduleNativeStatusRefresh()"
require_absent "let state = try nativeStatePayload(from: desktopCore.refreshJson())"
require_file_contains "$IOS_MODEL" 'private let nativeCoreQueue = DispatchQueue(label: "fi.siriusbusiness.drive.native-core"'
require_file_contains "$IOS_MODEL" "nativeCoreQueue.async"
require_file_contains "$IOS_MODEL" "nativeCoreQueue.sync"
require_file_contains "$IOS_MODEL" "private func runNative<T>"
require_file_contains "$IOS_MODEL" 'applyStateJson(runNative { $0.refreshJson() })'
require_file_contains "$IOS_MODEL" 'applyStateJson(runNative { $0.dispatchJson(actionJson) })'
require_file_absent "$IOS_MODEL" "let thread = Thread"
require_file_absent "$IOS_MODEL" "applyStateJson(nativeCore.refreshJson())"
require_file_absent "$IOS_MODEL" "applyStateJson(nativeCore.stateJson())"
require_file_absent "$IOS_MODEL" "applyStateJson(nativeCore.dispatchJson(actionJson))"
require_file_contains "$ANDROID_MAIN" "private val nativeCoreExecutor = Executors.newSingleThreadExecutor"
require_file_contains "$ANDROID_MAIN" "private val nativeCoreDispatcher = nativeCoreExecutor.asCoroutineDispatcher()"
require_file_contains "$ANDROID_MAIN" "private var nativeRefreshInFlight = false"
require_file_contains "$ANDROID_MAIN" "lifecycleScope.launch(nativeCoreDispatcher)"
require_file_contains "$ANDROID_MAIN" "AndroidDebugSupport.writeState"
require_file_contains "$ANDROID_DEBUG" "fun writeState"
require_file_absent "$ANDROID_MAIN" "NativeCore.stateJson(nativeHandle)"
require_file_contains "$WINDOWS_NATIVE_CORE" "private readonly SemaphoreSlim nativeCoreGate = new(1, 1)"
require_file_contains "$WINDOWS_NATIVE_CORE" "private async Task<T> RunNativeCoreAsync<T>"
require_file_contains "$WINDOWS_NATIVE_CORE" "private Task<IrisDriveStatusData> DispatchNativeActionAsync"
require_file_absent "$WINDOWS_SERVICE" "NativeCore.DispatchActionAsync("
require_file_contains "$WINDOWS_NATIVE" "public IrisDriveStatusData DispatchAction"
require_file_absent "$WINDOWS_NATIVE" "Task.Run(() => IrisDriveStatusData.FromNativeJson(DispatchJson(actionJson)))"

echo "NATIVE_CORE_SERIALIZATION_OK"
