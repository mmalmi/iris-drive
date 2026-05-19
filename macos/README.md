# macos

Native macOS File Provider scaffold for Iris Drive.

This is the Apple-backed "drive folder" path from `docs/DESIGN.md`: a containing
app registers a File Provider domain, and the `IrisDriveFileProvider` extension
will eventually bridge Finder-visible file operations to the shared Rust app
core. The containing app also starts the bundled `idrive daemon`, so there is
one app entrypoint in development.

## Development build

```bash
just run
```

`just run` builds the `idrive` helper, builds this app, copies `idrive` into the
app bundle, opens the app, registers the File Provider domain, and starts the
daemon. By default it uses a no-provisioning dev launch because macOS rejects
restricted File Provider/app-group entitlements when they are only ad-hoc
signed. For the fully entitled path, run with a configured Xcode account:

```bash
IRIS_DRIVE_DEVELOPMENT_TEAM=<team-id> IRIS_DRIVE_MACOS_SIGNING=development just run
```

`just macos-build` still exists as a compile-only check and passes
`CODE_SIGNING_ALLOWED=NO`, so it verifies the Swift target and plist structure
without requiring a local provisioning profile.

The app-launched daemon uses the shared app-group container for its config and
working tree. The user-visible drive folder should come from the File Provider
domain, not from a separate `~/Iris Drive` directory.

## Smoke test

```bash
just smoke-macos
```

The smoke test builds the macOS app, launches it through LaunchServices, waits
for the app process, verifies the bundled `idrive daemon` starts, then tears both
down. It uses an isolated temporary app data directory so it does not mutate the
normal app-group state.

## Entitlements

Development entitlements:

- `IrisDriveMac.entitlements`
- `FileProvider/FileProvider.entitlements`

Both include:

- app sandbox
- app group `group.to.iris.drive`
- outbound network client access
- `com.apple.developer.fileprovider.testing-mode`

Release entitlements:

- `Release.entitlements`
- `FileProvider/Release.entitlements`

Those intentionally omit `com.apple.developer.fileprovider.testing-mode`; Apple
requires that testing-only entitlement to be removed before TestFlight or Mac
App Store submission.

Bundle IDs:

- app: `to.iris.drive.macos`
- extension: `to.iris.drive.macos.FileProvider`
