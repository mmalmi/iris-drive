using System;
using System.IO;
using System.Security.Cryptography;
using System.Text;

namespace IrisDrive.WindowsShell;

internal static class WindowsProfileEnvironment
{
    private static string DefaultConfigDirectory => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "iris-drive");

    public static string ConfigDirectory
    {
        get
        {
            var configured = Environment.GetEnvironmentVariable("IRIS_DRIVE_CONFIG_DIR");
            if (string.IsNullOrWhiteSpace(configured))
            {
                configured = Environment.GetEnvironmentVariable("IRIS_DRIVE_DEV_VM_WINDOWS_CONFIG_DIR");
            }
            return string.IsNullOrWhiteSpace(configured)
                ? DefaultConfigDirectory
                : Path.GetFullPath(Environment.ExpandEnvironmentVariables(configured.Trim()));
        }
    }

    public static bool IsAlternateConfig => !string.Equals(
        NormalizeDirectory(ConfigDirectory), NormalizeDirectory(DefaultConfigDirectory),
        StringComparison.Ordinal);

    private static string ProfileSuffix => IsAlternateConfig
        ? "." + Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(
            NormalizeDirectory(ConfigDirectory)))).ToLowerInvariant()
        : "";

    public static string MutexName => "IrisDrive.WindowsShell" + ProfileSuffix;
    public static string LaunchPipeName => "IrisDrive.WindowsShell.LaunchArgs" + ProfileSuffix;
    public static string SyncRootIdentity => "iris-drive:main" + ProfileSuffix;

    public static string? CloudRootPath
    {
        get
        {
            var configured = Environment.GetEnvironmentVariable("IRIS_DRIVE_WINDOWS_CLOUD_ROOT")?.Trim();
            if (string.IsNullOrEmpty(configured))
            {
                return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Iris Drive");
            }
            return configured.ToLowerInvariant() switch
            {
                "0" or "false" or "off" or "disabled" or "none" => null,
                _ => Path.GetFullPath(configured),
            };
        }
    }

    private static string NormalizeDirectory(string path) => Path.GetFullPath(path)
        .TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
        .ToUpperInvariant();
}
