# Desktop GUI parity

Linux is the current behavior reference for the desktop control panel. macOS and
Windows should expose the same user-visible sync controls even when their OS
file-provider backends differ.

| Capability | Linux GTK | macOS SwiftUI | Windows WPF |
| --- | --- | --- | --- |
| First-run create profile | Yes | Yes | Yes |
| Restore owner profile | Yes | Yes | Yes |
| Link request handoff | Yes | Yes | Yes |
| Copy owner/device keys | Yes | Yes | Yes |
| Approve linked device from request link | Yes | Yes | Yes |
| Start/stop/restart sync daemon | Yes | Yes | Yes |
| Auto-scan local drive folder | Yes | Yes | Yes |
| Open drive folder | Yes | Yes | Yes |
| Copy/open snapshot link | Yes | Yes | Yes |
| Devices list and auth state | Yes | Yes | Yes |
| Device online/sync status | Yes | Planned | Yes |
| Owner device revoke control | Yes | Planned | Yes |
| Relay add/reset controls | Yes | Yes | Yes |
| Direct FIPS block sync | Yes | Yes | Yes |
| Blossom fallback server list | Yes | Yes | Yes |
| Hashtree config/block/root paths | Yes | Yes | Yes |
| Tray/menu-bar control | Yes | Yes | Yes |
| Close to tray/menu-bar | Yes | Yes | Yes |
| Native OS file-provider mount | Backing folder today; FUSE planned | FileProvider scaffold | Backing folder today; WinFsp planned |

## Desktop test target

The minimum parity smoke for Linux/Windows is:

1. Create an owner profile on one VM.
2. Link the other VM as a secondary device from the GUI and copy its request link.
3. Paste the request link into the owner GUI and approve it.
4. Confirm both Devices tabs show the authorized peer and its FIPS online/sync state.
5. Create a file in each drive folder.
6. Confirm each side sees both files after daemon sync.

The same flow is valid for macOS once the visible app has the latest control
panel build.

Block replication now tries direct hashtree-over-FIPS transfer between
authorized Iris Drive instances first. Blossom remains configured as a
fallback/cache path, not the primary sync transport.
