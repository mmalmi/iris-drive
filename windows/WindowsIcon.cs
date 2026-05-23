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
        var trayIcon = LoadIcon(iconPath, System.Windows.Forms.SystemInformation.SmallIconSize);
        if (trayIcon is not null)
        {
            return trayIcon;
        }

        var processPath = Environment.ProcessPath;
        if (!string.IsNullOrWhiteSpace(processPath) && File.Exists(processPath))
        {
            using var icon = System.Drawing.Icon.ExtractAssociatedIcon(processPath);
            var associatedIcon = CloneIcon(icon, System.Windows.Forms.SystemInformation.SmallIconSize);
            if (associatedIcon is not null)
            {
                return associatedIcon;
            }
        }

        return (System.Drawing.Icon)System.Drawing.SystemIcons.Application.Clone();
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

    private static System.Drawing.Icon? LoadIcon(string iconPath, System.Drawing.Size size)
    {
        if (!File.Exists(iconPath))
        {
            return null;
        }

        try
        {
            using var icon = new System.Drawing.Icon(iconPath, size);
            return (System.Drawing.Icon)icon.Clone();
        }
        catch
        {
            try
            {
                using var icon = new System.Drawing.Icon(iconPath);
                return (System.Drawing.Icon)icon.Clone();
            }
            catch
            {
                return null;
            }
        }
    }

    private static System.Drawing.Icon? CloneIcon(System.Drawing.Icon? icon, System.Drawing.Size size)
    {
        if (icon is null)
        {
            return null;
        }

        try
        {
            using var sizedIcon = new System.Drawing.Icon(icon, size);
            return (System.Drawing.Icon)sizedIcon.Clone();
        }
        catch
        {
            return (System.Drawing.Icon)icon.Clone();
        }
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
