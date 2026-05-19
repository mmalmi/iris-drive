# macos

Native macOS File Provider scaffold for Iris Drive.

This is the Apple-backed "drive folder" path from `docs/DESIGN.md`: a containing
app registers a File Provider domain, and the `IrisDriveFileProvider` extension
will eventually bridge Finder-visible file operations to the shared Rust app
core.

## Development build

```bash
just macos-xcodeproj
just macos-build
```

`macos-build` passes `CODE_SIGNING_ALLOWED=NO`, so it verifies the Swift target
and plist structure without requiring a local provisioning profile.

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
