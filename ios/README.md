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
