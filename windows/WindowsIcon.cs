using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace IrisDrive.WindowsShell;

internal static class WindowsIcon
{
    public static System.Drawing.Icon TrayIcon()
    {
        var iconPath = PackagedIconPath();
        if (File.Exists(iconPath))
        {
            return new System.Drawing.Icon(iconPath);
        }

        var processPath = Environment.ProcessPath;
        if (!string.IsNullOrWhiteSpace(processPath) && File.Exists(processPath))
        {
            var icon = System.Drawing.Icon.ExtractAssociatedIcon(processPath);
            if (icon is not null)
            {
                return icon;
            }
        }

        return System.Drawing.SystemIcons.Application;
    }

    public static ImageSource? LoadWindowIcon()
    {
        var iconPath = PackagedIconPath();
        if (!File.Exists(iconPath))
        {
            return null;
        }

        return BitmapFrame.Create(new Uri(iconPath, UriKind.Absolute));
    }

    public static void RefreshShortcutIcons()
    {
        var processPath = Environment.ProcessPath;
        var iconPath = PackagedIconPath();
        if (string.IsNullOrWhiteSpace(processPath) || !File.Exists(processPath) || !File.Exists(iconPath))
        {
            return;
        }

        var shellType = Type.GetTypeFromProgID("WScript.Shell");
        if (shellType is null)
        {
            return;
        }

        object? shell = null;
        try
        {
            shell = Activator.CreateInstance(shellType);
            if (shell is null)
            {
                return;
            }

            foreach (var shortcutPath in ShortcutSearchPaths())
            {
                RefreshShortcutIcon(shell, shortcutPath, processPath, iconPath);
            }
        }
        catch
        {
        }
        finally
        {
            ReleaseComObject(shell);
        }
    }

    private static string PackagedIconPath()
    {
        return Path.Combine(AppContext.BaseDirectory, "IrisDrive.ico");
    }

    private static IEnumerable<string> ShortcutSearchPaths()
    {
        foreach (var folder in new[]
                 {
                     Environment.SpecialFolder.DesktopDirectory,
                     Environment.SpecialFolder.CommonDesktopDirectory,
                     Environment.SpecialFolder.Programs,
                     Environment.SpecialFolder.CommonPrograms,
                 })
        {
            var path = Environment.GetFolderPath(folder);
            if (!Directory.Exists(path))
            {
                continue;
            }

            string[] shortcuts;
            try
            {
                shortcuts = Directory.GetFiles(path, "*.lnk", SearchOption.AllDirectories);
            }
            catch
            {
                continue;
            }

            foreach (var shortcut in shortcuts)
            {
                yield return shortcut;
            }
        }
    }

    private static void RefreshShortcutIcon(object shell, string shortcutPath, string processPath, string iconPath)
    {
        object? shortcut = null;
        try
        {
            shortcut = shell.GetType().InvokeMember(
                "CreateShortcut",
                BindingFlags.InvokeMethod,
                null,
                shell,
                [shortcutPath]);
            if (shortcut is null)
            {
                return;
            }

            var targetPath = shortcut.GetType().InvokeMember(
                "TargetPath",
                BindingFlags.GetProperty,
                null,
                shortcut,
                null) as string;
            if (!PathsMatch(targetPath, processPath))
            {
                return;
            }

            var iconLocation = shortcut.GetType().InvokeMember(
                "IconLocation",
                BindingFlags.GetProperty,
                null,
                shortcut,
                null) as string;
            if (iconLocation?.StartsWith(iconPath, StringComparison.OrdinalIgnoreCase) == true)
            {
                return;
            }

            shortcut.GetType().InvokeMember(
                "IconLocation",
                BindingFlags.SetProperty,
                null,
                shortcut,
                [$"{iconPath},0"]);
            shortcut.GetType().InvokeMember("Save", BindingFlags.InvokeMethod, null, shortcut, null);
        }
        catch
        {
        }
        finally
        {
            ReleaseComObject(shortcut);
        }
    }

    private static bool PathsMatch(string? left, string right)
    {
        if (string.IsNullOrWhiteSpace(left))
        {
            return false;
        }

        try
        {
            return string.Equals(
                Path.GetFullPath(left).TrimEnd(Path.DirectorySeparatorChar),
                Path.GetFullPath(right).TrimEnd(Path.DirectorySeparatorChar),
                StringComparison.OrdinalIgnoreCase);
        }
        catch
        {
            return string.Equals(left, right, StringComparison.OrdinalIgnoreCase);
        }
    }

    private static void ReleaseComObject(object? value)
    {
        if (value is not null && Marshal.IsComObject(value))
        {
            Marshal.FinalReleaseComObject(value);
        }
    }
}
