using System;
using System.Linq;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private async void LinkOwnerBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        await TrySubmitLinkOwnerAsync(force: false);
    }

    private async void LinkSubmit_Click(object sender, RoutedEventArgs e)
    {
        await TrySubmitLinkOwnerAsync(force: true);
    }

    private async void LinkOwnerBox_KeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
        {
            return;
        }
        e.Handled = true;
        await TrySubmitLinkOwnerAsync(force: true);
    }

    private async Task TrySubmitLinkOwnerAsync(bool force)
    {
        var owner = LinkOwnerBox.Text.Trim();
        if (string.IsNullOrEmpty(owner))
        {
            return;
        }
        if (!force && !IsCompleteDeviceLinkOwnerInput(owner))
        {
            return;
        }
        if (submittedLinkOwner == owner)
        {
            return;
        }
        submittedLinkOwner = owner;
        await RunSetupAsync(() => service.LinkDeviceAsync(owner));
    }

    private static bool IsCompleteDeviceLinkOwnerInput(string value)
    {
        var trimmed = value.Trim();
        if (trimmed.Length == 0 || trimmed.Any(char.IsWhiteSpace))
        {
            return false;
        }
        var lower = trimmed.ToLowerInvariant();
        if (lower.StartsWith("npub1", StringComparison.Ordinal))
        {
            return lower.Length >= 63;
        }
        if (lower.Length == 64 && lower.All(Uri.IsHexDigit))
        {
            return true;
        }
        foreach (var prefix in new[]
        {
            "iris-drive://invite/",
            "iris-drive:/invite/",
            "https://drive.iris.to/invite/",
        })
        {
            if (lower.StartsWith(prefix, StringComparison.Ordinal))
            {
                return lower[prefix.Length..].Length >= 32;
            }
        }
        return (lower.StartsWith("iris-drive://link-device?", StringComparison.Ordinal)
                || lower.StartsWith("iris-drive:/link-device?", StringComparison.Ordinal)
                || lower.StartsWith("https://drive.iris.to/link-device?", StringComparison.Ordinal))
            && lower.Contains("owner=", StringComparison.Ordinal)
            && lower.Contains("admin=", StringComparison.Ordinal)
            && lower.Contains("secret=", StringComparison.Ordinal);
    }
}
