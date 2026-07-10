# Verification tiers and native lab

Iris Drive separates deterministic per-change confidence from scarce native
lab confidence.

## Fast tier

Run `just verify-fast` for each coherent change. It never allocates a phone,
simulator, VM, GUI session, FileProvider domain, or Cloud Files sync root. It
runs formatting, strict all-target Clippy, repository structure/contracts,
release helper tests, workspace tests outside the CLI, and the focused CLI link
contract.

## Full tier

Run `just verify-full` nightly and before a release. Configure these resources:

- `IRIS_DRIVE_E2E_UBUNTU_HOST`
- `IRIS_DRIVE_E2E_WINDOWS_HOST`
- `IRIS_DRIVE_E2E_MACOS_HOST`
- `IRIS_DRIVE_E2E_IOS_HOST`
- `IRIS_DRIVE_E2E_ANDROID_HOST`

The mobile host value may be `local`. For local resources,
`IRIS_DRIVE_LAB_IOS_SIMULATOR`, `IRIS_DRIVE_LAB_IOS_DEVICE`, and
`IRIS_DRIVE_LAB_ANDROID_SERIAL` select a stable allocation by name or ID;
`auto` is the default. Prefer explicit IDs in scheduled jobs.
For remote mobile hosts all three selectors are required so the job cannot
silently switch devices.

`scripts/native_lab.py` atomically reserves the matrix plus every selected
local/SSH host, simulator, and phone. It passes the exact selected IDs into the
test process and writes `artifacts/verification/full-native-result.json`. A
missing or busy resource exits 75 with status `infrastructure_unavailable`.
Test failures retain their normal nonzero exit and status `product_failure`.

Use `just verify-health` to inspect resource health without running tests.

## Deterministic resets

Resets are destructive and off by default. Use
`IRIS_NATIVE_LAB_RESET=1 just verify-full` only with dedicated lab devices.
The full wrapper erases the selected iOS simulator and clears the selected
Android app state while holding the reservation. It also resets the configured
macOS FileProvider domain and Windows Cloud Files root. Configure:

- `IRIS_DRIVE_LAB_FILEPROVIDER_DOMAIN_ID`
- `IRIS_DRIVE_LAB_FILEPROVIDER_DISPLAY_NAME`
- `IRIS_DRIVE_LAB_FILEPROVIDER_STATE_DIR` when the lab uses a dedicated temp state directory
- `IRIS_DRIVE_LAB_WINDOWS_CONFIG_DIR`
- `IRIS_DRIVE_LAB_WINDOWS_SYNC_ROOT`

Remote iOS reset requires an explicit simulator UDID, not only a display name.

Drive-specific reset helpers are also available:

- `scripts/native_state_reset.sh macos-fileprovider` removes a named
  FileProvider domain and optionally a dedicated temporary state directory.
- `scripts/reset_windows_cloudfiles.ps1` stops processes using explicit lab
  paths, unregisters that Cloud Files sync root, and removes its dedicated
  config/sync directories.

Both helpers require `IRIS_NATIVE_LAB_ALLOW_RESET=1`. The macOS helper refuses to
remove state outside temporary directories; the Windows helper accepts only
paths below `%TEMP%` or `%LOCALAPPDATA%\IrisDriveLab`.

Never point reset helpers at a normal user profile, production FileProvider
domain, or ordinary Cloud Files sync root.
