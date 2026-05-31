# Platform GUI parity

macOS is the current behavior reference for the native control panel. Linux,
Windows, iOS, and Android should expose the same user-visible sync controls
where the platform allows it, even when their OS file-provider backends differ.
`UI scaffold` means the shell exposes the control and state shape, but still
needs the full mobile app-core/idrive backend hookup before it is production
equivalent to desktop.

| Capability | Linux GTK | macOS SwiftUI | Windows WPF | iOS SwiftUI | Android Compose |
| --- | --- | --- | --- | --- | --- |
| First-run create profile | Yes | Yes | Yes | Local create flow | App-core create flow |
| Restore owner profile | Yes | Yes | Yes | Local restore flow | App-core restore flow |
| Link this device flow | Sign in -> Link this device | Sign in -> Link this device | Sign in -> Link this device | Sign in -> Link this device + deep link | Sign in -> Link this device + deep link |
| Log out local profile | Yes | Yes | Yes | App-core logout flow | App-core logout flow |
| Copy owner/device keys | Yes | Yes | Yes | Yes | App-core UI flow |
| Add another device | Add Device dialog | Add Device sheet | Add Device dialog | Add Device sheet + deep link | Add Device dialog + deep link |
| Start/stop/restart sync daemon | Yes | Yes | Yes | Foreground sync control scaffold | Foreground service start/stop/restart |
| Auto-scan local drive folder | No; mount publishes writes | No | No | No | No |
| Open drive folder | Yes, mounted | FileProvider domain | Cloud Files placeholders | Files app FileProvider domain + open action | SAF DocumentsProvider + open action |
| Copy/view drive.iris.to link | Yes | Yes | Yes | Yes | App-core UI flow |
| Devices list and auth state | Yes | Yes | Yes | Local UI flow | App-core UI flow |
| Device online/sync status | Yes | Planned | Yes | Local-only scaffold | Local scaffold |
| Owner device revoke control | Yes | Planned | Yes | Local UI flow | App-core UI flow |
| Relay add/reset controls | Yes | Yes | Yes | Local UI flow | App-core UI flow |
| Direct FIPS block sync | Yes | Yes | Yes | Harness daemon peer; app pending | Harness daemon peer; app pending |
| Blossom remote server list | Yes | Yes | Yes | Read-only list | Read-only list |
| Hashtree config/block/root paths | Yes | Yes | Yes | App-group runtime/config/block paths | App files path only |
| Tray/menu-bar control | Yes | Yes | Yes | N/A | N/A |
| Close to tray/menu-bar | Yes | Yes | Yes | N/A | N/A |
| Native OS file-provider surface | FUSE mount | FileProvider domain | Cloud Files read hydration | FileProvider extension/domain | DocumentsProvider read/write surface |
| Multidevice e2e label | `ubuntu` | `macos` | `windows` | `ios` provider-command peer + simulator smoke | `android` provider-command peer + adb smoke |

## Desktop test target

The local "e2e everything between 3 VMs" command is:

```bash
just e2e-3vms
```

It runs the Rust workspace tests, updates/builds/starts the configured macOS,
Ubuntu, and Windows dev VMs, then runs the native 3-VM sync battery against the
real FileProvider, FUSE, and Cloud Files surfaces. VM hostnames stay in
`~/.config/iris-drive/dev-lab.env` or local git remotes, not in tracked files.
The native smoke writes per-hop timing JSONL to `target/e2e-3vms-*-timings.jsonl`.

The minimum parity smoke for native desktop shells is:

1. Create an owner profile on one VM.
2. Link the other VM as a secondary device from the GUI and copy its Device ID.
3. Open Add Device in the owner GUI, paste the Device ID, and approve it.
4. Confirm both Devices tabs show the authorized peer and its FIPS online/sync state.
5. Create, rename, edit, and delete files inside the mounted drive.
6. Confirm authorized peers receive the new roots without using a normal folder scan.
7. Confirm native directory viewers/watchers wake after remote creates and deletes without reopening the folder.
8. Confirm the three native visible directories have matching path/content manifests and no unintended conflict copies.

Block replication now tries direct hashtree-over-FIPS transfer between
authorized Iris Drive instances first. Blossom remains configured as a remote
cache path, not the primary direct sync transport.

## Mobile test target

iOS has a buildable SwiftUI shell plus a FileProvider extension registered by
the containing app. The local iOS simulator smoke is:

```bash
just ios-smoke
just ios-gui-smoke
```

Android has a buildable Jetpack Compose shell plus a SAF `DocumentsProvider`
registered as `to.iris.drive.documents`. The local Android adb smoke is:

```bash
just android-smoke
just android-gui-smoke
```

For the full desktop + mobile lab, use:

```bash
just e2e-5devices
```

That runs the iOS simulator and GUI linking smokes, runs the Android GUI
linking and provider smokes on the configured Android host, then includes both
mobile host labels as daemon peers in the shared multidevice sync harness.
