using System;
using System.Diagnostics;
using System.IO;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Threading;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private IrisDriveUpdateProgress updateProgress = IrisDriveUpdateProgress.Idle;
    private DispatcherTimer? updateProgressTimer;

    private async Task CheckUpdatesAsync(bool manual = true)
    {
        if (updateChecking || updateInstalling)
        {
            return;
        }

        var shouldInstall = false;
        updateChecking = true;
        if (manual)
        {
            updateStatus = "Checking for updates";
        }
        RenderUpdateState();

        try
        {
            var result = await service.CheckUpdateAsync();
            if (!string.IsNullOrWhiteSpace(result.Error))
            {
                throw new InvalidOperationException(result.Error);
            }

            latestUpdate = result;
            updateAvailable = result.Available;
            if (result.Available)
            {
                updateStatus = string.IsNullOrWhiteSpace(result.Asset)
                    ? $"Update {result.Tag} found without a Windows asset"
                    : $"Update {result.Tag} available";
                shouldInstall =
                    AutoInstallUpdatesCheckBox.IsChecked == true &&
                    !string.IsNullOrWhiteSpace(result.Asset);
            }
            else
            {
                updateStatus = manual ? "Up to date" : "";
                updateProgress = IrisDriveUpdateProgress.Idle;
            }
        }
        catch (Exception error)
        {
            if (manual)
            {
                updateStatus = error.Message;
            }
        }
        finally
        {
            updateChecking = false;
            RenderUpdateState();
        }
        if (shouldInstall)
        {
            await InstallUpdateAsync();
        }
    }

    /// Every step after the press stays on screen until the installer opens or
    /// the install fails with its reason and Try Again.
    private async Task InstallUpdateAsync()
    {
        if (!CanInstallUpdate || updateInstalling)
        {
            return;
        }

        updateInstalling = true;
        ReportUpdateStage("finding_release");
        StartUpdateProgressPolling();
        try
        {
            var downloadDir = UpdateDownloadDirectory();
            Directory.CreateDirectory(downloadDir);
            var result = await service.DownloadUpdateAsync(downloadDir);
            if (!string.IsNullOrWhiteSpace(result.Error))
            {
                throw new InvalidOperationException(result.Error);
            }
            if (string.IsNullOrWhiteSpace(result.Path))
            {
                throw new InvalidOperationException("Updater did not return a downloaded file.");
            }

            ReportUpdateStage("opening_installer");
            if (!WindowsProfileEnvironment.IsTruthy(Environment.GetEnvironmentVariable("IRIS_DRIVE_UPDATE_SKIP_OPEN")))
            {
                _ = Process.Start(new ProcessStartInfo(result.Path) { UseShellExecute = true });
            }
            ReportUpdateStage("installer_opened");
        }
        catch (Exception error)
        {
            ReportUpdateStage("failed", error.Message);
        }
        finally
        {
            updateInstalling = false;
            updateProgressTimer?.Stop();
            RenderUpdateState();
        }
    }

    private void StartUpdateProgressPolling()
    {
        updateProgressTimer ??= CreateUpdateProgressTimer();
        updateProgressTimer.Start();
    }

    private DispatcherTimer CreateUpdateProgressTimer()
    {
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(250) };
        timer.Tick += (_, _) =>
        {
            if (!updateInstalling)
            {
                timer.Stop();
                return;
            }
            updateProgress = service.UpdateProgress();
            RenderUpdateState();
        };
        return timer;
    }

    private void ReportUpdateStage(string stage, string reason = "")
    {
        updateProgress = service.ReportUpdateStage(stage, reason);
        RenderUpdateState();
    }

    private void RenderUpdateState()
    {
        var shown = updateProgress.Shown;
        var fraction = updateProgress.Fraction;
        UpdateBanner.Visibility = updateAvailable || shown ? Visibility.Visible : Visibility.Collapsed;
        UpdateBannerText.Text = shown ? updateProgress.Text : UpdateStripeText();
        UpdateBannerText.ToolTip = shown ? updateProgress.Text : null;
        UpdateStatusText.Text = shown ? updateProgress.Text : updateStatus;
        foreach (var bar in new[] { UpdateBannerProgress, UpdateProgressBar })
        {
            bar.Visibility = updateProgress.Active ? Visibility.Visible : Visibility.Collapsed;
            bar.IsIndeterminate = fraction is null;
            bar.Value = (fraction ?? 0) * 100;
        }
        UpdateBannerAutoInstallCheckBox.Visibility = shown ? Visibility.Collapsed : Visibility.Visible;
        UpdateBannerInstallButton.Visibility =
            shown && !updateProgress.CanRetry ? Visibility.Collapsed : Visibility.Visible;
        UpdateBannerInstallText.Text = updateProgress.CanRetry ? "Try Again" : "Install";
        InstallUpdateText.Text = updateProgress.CanRetry ? "Try Again" : "Install Update";
        CheckUpdatesButton.IsEnabled = !updateChecking && !updateInstalling;
        InstallUpdateButton.IsEnabled = CanInstallUpdate;
        UpdateBannerInstallButton.IsEnabled = CanInstallUpdate;
        settingsUpdating = true;
        UpdateBannerAutoInstallCheckBox.IsChecked = AutoInstallUpdatesCheckBox.IsChecked;
        settingsUpdating = false;
    }

#if DEBUG
    /// `IRIS_DRIVE_WINDOWS_UPDATE_FIXTURE=<stage>[:reason]` starts with an
    /// update offered and this install step shown, worded by the core.
    private void ApplyUpdateFixture()
    {
        var value = Environment.GetEnvironmentVariable("IRIS_DRIVE_WINDOWS_UPDATE_FIXTURE");
        if (string.IsNullOrWhiteSpace(value))
        {
            return;
        }
        var parts = value.Split(':', 2);
        updateAvailable = true;
        latestUpdate = new IrisDriveUpdateResult(
            true, service.AppVersion, "0.1.47", "v0.1.47", "iris-drive-v0.1.47-windows-x64.msi",
            "", true, null, null, null, "");
        ReportUpdateStage(parts[0], parts.Length > 1 ? parts[1] : "");
    }
#endif

    private bool CanInstallUpdate =>
        updateAvailable &&
        latestUpdate is not null &&
        !string.IsNullOrWhiteSpace(latestUpdate.Asset) &&
        !updateChecking &&
        !updateInstalling;

    private string UpdateVersionText =>
        latestUpdate is null || string.IsNullOrWhiteSpace(latestUpdate.Tag)
            ? "update"
            : latestUpdate.Tag;

    private string UpdateStripeText()
    {
        var current = service.AppVersion;
        return string.IsNullOrWhiteSpace(current)
            ? $"Update available: {UpdateVersionText}"
            : $"Update available: {UpdateVersionText} (you're on {current})";
    }
}
