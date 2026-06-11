# Windows Native Shell

WPF shell for Iris Drive. It mirrors the Linux GTK control panel over the
shared `idrive` CLI:

- first-run create/restore/link flows
- Device key copy and owner-side approval
- sync start/stop/restart
- drive folder and snapshot link actions
- Devices, shares, network, and settings pages
- shared-folder create, invite-by-evidence, accept invite, shortcut, repair,
  member role, and revoke actions through Rust app-core
- local profile logout
- Windows tray icon with show/open/sync/quit actions
- self-contained `win-x64` publish output for shortcuts/installers

Build from the repo root on Windows:

```powershell
cargo build -p idrive
dotnet build .\windows\IrisDrive.Windows.csproj
dotnet run --project .\windows\IrisDrive.Windows.csproj
```

Publish a runnable Windows app without requiring the .NET Desktop Runtime:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\windows-publish.ps1 -Configuration Debug -DesktopShortcut -StopRunningApp
```

The publish script builds `idrive.exe`, publishes the WPF shell self-contained
for `win-x64`, copies `idrive.exe` next to `IrisDrive.exe`, and can recreate the
desktop shortcut with the packaged Iris Drive icon.

Build the release installer used by `scripts/local-release.mjs`:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\windows-publish.ps1 -Configuration Release -Installer -Tag v0.1.0 -OutputDir dist
```

The installer path requires Inno Setup 6 (`iscc` / `ISCC.exe`) on the Windows
builder and writes `iris-drive-v...-windows-x64-setup.exe` into the output
directory.

The app looks for `idrive.exe` next to the app, under `target\debug`, under
`target\release`, or at `IRIS_DRIVE_CLI`. It starts the daemon with the shared
loopback gateway/resolver enabled by default and opens a native Windows drive
folder instead of relying on the Windows WebClient redirector.

The **Open Drive Folder** action registers `%USERPROFILE%\Iris Drive` as an
Iris Drive Cloud Files sync root when the Windows Cloud Files API is available.
It pre-populates the provider namespace as Cloud Files placeholders from
`idrive provider list`, keeps a `CfConnectSyncRoot` connection alive while the
shell is running, and hydrates file reads through `idrive provider read`.

See [`docs/PARITY.md`](../docs/PARITY.md) for the current desktop parity matrix.
