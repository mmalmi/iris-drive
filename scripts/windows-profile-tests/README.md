Run on Windows with the .NET 8 desktop SDK:

```powershell
dotnet run --project scripts/windows-profile-tests/WindowsProfileTests.csproj -p:UseSharedCompilation=false
```

This links the production shell IPC and Cloud Files code without launching a
window. It checks default names, normalized profile identities, real secondary
launch delivery between isolated named pipes, provider paths and metadata,
disabled-provider behavior, and automatic startup registration policy. It never
opens the default user pipe, registers a provider, or writes the registry.

For concurrent profiles, set both `IRIS_DRIVE_CONFIG_DIR` and
`IRIS_DRIVE_WINDOWS_CLOUD_ROOT` to separate profile and provider directories.
The provider remains Windows Cloud Files. An absent or empty cloud-root override
keeps the ordinary user root; `0`, `false`, `off`, `disabled`, or `none` disables
the provider, matching the daemon's existing setting.
