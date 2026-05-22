# Windows Native Shell

WPF shell for Iris Drive. It mirrors the Linux GTK control panel over the
shared `idrive` CLI:

- first-run create/restore/link flows
- device key copy and owner-side approval
- sync start/stop/restart
- drive folder and snapshot link actions
- devices, network, hashtree, and settings pages
- Windows tray icon with show/open/start/stop/restart/quit actions

Build from the repo root on Windows:

```powershell
cargo build -p idrive
dotnet build .\windows\IrisDrive.Windows.csproj
dotnet run --project .\windows\IrisDrive.Windows.csproj
```

The app looks for `idrive.exe` next to the app, under `target\debug`, under
`target\release`, or at `IRIS_DRIVE_CLI`. The visible drive folder is
`%USERPROFILE%\Iris Drive`.

See [`docs/PARITY.md`](../docs/PARITY.md) for the current desktop parity matrix.
