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
signed. In that default mode the app data directory is
`macos/.build/AppData`, avoiding app-group container prompts. For the fully
entitled path, run with a configured Xcode account:

```bash
IRIS_DRIVE_DEVELOPMENT_TEAM=<team-id> IRIS_DRIVE_MACOS_SIGNING=development just run
```

For normal local development, copy `.env.local.example` to `.env.local` and set
the same values there. `scripts/macos-dev-app.sh` auto-loads `.env.local` as
defaults, while explicit shell variables still win. Keep `.env.local` for
machine-local development settings; keep `.env.release.local` for future
release-only signing or notarization inputs. If Xcode has no account signed in,
`.env.local` can also hold the optional `IRIS_DRIVE_ASC_AUTH_KEY_*` values for
`xcodebuild -allowProvisioningUpdates`.

`just macos-build` still exists as a compile-only check and passes
`CODE_SIGNING_ALLOWED=NO`, so it verifies the Swift target and plist structure
without requiring a local provisioning profile.

The app-launched daemon uses the shared app-group container for its config and
hashtree blocks and exposes the drive through the virtual provider/gateway
surface. The user-visible drive folder comes from the File Provider domain, not
from a separate `~/Iris Drive` directory. Unsigned/ad-hoc dev runs cannot mount
the real File Provider domain, and the app must fail visibly in that state
rather than opening a materialized normal folder. When the app-group container
is unavailable, dev builds use their own sandboxed Application Support runtime path
instead of hand-building a `~/Library/Group Containers` path, which would
trigger macOS privacy prompts for other apps' data.

## Smoke test

```bash
just smoke-macos
```

The smoke test builds the macOS app, launches it through LaunchServices, waits
for the app process, verifies the bundled `idrive daemon` starts, then tears
both down. It launches the app hidden, forces `IRIS_DRIVE_MACOS_SIGNING=none`,
and uses an isolated temporary app data directory so it does not mutate the
normal app-group state.

The menu-click check is opt-in because it opens Finder on the active desktop:

```bash
IRIS_DRIVE_MACOS_SMOKE_UI=1 just smoke-macos
```

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
