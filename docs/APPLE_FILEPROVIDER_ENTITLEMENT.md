# Apple File Provider entitlement request

Iris Drive's macOS App Store path uses Apple's File Provider framework so the
drive appears under `~/Library/CloudStorage/` and in Finder's sidebar.

Development can use `com.apple.developer.fileprovider.testing-mode` on local
devices. Distribution cannot: Apple documents this entitlement as testing-only
and requires it to be removed before TestFlight or Mac App Store submission.

References:

- <https://developer.apple.com/documentation/BundleResources/Entitlements/com.apple.developer.fileprovider.testing-mode>
- <https://developer.apple.com/documentation/FileProvider/synchronizing-files-using-file-provider-extensions>

## Targets

- App bundle ID: `to.iris.drive.macos`
- File Provider extension bundle ID: `to.iris.drive.macos.FileProvider`
- App group: `group.to.iris.drive`

## Request draft

Iris Drive is a privacy-first file synchronization app built on content-addressed
encrypted storage. We need File Provider support on macOS so users can access
their synced files through Finder and `~/Library/CloudStorage/` using Apple's
standard file-provider storage model.

The File Provider extension will:

- expose a user-authorized Iris Drive domain in Finder
- lazily materialize encrypted files on demand
- propagate local creates, edits, moves, and deletes back to the user's private
  Iris Drive root
- keep sync metadata in an app group shared with the containing app

Iris Drive does not use the entitlement to access unrelated user files. The
extension's visible file hierarchy is limited to the user's Iris Drive domain
and shares they explicitly accept.
