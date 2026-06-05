using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private async void LinkTargetBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        await TrySubmitLinkTargetAsync();
    }

    private async void LinkSubmit_Click(object sender, RoutedEventArgs e)
    {
        await TrySubmitLinkTargetAsync();
    }

    private async void LinkTargetBox_KeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
        {
            return;
        }
        e.Handled = true;
        await TrySubmitLinkTargetAsync();
    }

    private async Task TrySubmitLinkTargetAsync()
    {
        var target = LinkTargetBox.Text.Trim();
        if (string.IsNullOrEmpty(target))
        {
            return;
        }
        if (!await service.IsCompleteLinkInputAsync(target))
        {
            return;
        }
        if (submittedLinkTarget == target)
        {
            return;
        }
        submittedLinkTarget = target;
        await RunSetupAsync(() => service.LinkDeviceAsync(target));
    }
}
