using System;
using System.IO;

namespace IrisDrive.WindowsShell;

internal static class WindowsShellTrace
{
    private const string TracePathEnv = "IRIS_DRIVE_WINDOWS_SHELL_TRACE";

    public static void Write(string message)
    {
        var path = Environment.GetEnvironmentVariable(TracePathEnv);
        if (string.IsNullOrWhiteSpace(path))
        {
            return;
        }

        try
        {
            var directory = Path.GetDirectoryName(path);
            if (!string.IsNullOrWhiteSpace(directory))
            {
                Directory.CreateDirectory(directory);
            }

            File.AppendAllText(
                path,
                $"{DateTimeOffset.Now:O} pid={Environment.ProcessId} thread={Environment.CurrentManagedThreadId} {message}{Environment.NewLine}");
        }
        catch
        {
            // Smoke tracing must never affect normal app startup.
        }
    }
}
