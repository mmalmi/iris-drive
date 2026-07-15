using Microsoft.Win32;

namespace IrisDrive.WindowsShell;

public static class StartupService
{
    public const string HiddenLaunchArgument = "--hidden";
    private const string RunKeyPath = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string AppName = "Iris Drive";

    public static void SyncLaunchOnStartup(bool enabled)
    {
        SetLaunchOnStartup(enabled);
    }

    public static void SetLaunchOnStartup(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(RunKeyPath);
        if (enabled)
        {
            key.SetValue(AppName, StartupCommand());
        }
        else
        {
            key.DeleteValue(AppName, throwOnMissingValue: false);
        }
    }

    public static bool IsHiddenLaunch(IEnumerable<string> arguments)
    {
        return arguments.Any(argument =>
            string.Equals(argument, HiddenLaunchArgument, StringComparison.OrdinalIgnoreCase));
    }

    private static string StartupCommand()
    {
        var exe = Environment.ProcessPath;
        if (string.IsNullOrWhiteSpace(exe))
        {
            throw new InvalidOperationException("App executable was not found.");
        }
        return $"\"{exe}\" {HiddenLaunchArgument}";
    }
}
