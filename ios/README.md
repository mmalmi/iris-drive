# ios

Native SwiftUI shell for Iris Drive on iOS.

The app is a mobile control surface with the same main destinations as the
desktop shells: My Drive, Devices, Shares, Backup, and Settings. The bundled File
Provider extension exposes a virtual Iris Drive domain to the Files app; it
does not create a user-visible normal folder.

## Development build

```bash
just ios-build
```

This generates the Xcode project with XcodeGen and compiles the app for an
available iOS simulator without requiring local provisioning.

## Simulator smoke

```bash
just ios-smoke
```

The smoke test boots an available iPhone simulator, builds the app, installs
it, launches it, and verifies that the app container is available.

## Multidevice e2e

```bash
just e2e-4devices
```

The four-device wrapper runs the iOS simulator smoke on the configured iOS host
and then adds an `ios` peer to the existing Ubuntu, macOS, and Windows sync
matrix. The iOS peer uses the real `idrive` daemon and provider commands in the
test harness; no normal-folder substitute is mounted for the mobile device.

## Sharing from the Files app

The Files app has no "Share with Iris Drive" action on folders; sharing starts
from the app's Shares tab. A File Provider custom action
(`NSFileProviderCustomAction`) runs inside the non-UI extension, which can
neither show a sheet nor open the app: iOS gives extensions no
`UIApplication`, and a File Provider UI action extension's context only
completes or cancels the request. Posting a notification for the person to tap
would be a detour, not a share flow.

A clean version would be a File Provider UI action extension that shows the
share sheet (View or Edit, QR, Copy link) inside Files and creates the share
through Rust core itself. That is a new extension target with its own access
to the profile in the app group, so it is left out until it can be built and
checked on a device.
