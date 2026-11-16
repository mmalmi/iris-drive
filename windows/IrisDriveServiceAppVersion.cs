using System;
using System.Reflection;

namespace IrisDrive.WindowsShell;

public sealed partial class IrisDriveService
{
    // IrisDrive.Windows.csproj stamps this from [workspace.package] version in the root Cargo.toml.
    // Settings shows it and the update check compares release tags against it.
    public string AppVersion => typeof(IrisDriveService).Assembly
        .GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion
        ?? throw new InvalidOperationException("IrisDrive.exe has no version from the workspace Cargo.toml.");
}
