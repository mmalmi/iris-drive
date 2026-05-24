using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Shapes;
using System.Windows.Threading;
using Forms = System.Windows.Forms;
using WpfApplication = System.Windows.Application;
using WpfBrush = System.Windows.Media.Brush;
using WpfBrushes = System.Windows.Media.Brushes;
using WpfButton = System.Windows.Controls.Button;
using WpfClipboard = System.Windows.Clipboard;
using WpfHorizontalAlignment = System.Windows.HorizontalAlignment;
using IOPath = System.IO.Path;
using WpfOrientation = System.Windows.Controls.Orientation;

namespace IrisDrive.WindowsShell;

public partial class MainWindow : Window
{
    private static readonly TimeSpan DriveFolderReconciliationInterval = TimeSpan.FromSeconds(2);
    private readonly IrisDriveService service = new();
    private readonly DispatcherTimer refreshTimer;
    private Process? daemon;
    private IrisDriveStatusData? currentStatus;
    private bool preparingDriveFolder;
    private string? preparedDriveRefreshKey;
    private DateTimeOffset lastDriveFolderReconciliationAt = DateTimeOffset.MinValue;
    private bool refreshing;
    private bool quitRequested;
    private Forms.NotifyIcon? trayIcon;

    public MainWindow()
    {
        InitializeComponent();
        Icon = WindowsIcon.LoadWindowIcon();
        CloseToTrayCheckBox.IsChecked = ReadCloseToTrayOnClose();
        refreshTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) };
        refreshTimer.Tick += async (_, _) => await RefreshAsync();
        SelectPage("Drive");
        RenderLoading();
    }

    private async void Window_Loaded(object sender, RoutedEventArgs e)
    {
        InstallTraySafely();
        _ = Task.Run(WindowsIcon.RefreshShortcutIcons);
        refreshTimer.Start();
        await RefreshAsync();
    }

    private void Window_Closing(object? sender, System.ComponentModel.CancelEventArgs e)
    {
        if (!quitRequested && CloseToTrayCheckBox.IsChecked == true && trayIcon is not null)
        {
            e.Cancel = true;
            Hide();
            return;
        }

        refreshTimer.Stop();
        trayIcon?.Dispose();
        StopDaemon();
        WpfApplication.Current.Shutdown();
    }

    private async Task RefreshAsync()
    {
        if (refreshing)
        {
            return;
        }

        refreshing = true;
        try
        {
            var status = await service.StatusAsync();
            currentStatus = status;
            if (!status.Initialized)
            {
                SetupRoot.Visibility = Visibility.Visible;
                MainRoot.Visibility = Visibility.Collapsed;
                SetupNotice.Text = "Setup needed";
                return;
            }

            var syncRunning = EnsureDaemonRunning(status);
            ScheduleDriveFolderRefresh(status);
            if (status.IsAwaitingLinkedApproval)
            {
                RenderAwaitingApproval(status, syncRunning, null);
                return;
            }

            SetupRoot.Visibility = Visibility.Collapsed;
            MainRoot.Visibility = Visibility.Visible;
            RenderStatus(status, syncRunning, null);
        }
        catch (Exception error)
        {
            SetupRoot.Visibility = Visibility.Collapsed;
            MainRoot.Visibility = Visibility.Visible;
            RenderUnavailable(error.Message);
        }
        finally
        {
            refreshing = false;
        }
    }

    private void RenderLoading()
    {
        SetupRoot.Visibility = Visibility.Collapsed;
        MainRoot.Visibility = Visibility.Visible;
        DriveTitle.Text = "My Drive";
        DriveMessage.Text = "Starting sync";
        StatusPill.Text = "Starting";
        FilesValue.Text = "0";
        BlocksValue.Text = "0";
        StorageValue.Text = "0 B";
        DevicesValue.Text = "0/0";
        NoticeText.Text = "";
        OwnerValue.Text = "-";
        DeviceValue.Text = "-";
        AuthValue.Text = "-";
    }

    private void RenderAwaitingApproval(
        IrisDriveStatusData status,
        bool syncRunning,
        string? notice)
    {
        SetupRoot.Visibility = Visibility.Visible;
        MainRoot.Visibility = Visibility.Collapsed;
        ShowSetupPanel(AwaitingPanel);
        AwaitingOwnerBox.Text = status.OwnerNpub ?? "";
        AwaitingDeviceBox.Text = status.DeviceNpub ?? "";
        DeviceRequestBox.Text = status.DeviceLinkRequestUrl ?? "";
        SetupNotice.Text = notice ?? (syncRunning ? "Waiting for approval" : "Sync stopped");
    }

    private void RenderStatus(IrisDriveStatusData status, bool syncRunning, string? notice)
    {
        DriveTitle.Text = status.DriveName;
        DriveMessage.Text = syncRunning ? "Running" : "Stopped";
        StatusPill.Text = syncRunning ? "Running" : "Stopped";
        FilesValue.Text = (status.FileCount > 0 ? status.FileCount : status.TopLevelEntries)
            .ToString(CultureInfo.InvariantCulture);
        BlocksValue.Text = status.LocalBlockCount.ToString(CultureInfo.InvariantCulture);
        StorageValue.Text = FormatBytes(status.LocalBlockBytes);
        DevicesValue.Text = $"{status.PublishedDeviceRoots}/{status.AuthorizedDeviceCount}";
        NoticeText.Text = notice ?? "";

        CopySnapshotButton.IsEnabled = !string.IsNullOrWhiteSpace(status.SnapshotUrl);
        OpenSnapshotButton.IsEnabled = !string.IsNullOrWhiteSpace(status.SnapshotUrl);
        StartButton.IsEnabled = !syncRunning;
        StopButton.IsEnabled = syncRunning;

        OwnerValue.Text = status.OwnerNpub ?? "-";
        DeviceValue.Text = status.DeviceNpub ?? "-";
        AuthValue.Text = status.AuthorizationState ?? "-";
        ApprovePanel.Visibility =
            status.HasOwnerSigningAuthority ? Visibility.Visible : Visibility.Collapsed;

        RenderDrives(status);
        RenderPeers(status);
        RenderBackups(status);
        RenderNetwork(status);
        UpdateTrayText(syncRunning);
    }

    private void RenderUnavailable(string message)
    {
        DriveTitle.Text = "My Drive";
        DriveMessage.Text = "Unavailable";
        StatusPill.Text = "Stopped";
        FilesValue.Text = "0";
        BlocksValue.Text = "0";
        StorageValue.Text = "0 B";
        DevicesValue.Text = "0/0";
        NoticeText.Text = message;
        OwnerValue.Text = "-";
        DeviceValue.Text = "-";
        AuthValue.Text = "-";
        CopySnapshotButton.IsEnabled = false;
        OpenSnapshotButton.IsEnabled = false;
        StartButton.IsEnabled = true;
        StopButton.IsEnabled = false;
        DrivesList.Items.Clear();
        PeersList.Items.Clear();
        BackupsList.Items.Clear();
        RelaysList.Items.Clear();
        BlossomList.Items.Clear();
    }

    private void RenderDrives(IrisDriveStatusData status)
    {
        DrivesList.Items.Clear();
        foreach (var drive in status.Drives)
        {
            DrivesList.Items.Add(Row(drive.Name, drive.Path, drive.State));
        }
    }

    private void RenderPeers(IrisDriveStatusData status)
    {
        PeersList.Items.Clear();
        if (status.Peers.Count == 0)
        {
            PeersList.Items.Add(Row("No authorized devices", "", ""));
            return;
        }

        foreach (var peer in status.Peers)
        {
            PeersList.Items.Add(PeerListRow(peer));
        }
    }

    private void RenderBackups(IrisDriveStatusData status)
    {
        BackupsList.Items.Clear();
        if (status.BackupTargets.Count == 0)
        {
            BackupsList.Items.Add(Row("No backup targets", "", ""));
            return;
        }

        foreach (var target in status.BackupTargets)
        {
            BackupsList.Items.Add(Row(target.Title, target.Subtitle, target.State));
        }
    }

    private Border PeerListRow(PeerRow peer)
    {
        var titleBlock = new TextBlock
        {
            Text = peer.Title,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        var stack = new StackPanel { Orientation = WpfOrientation.Vertical };
        stack.Children.Add(titleBlock);

        if (!string.IsNullOrWhiteSpace(peer.Subtitle))
        {
            stack.Children.Add(new TextBlock
            {
                Text = peer.Subtitle,
                Foreground = (WpfBrush)WpfApplication.Current.Resources["IrisMutedBrush"],
                TextTrimming = TextTrimming.CharacterEllipsis,
                FontSize = 12,
            });
        }

        var dot = new Ellipse
        {
            Width = 8,
            Height = 8,
            Fill = PeerConnectivityBrush(peer),
            VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = WpfHorizontalAlignment.Left,
            ToolTip = peer.State,
        };

        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(16) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(dot);
        Grid.SetColumn(stack, 1);
        grid.Children.Add(stack);

        if (peer.CanRevoke)
        {
            var revoke = new WpfButton
            {
                Content = new TextBlock { Text = "\uE74D", Style = (Style)FindResource("IconGlyph") },
                Style = (Style)FindResource("IconButton"),
                Tag = peer.DeviceNpub,
                Margin = new Thickness(8, 0, 0, 0),
                ToolTip = "Revoke device",
            };
            revoke.Click += RevokeDevice_Click;
            Grid.SetColumn(revoke, 2);
            grid.Children.Add(revoke);
        }

        return new Border
        {
            Padding = new Thickness(12, 9, 12, 9),
            Child = grid,
        };
    }

    private static WpfBrush PeerConnectivityBrush(PeerRow peer)
    {
        return (WpfBrush)WpfApplication.Current.Resources[
            peer.IsOnline ? "IrisSuccessBrush" : "IrisMutedBrush"];
    }

    private void RenderNetwork(IrisDriveStatusData status)
    {
        FipsList.Items.Clear();
        FipsList.Items.Add(Row("State", "", status.Fips.State));
        FipsList.Items.Add(Row("Roster FIPS", status.Fips.RosterText, ""));
        FipsList.Items.Add(Row("Other FIPS", status.Fips.OtherPeerCount.ToString(), ""));
        FipsList.Items.Add(Row("Connected", status.Fips.ConnectedPeerCount.ToString(), ""));
        if (!string.IsNullOrWhiteSpace(status.Fips.EndpointNpub))
        {
            FipsList.Items.Add(Row("Endpoint", status.Fips.EndpointNpub, ""));
        }

        if (!string.IsNullOrWhiteSpace(status.Fips.DiscoveryScope))
        {
            FipsList.Items.Add(Row("Scope", status.Fips.DiscoveryScope, ""));
        }

        if (!string.IsNullOrWhiteSpace(status.Fips.Error))
        {
            FipsList.Items.Add(Row("Error", status.Fips.Error, ""));
        }

        BlossomList.Items.Clear();
        if (status.BlossomServers.Count == 0)
        {
            BlossomList.Items.Add(Row("No Blossom servers", "", ""));
        }
        else
        {
            foreach (var server in status.BlossomServers)
            {
                BlossomList.Items.Add(Row(server, "", ""));
            }
        }

        RelaysList.Items.Clear();
        if (status.Relays.Count == 0)
        {
            RelaysList.Items.Add(Row("No relays", "", ""));
            return;
        }

        foreach (var relay in status.Relays)
        {
            var state = status.RelayStatuses.TryGetValue(relay, out var value) ? value : "saved";
            RelaysList.Items.Add(Row(relay, "", state));
        }
    }

    private static Border Row(string title, string subtitle, string state)
    {
        var titleBlock = new TextBlock
        {
            Text = title,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        var stack = new StackPanel { Orientation = WpfOrientation.Vertical };
        stack.Children.Add(titleBlock);

        if (!string.IsNullOrWhiteSpace(subtitle))
        {
            stack.Children.Add(new TextBlock
            {
                Text = subtitle,
                Foreground = (WpfBrush)WpfApplication.Current.Resources["IrisMutedBrush"],
                TextTrimming = TextTrimming.CharacterEllipsis,
                FontSize = 12,
            });
        }

        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(stack);

        if (!string.IsNullOrWhiteSpace(state))
        {
            var stateBlock = new TextBlock
            {
                Text = state,
                Foreground = (WpfBrush)WpfApplication.Current.Resources["IrisMutedBrush"],
                Margin = new Thickness(12, 0, 0, 0),
                VerticalAlignment = VerticalAlignment.Center,
                TextTrimming = TextTrimming.CharacterEllipsis,
            };
            Grid.SetColumn(stateBlock, 1);
            grid.Children.Add(stateBlock);
        }

        return new Border
        {
            Padding = new Thickness(12, 9, 12, 9),
            Child = grid,
        };
    }

    private bool EnsureDaemonRunning(IrisDriveStatusData status)
    {
        if (daemon is { HasExited: false } || service.DaemonLockIsRunning(status))
        {
            return true;
        }

        try
        {
            daemon = service.StartDaemonProcess();
            NoticeText.Text = "Sync started";
            return true;
        }
        catch (Exception error)
        {
            NoticeText.Text = $"Could not start sync: {error.Message}";
            return false;
        }
    }

    private void StopDaemon()
    {
        var stopped = false;
        if (daemon is { HasExited: false })
        {
            KillProcess(daemon);
            daemon = null;
            stopped = true;
        }

        if (currentStatus is not null)
        {
            var pid = service.DaemonLockPid(currentStatus);
            if (pid.HasValue && IrisDriveService.ProcessIsRunning(pid.Value))
            {
                try
                {
                    using var process = Process.GetProcessById(pid.Value);
                    KillProcess(process);
                    stopped = true;
                }
                catch
                {
                    // Process may have exited between lock read and kill.
                }
            }
        }

        if (stopped)
        {
            NoticeText.Text = "Sync stopped";
        }
    }

    private static void KillProcess(Process process)
    {
        try
        {
            process.Kill(entireProcessTree: true);
            process.WaitForExit(1500);
        }
        catch
        {
            // Best effort; stale lock handling will recover on the next start.
        }
    }

    private async void Start_Click(object sender, RoutedEventArgs e)
    {
        if (currentStatus is not null)
        {
            EnsureDaemonRunning(currentStatus);
        }
        await RefreshAsync();
    }

    private async void Stop_Click(object sender, RoutedEventArgs e)
    {
        StopDaemon();
        await RefreshAsync();
    }

    private async void Restart_Click(object sender, RoutedEventArgs e)
    {
        StopDaemon();
        if (currentStatus is not null)
        {
            EnsureDaemonRunning(currentStatus);
        }
        await RefreshAsync();
    }

    private async void OpenDrive_Click(object sender, RoutedEventArgs e)
    {
        await OpenDriveMountAsync();
    }

    private async Task OpenDriveMountAsync()
    {
        if (currentStatus is { Initialized: true } status && !EnsureDaemonRunning(status))
        {
            NoticeText.Text = "Could not start sync";
            return;
        }

        if (currentStatus is { Initialized: true })
        {
            await Task.Delay(500);
            try
            {
                currentStatus = await service.StatusAsync();
            }
            catch
            {
                // Keep the last known status; the native drive folder path is deterministic.
            }
        }

        if (currentStatus is { Initialized: true })
        {
            try
            {
                var driveFolder = await service.PrepareDriveFolderAsync();
                preparedDriveRefreshKey = DriveFolderFullyPrepared(driveFolder)
                    ? currentStatus?.ProviderRefreshKey
                    : null;
                lastDriveFolderReconciliationAt = DateTimeOffset.UtcNow;
                if (driveFolder.NativeSyncRootReady)
                {
                    service.OpenPath(driveFolder.Path);
                }

                NoticeText.Text = driveFolder.Warning ??
                    (driveFolder.NativeSyncRootReady
                        ? "Opening drive folder"
                        : "Windows Cloud Files unavailable");
            }
            catch (Exception error)
            {
                NoticeText.Text = $"Could not open drive folder: {error.Message}";
            }
            return;
        }

        NoticeText.Text = "Setup needed";
    }

    private void OpenDriveMount()
    {
        _ = OpenDriveMountAsync();
    }

    private void ScheduleDriveFolderRefresh(IrisDriveStatusData status)
    {
        if (!status.Initialized || string.IsNullOrWhiteSpace(status.ProviderRefreshKey))
        {
            return;
        }

        var reconciliationDue =
            DateTimeOffset.UtcNow - lastDriveFolderReconciliationAt >= DriveFolderReconciliationInterval;
        WindowsCloudFiles.DebugLog(
            $"schedule prepared={preparedDriveRefreshKey == status.ProviderRefreshKey} " +
            $"due={reconciliationDue} preparing={preparingDriveFolder}");
        if (preparingDriveFolder ||
            (preparedDriveRefreshKey == status.ProviderRefreshKey && !reconciliationDue))
        {
            return;
        }

        preparingDriveFolder = true;
        _ = Task.Run(async () =>
        {
            try
            {
                var driveFolder = await service.PrepareDriveFolderAsync();
                await Dispatcher.InvokeAsync(() =>
                {
                    preparedDriveRefreshKey = DriveFolderFullyPrepared(driveFolder)
                        ? status.ProviderRefreshKey
                        : null;
                    lastDriveFolderReconciliationAt = DateTimeOffset.UtcNow;
                });
            }
            catch
            {
            }
            finally
            {
                await Dispatcher.InvokeAsync(() => preparingDriveFolder = false);
            }
        });
    }

    private static bool DriveFolderFullyPrepared(DriveFolderPreparation driveFolder) =>
        driveFolder.NativeSyncRootReady && driveFolder.SkippedLocalItemCount == 0;

    private void CopySnapshot_Click(object sender, RoutedEventArgs e)
    {
        CopyText(currentStatus?.SnapshotUrl, "Snapshot copied");
    }

    private void OpenSnapshot_Click(object sender, RoutedEventArgs e)
    {
        if (!string.IsNullOrWhiteSpace(currentStatus?.SnapshotUrl))
        {
            service.OpenUri(currentStatus.SnapshotUrl);
        }
    }

    private void CopyOwner_Click(object sender, RoutedEventArgs e)
    {
        CopyText(currentStatus?.OwnerNpub, "Owner key copied");
    }

    private void CopyDevice_Click(object sender, RoutedEventArgs e)
    {
        CopyText(currentStatus?.DeviceNpub, "Device key copied");
    }

    private void CopyDeviceRequest_Click(object sender, RoutedEventArgs e)
    {
        CopySetupText(currentStatus?.DeviceLinkRequestUrl, "Request copied");
    }

    private async void ApproveDevice_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.ApproveDeviceAsync(ApproveDeviceBox.Text, ApproveLabelBox.Text);
            ApproveDeviceBox.Clear();
            ApproveLabelBox.Clear();
            StopDaemon();
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            NoticeText.Text = "Device approved";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void RevokeDevice_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string deviceNpub })
        {
            return;
        }

        try
        {
            await service.RevokeDeviceAsync(deviceNpub);
            StopDaemon();
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            NoticeText.Text = "Device revoked";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void AddRelay_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.AddRelayAsync(RelayBox.Text);
            RelayBox.Clear();
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void ResetRelays_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.ResetRelaysAsync();
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void AddBackup_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.AddBackupTargetAsync(BackupTargetBox.Text, BackupLabelBox.Text);
            BackupTargetBox.Clear();
            BackupLabelBox.Clear();
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void SyncBackups_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.SyncBackupsAsync();
            NoticeText.Text = "Backups synced";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private void CopyText(string? value, string message)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            NoticeText.Text = "Nothing to copy";
            return;
        }

        WpfClipboard.SetText(value);
        NoticeText.Text = message;
    }

    private void CopySetupText(string? value, string message)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            SetupNotice.Text = "Nothing to copy";
            return;
        }

        WpfClipboard.SetText(value);
        SetupNotice.Text = message;
    }

    private async void CreateSubmit_Click(object sender, RoutedEventArgs e)
    {
        await RunSetupAsync(() => service.CreateProfileAsync(CreateLabelBox.Text));
    }

    private async void RestoreSubmit_Click(object sender, RoutedEventArgs e)
    {
        await RunSetupAsync(() => service.RestoreProfileAsync(
            RestoreSecretBox.Password,
            RestoreLabelBox.Text));
    }

    private void RestoreSecretBox_PasswordChanged(object sender, RoutedEventArgs e)
    {
        RestoreSecretPlaceholder.Visibility = string.IsNullOrEmpty(RestoreSecretBox.Password)
            ? Visibility.Visible
            : Visibility.Collapsed;
    }

    private async void LinkSubmit_Click(object sender, RoutedEventArgs e)
    {
        await RunSetupAsync(() => service.LinkDeviceAsync(LinkOwnerBox.Text, LinkLabelBox.Text));
    }

    private async Task RunSetupAsync(Func<Task> operation)
    {
        SetSetupEnabled(false);
        SetupNotice.Text = "Setting up";
        try
        {
            await operation();
            ShowWelcome();
            await RefreshAsync();
        }
        catch (Exception error)
        {
            SetupNotice.Text = error.Message;
        }
        finally
        {
            SetSetupEnabled(true);
        }
    }

    private void SetSetupEnabled(bool enabled)
    {
        CreateSubmitButton.IsEnabled = enabled;
        RestoreSubmitButton.IsEnabled = enabled;
        LinkSubmitButton.IsEnabled = enabled;
    }

    private void ShowCreate_Click(object sender, RoutedEventArgs e)
    {
        ShowSetupPanel(CreatePanel);
        CreateLabelBox.Focus();
    }

    private void ShowRestore_Click(object sender, RoutedEventArgs e)
    {
        ShowSetupPanel(RestorePanel);
        RestoreSecretBox.Focus();
    }

    private void ShowLink_Click(object sender, RoutedEventArgs e)
    {
        ShowSetupPanel(LinkPanel);
        LinkOwnerBox.Focus();
    }

    private void ShowWelcome_Click(object sender, RoutedEventArgs e)
    {
        ShowWelcome();
    }

    private void ShowWelcome()
    {
        ShowSetupPanel(WelcomePanel);
    }

    private void ShowSetupPanel(FrameworkElement visible)
    {
        WelcomePanel.Visibility = Visibility.Collapsed;
        CreatePanel.Visibility = Visibility.Collapsed;
        RestorePanel.Visibility = Visibility.Collapsed;
        LinkPanel.Visibility = Visibility.Collapsed;
        AwaitingPanel.Visibility = Visibility.Collapsed;
        visible.Visibility = Visibility.Visible;
        SetupNotice.Text = "";
    }

    private void Nav_Click(object sender, RoutedEventArgs e)
    {
        if (sender is WpfButton button && button.Tag is string page)
        {
            SelectPage(page);
        }
    }

    private void SelectPage(string page)
    {
        DrivePage.Visibility = page == "Drive" ? Visibility.Visible : Visibility.Collapsed;
        DevicesPage.Visibility = page == "Devices" ? Visibility.Visible : Visibility.Collapsed;
        BackupsPage.Visibility = page == "Backups" ? Visibility.Visible : Visibility.Collapsed;
        NetworkPage.Visibility = page == "Network" ? Visibility.Visible : Visibility.Collapsed;
        SettingsPage.Visibility = page == "Settings" ? Visibility.Visible : Visibility.Collapsed;

        foreach (var button in new[]
        {
            NavDriveButton,
            NavDevicesButton,
            NavBackupsButton,
            NavNetworkButton,
            NavSettingsButton,
        })
        {
            var selected = button.Tag as string == page;
            button.FontWeight = selected ? FontWeights.Bold : FontWeights.Normal;
            button.Background = selected
                ? (WpfBrush)WpfApplication.Current.Resources["IrisPanelBrush"]
                : WpfBrushes.Transparent;
        }
    }

    private void InstallTray()
    {
        var menu = new Forms.ContextMenuStrip();
        menu.Items.Add("Show Iris Drive", null, (_, _) => ShowFromTray());
        menu.Items.Add("Open Drive Folder", null, (_, _) => OpenDriveMount());
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add("Start Sync", null, async (_, _) =>
        {
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            await RefreshAsync();
        });
        menu.Items.Add("Stop Sync", null, async (_, _) =>
        {
            StopDaemon();
            await RefreshAsync();
        });
        menu.Items.Add("Restart Sync", null, async (_, _) =>
        {
            StopDaemon();
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            await RefreshAsync();
        });
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add("Quit", null, (_, _) => Quit());

        trayIcon = new Forms.NotifyIcon
        {
            Icon = WindowsIcon.TrayIcon(),
            Text = "Iris Drive",
            ContextMenuStrip = menu,
            Visible = true,
        };
        trayIcon.DoubleClick += (_, _) => ShowFromTray();
    }

    private void InstallTraySafely()
    {
        if (trayIcon is not null)
        {
            return;
        }

        try
        {
            InstallTray();
        }
        catch
        {
            trayIcon = null;
        }
    }

    private void ShowFromTray()
    {
        Show();
        WindowState = WindowState.Normal;
        Activate();
        _ = RefreshAsync();
    }

    private void UpdateTrayText(bool syncRunning)
    {
        if (trayIcon is not null)
        {
            trayIcon.Text = syncRunning ? "Iris Drive - running" : "Iris Drive - stopped";
        }
    }

    private void Quit()
    {
        quitRequested = true;
        Close();
    }

    private void CloseToTray_Changed(object sender, RoutedEventArgs e)
    {
        WriteCloseToTrayOnClose(CloseToTrayCheckBox.IsChecked == true);
    }

    private bool ReadCloseToTrayOnClose()
    {
        var path = CloseToTrayConfigPath();
        return !File.Exists(path) || File.ReadAllText(path).Trim() != "false";
    }

    private void WriteCloseToTrayOnClose(bool enabled)
    {
        var path = CloseToTrayConfigPath();
        Directory.CreateDirectory(IOPath.GetDirectoryName(path)!);
        File.WriteAllText(path, enabled ? "true\n" : "false\n");
    }

    private string CloseToTrayConfigPath()
    {
        return IOPath.Combine(service.DefaultConfigDirectory, "windows-close-to-tray-on-close");
    }

    private static string FormatBytes(long bytes)
    {
        string[] units = { "B", "KB", "MB", "GB", "TB" };
        double value = bytes;
        var unit = 0;
        while (value >= 1024 && unit < units.Length - 1)
        {
            value /= 1024;
            unit += 1;
        }

        return unit == 0 ? $"{bytes} B" : $"{value:0.0} {units[unit]}";
    }
}
