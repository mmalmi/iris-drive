using System;
using System.Windows;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private async void LaunchOnStartup_Changed(object sender, RoutedEventArgs e)
    {
        if (settingsUpdating)
        {
            return;
        }

        var enabled = LaunchOnStartupCheckBox.IsChecked == true;
        try
        {
            StartupService.SetLaunchOnStartup(enabled);
            await service.SetLaunchOnStartupAsync(enabled);
            NoticeText.Text = enabled ? "Launch on startup enabled" : "Launch on startup disabled";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
            await RefreshAsync();
        }
    }

    private void SyncLaunchOnStartup(bool enabled)
    {
        try
        {
            StartupService.SyncLaunchOnStartup(enabled);
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }
}
