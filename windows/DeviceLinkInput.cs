using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private async void LinkOwnerBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        await TrySubmitLinkOwnerAsync();
    }

    private async void LinkSubmit_Click(object sender, RoutedEventArgs e)
    {
        await TrySubmitLinkOwnerAsync();
    }

    private async void LinkOwnerBox_KeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
        {
            return;
        }
        e.Handled = true;
        await TrySubmitLinkOwnerAsync();
    }

    private async Task TrySubmitLinkOwnerAsync()
    {
        var owner = LinkOwnerBox.Text.Trim();
        if (string.IsNullOrEmpty(owner))
        {
            return;
        }
        if (!await service.IsCompleteLinkInputAsync(owner))
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
}
