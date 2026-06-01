using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;
using System.Windows.Threading;
using Microsoft.Win32;
using Forms = System.Windows.Forms;
using WpfApplication = System.Windows.Application;
using WpfBrush = System.Windows.Media.Brush;
using WpfBrushes = System.Windows.Media.Brushes;
using WpfButton = System.Windows.Controls.Button;
using WpfClipboard = System.Windows.Clipboard;
using WpfHorizontalAlignment = System.Windows.HorizontalAlignment;
using IOPath = System.IO.Path;
using WpfOrientation = System.Windows.Controls.Orientation;
using WpfTextBox = System.Windows.Controls.TextBox;

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
    private string submittedLinkOwner = "";
    private bool settingsUpdating;
    private Forms.NotifyIcon? trayIcon;

    public MainWindow()
    {
        InitializeComponent();
        Icon = WindowsIcon.LoadWindowIcon();
        CloseToTrayCheckBox.IsChecked = ReadCloseToTrayOnClose();
        settingsUpdating = true;
        LocalNhashResolverCheckBox.IsChecked = true;
        settingsUpdating = false;
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
                SetupNotice.Text = "";
                return;
            }

            if (status.IsRevoked)
            {
                StopDaemon();
                RenderRevokedDevice(status, null);
                return;
            }

            var syncRunning = EnsureDaemonRunning(status);
            if (status.IsAwaitingLinkedApproval)
            {
                RenderAwaitingApproval(status, null);
                return;
            }
            if (!status.IsSetupComplete)
            {
                SetupRoot.Visibility = Visibility.Visible;
                MainRoot.Visibility = Visibility.Collapsed;
                SetupNotice.Text = status.SetupLabel;
                return;
            }

            ScheduleDriveFolderRefresh(status);
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
        DriveMessage.Text = "Turning sync on";
        StatusPill.Text = "Sync on";
        FilesValue.Text = "0";
        StorageValue.Text = "0 B";
        DevicesValue.Text = "0/0";
        NoticeText.Text = "";
        OwnerValue.Text = "-";
        DeviceValue.Text = "-";
        AuthValue.Text = "-";
    }

    private void RenderAwaitingApproval(
        IrisDriveStatusData status,
        string? notice)
    {
        SetupRoot.Visibility = Visibility.Visible;
        MainRoot.Visibility = Visibility.Collapsed;
        ShowSetupPanel(AwaitingPanel);
        AwaitingOwnerBox.Text = status.OwnerNpub ?? "";
        AwaitingDeviceBox.Text = status.DeviceNpub ?? "";
        SetupNotice.Text = notice ?? status.PrimaryStatusLabel;
    }

    private void RenderRevokedDevice(IrisDriveStatusData status, string? notice)
    {
        SetupRoot.Visibility = Visibility.Visible;
        MainRoot.Visibility = Visibility.Collapsed;
        ShowSetupPanel(RevokedPanel);
        RevokedOwnerBox.Text = status.OwnerNpub ?? "";
        RevokedDeviceBox.Text = status.DeviceNpub ?? "";
        RevokedRelinkButton.IsEnabled = !string.IsNullOrWhiteSpace(status.OwnerNpub);
        SetupNotice.Text = notice ?? "Device removed";
        UpdateTrayText(false);
    }

    private void RenderStatus(IrisDriveStatusData status, bool syncRunning, string? notice)
    {
        DriveTitle.Text = status.DriveName;
        DriveMessage.Text = status.PrimaryStatusLabel;
        StatusPill.Text = status.PrimaryStatusLabel;
        FilesValue.Text = status.FileCount.ToString(CultureInfo.InvariantCulture);
        StorageValue.Text = FormatBytes(status.VisibleFileBytes);
        DevicesValue.Text = $"{status.OnlineDeviceCount}/{status.AuthorizedDeviceCount}";
        NoticeText.Text = notice ?? "";

        CopySnapshotButton.IsEnabled = !string.IsNullOrWhiteSpace(status.SnapshotUrl);
        OpenSnapshotButton.IsEnabled = !string.IsNullOrWhiteSpace(status.SnapshotUrl);
        StartButton.IsEnabled = !syncRunning;
        StopButton.IsEnabled = syncRunning;
        StartButton.Visibility = syncRunning ? Visibility.Collapsed : Visibility.Visible;
        StopButton.Visibility = syncRunning ? Visibility.Visible : Visibility.Collapsed;

        OwnerValue.Text = status.OwnerNpub ?? "-";
        DeviceValue.Text = status.DeviceNpub ?? "-";
        AuthValue.Text = status.SetupLabel;
        ApprovePanel.Visibility =
            status.HasOwnerSigningAuthority ? Visibility.Visible : Visibility.Collapsed;

        RenderDrives(status);
        RenderPeers(status);
        RenderBackups(status);
        RenderNetwork(status);
        try
        {
            settingsUpdating = true;
            LocalNhashResolverCheckBox.IsChecked = status.LocalNhashResolverEnabled;
        }
        finally
        {
            settingsUpdating = false;
        }
        UpdateTrayText(syncRunning);
    }

    private void RenderUnavailable(string message)
    {
        DriveTitle.Text = "My Drive";
        DriveMessage.Text = "Unavailable";
        StatusPill.Text = "Paused";
        FilesValue.Text = "0";
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
        StartButton.Visibility = Visibility.Visible;
        StopButton.Visibility = Visibility.Collapsed;
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
            PeersList.Items.Add(Row("No devices", "", ""));
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
        if (peer.IsCurrentDevice && !string.IsNullOrWhiteSpace(peer.DeviceNpub))
        {
            stack.Children.Add(new TextBlock
            {
                Text = $"Device ID: {peer.DeviceNpub}",
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

        var actions = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };

        if (peer.CanAppointAdmin)
        {
            var appointAdmin = PeerActionButton("\uE8D7", "Make admin", peer.DeviceNpub);
            appointAdmin.Click += AppointAdmin_Click;
            actions.Children.Add(appointAdmin);
        }

        if (peer.CanDemoteAdmin)
        {
            var demoteAdmin = PeerActionButton("\uE711", "Remove admin", peer.DeviceNpub);
            demoteAdmin.Click += DemoteAdmin_Click;
            actions.Children.Add(demoteAdmin);
        }

        if (peer.CanRevoke)
        {
            var delete = PeerActionButton("\uE74D", "Delete device", peer.DeviceNpub);
            delete.Click += DeleteDevice_Click;
            actions.Children.Add(delete);
        }

        if (actions.Children.Count > 0)
        {
            Grid.SetColumn(actions, 2);
            grid.Children.Add(actions);
        }

        return new Border
        {
            Padding = new Thickness(12, 9, 12, 9),
            Child = grid,
        };
    }

    private WpfButton PeerActionButton(string glyph, string toolTip, string deviceNpub)
    {
        return new WpfButton
        {
            Content = new TextBlock { Text = glyph, Style = (Style)FindResource("IconGlyph") },
            Style = (Style)FindResource("IconButton"),
            Tag = deviceNpub,
            Margin = new Thickness(8, 0, 0, 0),
            ToolTip = toolTip,
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
        FipsList.Items.Add(Row("State", status.Fips.State, status.Fips.StateLabel));
        FipsList.Items.Add(Row("Roster FIPS", status.Fips.RosterLabel, ""));
        FipsList.Items.Add(Row("Other FIPS", status.Fips.OtherPeerCount.ToString(), ""));
        FipsList.Items.Add(Row("Online", status.Fips.OnlineDeviceCount.ToString(), ""));
        FipsList.Items.Add(Row("Direct", status.Fips.DirectDeviceCount.ToString(), ""));
        FipsList.Items.Add(Row("Mesh", status.Fips.MeshDeviceCount.ToString(), ""));
        if (!string.IsNullOrWhiteSpace(status.Fips.EndpointNpub))
        {
            FipsList.Items.Add(Row("Endpoint", status.Fips.EndpointNpub, ""));
        }

        if (!string.IsNullOrWhiteSpace(status.Fips.DiscoveryScope))
        {
            FipsList.Items.Add(Row("Scope", status.Fips.DiscoveryScope, ""));
        }

        foreach (var peer in status.Fips.Peers)
        {
            FipsList.Items.Add(Row($"Peer {IrisDriveStatusData.ShortText(peer.Npub)}", peer.Subtitle, ""));
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
        if (status.RelayStatuses.Count == 0)
        {
            RelaysList.Items.Add(Row("No relays", "", ""));
            return;
        }

        foreach (var relay in status.RelayStatuses)
        {
            RelaysList.Items.Add(Row(relay.Url, relay.Health, relay.StatusLabel));
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
        if (status.IsRevoked)
        {
            StopDaemon();
            NoticeText.Text = "Device removed";
            return false;
        }

        if (ExternalDaemonMode)
        {
            return true;
        }

        if (daemon is { HasExited: false } || service.DaemonLockIsRunning(status))
        {
            return true;
        }

        try
        {
            daemon = service.StartDaemonProcess();
            NoticeText.Text = "Sync resumed";
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
        if (ExternalDaemonMode)
        {
            daemon = null;
            return;
        }

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
            NoticeText.Text = "Sync paused";
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

    private async void OpenDrive_Click(object sender, RoutedEventArgs e)
    {
        await OpenDriveMountAsync();
    }

    private async Task OpenDriveMountAsync()
    {
        if (currentStatus is { IsSetupComplete: true } status && !EnsureDaemonRunning(status))
        {
            NoticeText.Text = "Could not start sync";
            return;
        }

        if (currentStatus is { IsSetupComplete: true })
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

        if (currentStatus is { IsSetupComplete: true })
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
        if (!status.IsSetupComplete || string.IsNullOrWhiteSpace(status.ProviderRefreshKey))
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
        driveFolder.NativeSyncRootReady;

    private void CopySnapshot_Click(object sender, RoutedEventArgs e)
    {
        CopyText(currentStatus?.SnapshotUrl, "drive.iris.to link copied");
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

    private void CopyAwaitingDevice_Click(object sender, RoutedEventArgs e)
    {
        CopySetupText(currentStatus?.DeviceNpub, "Device ID copied");
    }

    private void CopyRevokedDevice_Click(object sender, RoutedEventArgs e)
    {
        CopySetupText(currentStatus?.DeviceNpub, "Device ID copied");
    }

    private async void RelinkRevokedDevice_Click(object sender, RoutedEventArgs e)
    {
        var owner = currentStatus?.OwnerNpub;
        if (string.IsNullOrWhiteSpace(owner))
        {
            SetupNotice.Text = "Owner key unavailable";
            return;
        }

        try
        {
            RevokedRelinkButton.IsEnabled = false;
            SetupNotice.Text = "Linking device";
            await service.RelinkDeviceAsync(owner);
            await RefreshAsync();
        }
        catch (Exception error)
        {
            SetupNotice.Text = error.Message;
            RevokedRelinkButton.IsEnabled = true;
        }
    }

    private void ShowAddDevice_Click(object sender, RoutedEventArgs e)
    {
        var deviceBox = new WpfTextBox
        {
            Tag = "Device ID",
            MinHeight = 34,
            MinWidth = 360,
            Margin = new Thickness(0, 4, 0, 10),
        };
        var labelBox = new WpfTextBox
        {
            Tag = "Name (optional)",
            MinHeight = 34,
            MinWidth = 360,
            Margin = new Thickness(0, 4, 0, 14),
        };
        var notice = new TextBlock
        {
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
            Text = "Paste the Device ID shown on the other device when you link it manually.",
        };
        var cancel = new WpfButton { Content = "Cancel", MinWidth = 92 };
        var add = new WpfButton
        {
            Content = "Add",
            Style = (Style)FindResource("PrimaryButton"),
            MinWidth = 92,
            Margin = new Thickness(8, 0, 0, 0),
            IsEnabled = false,
        };
        var buttons = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };
        buttons.Children.Add(cancel);
        buttons.Children.Add(add);

        var body = new StackPanel { Margin = new Thickness(18), Width = 400 };
        body.Children.Add(new TextBlock
        {
            Text = "Add a device",
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(0, 0, 0, 10),
        });
        body.Children.Add(notice);
        body.Children.Add(new TextBlock { Text = "Device ID", Style = (Style)FindResource("FieldName") });
        body.Children.Add(deviceBox);
        body.Children.Add(new TextBlock { Text = "Name (optional)", Style = (Style)FindResource("FieldName") });
        body.Children.Add(labelBox);
        body.Children.Add(buttons);

        var dialog = new Window
        {
            Title = "Add a device",
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            SizeToContent = SizeToContent.WidthAndHeight,
            Content = body,
        };

        var deviceValidationSequence = 0;
        async Task RefreshAddDeviceInputAsync()
        {
            var sequence = ++deviceValidationSequence;
            add.IsEnabled = false;
            var isComplete = await service.IsCompleteLinkInputAsync(deviceBox.Text);
            if (sequence == deviceValidationSequence)
            {
                add.IsEnabled = isComplete;
            }
        }

        async Task SubmitDeviceAsync()
        {
            if (!await service.IsCompleteLinkInputAsync(deviceBox.Text))
            {
                notice.Text = "Paste the complete Device ID or device invite.";
                add.IsEnabled = false;
                return;
            }
            add.IsEnabled = false;
            try
            {
                await ApproveDeviceAsync(deviceBox.Text, labelBox.Text);
                dialog.Close();
            }
            catch (Exception error)
            {
                notice.Text = error.Message;
                await RefreshAddDeviceInputAsync();
            }
        }
        cancel.Click += (_, _) => dialog.Close();
        add.Click += async (_, _) => await SubmitDeviceAsync();
        deviceBox.TextChanged += async (_, _) => await RefreshAddDeviceInputAsync();
        deviceBox.KeyDown += async (_, key) =>
        {
            if (key.Key != Key.Enter)
            {
                return;
            }
            key.Handled = true;
            await SubmitDeviceAsync();
        };
        labelBox.KeyDown += async (_, key) =>
        {
            if (key.Key != Key.Enter)
            {
                return;
            }
            key.Handled = true;
            await SubmitDeviceAsync();
        };
        dialog.ShowDialog();
    }

    private async Task ApproveDeviceAsync(string device, string label)
    {
        await service.ApproveDeviceAsync(device, label);
        StopDaemon();
        if (currentStatus is not null)
        {
            EnsureDaemonRunning(currentStatus);
        }
        NoticeText.Text = "Device approved";
        await RefreshAsync();
    }

    private async void ResetInvite_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.ResetInviteAsync();
            NoticeText.Text = "Invite reset";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void DeleteDevice_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string deviceNpub })
        {
            return;
        }

        if (System.Windows.MessageBox.Show(
                this,
                $"Delete this device from Iris Drive?\n\n{deviceNpub}",
                "Delete device",
                MessageBoxButton.YesNo,
                MessageBoxImage.Warning) != MessageBoxResult.Yes)
        {
            return;
        }

        try
        {
            await service.DeleteDeviceAsync(deviceNpub);
            StopDaemon();
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            NoticeText.Text = "Device deleted";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void AppointAdmin_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string deviceNpub })
        {
            return;
        }

        try
        {
            await service.AppointAdminAsync(deviceNpub);
            StopDaemon();
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            NoticeText.Text = "Device made admin";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void DemoteAdmin_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string deviceNpub })
        {
            return;
        }

        try
        {
            await service.DemoteAdminAsync(deviceNpub);
            StopDaemon();
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            NoticeText.Text = "Admin removed";
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

    private async void CheckBackups_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.CheckBackupsAsync();
            NoticeText.Text = "Backups checked";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void Logout_Click(object sender, RoutedEventArgs e)
    {
        await LogoutAsync();
    }

    private async Task LogoutAsync()
    {
        if (System.Windows.MessageBox.Show(
                this,
                "Remove this local Iris Drive profile from Windows?",
                "Log out",
                MessageBoxButton.YesNo,
                MessageBoxImage.Warning) != MessageBoxResult.Yes)
        {
            return;
        }

        try
        {
            StopDaemon();
            await service.LogoutAsync();
            currentStatus = null;
            preparedDriveRefreshKey = null;
            ShowWelcome();
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
        if (string.IsNullOrWhiteSpace(CreateUsernameBox.Text))
        {
            await RunSetupAsync(() => service.CreateProfileAsync("", ""));
            return;
        }
        ShowSetupPanel(CreatePhotoPanel);
        CreatePhotoPathBox.Focus();
    }

    private void CreateUsernameBox_KeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
        {
            return;
        }
        e.Handled = true;
        CreateSubmit_Click(sender, e);
    }

    private void ChooseCreatePhoto_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFileDialog
        {
            Filter = "Image files|*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp|All files|*.*",
            Multiselect = false,
        };
        if (dialog.ShowDialog(this) == true)
        {
            CreatePhotoPathBox.Text = dialog.FileName;
            CreatePhotoSubmitText.Text = "Create profile";
        }
    }

    private void CreateUsernameBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        CreateSubmitText.Text = string.IsNullOrWhiteSpace(CreateUsernameBox.Text)
            ? "Create profile"
            : "Continue";
    }

    private async void CreatePhotoSubmit_Click(object sender, RoutedEventArgs e)
    {
        await RunSetupAsync(() => service.CreateProfileAsync(
            CreateUsernameBox.Text,
            CreatePhotoPathBox.Text));
    }

    private async void RestoreSubmit_Click(object sender, RoutedEventArgs e)
    {
        await RunSetupAsync(() => service.RestoreProfileAsync(RestoreSecretBox.Password));
    }

    private void RestoreSecretBox_KeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
        {
            return;
        }
        e.Handled = true;
        RestoreSubmit_Click(sender, e);
    }

    private void RestoreSecretBox_PasswordChanged(object sender, RoutedEventArgs e)
    {
        RestoreSecretPlaceholder.Visibility = string.IsNullOrEmpty(RestoreSecretBox.Password)
            ? Visibility.Visible
            : Visibility.Collapsed;
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
        CreatePhotoSubmitButton.IsEnabled = enabled;
        RestoreSubmitButton.IsEnabled = enabled;
        LinkSubmitButton.IsEnabled = enabled;
    }

    private void ShowCreate_Click(object sender, RoutedEventArgs e)
    {
        ShowSetupPanel(CreatePanel);
        CreateSubmitText.Text = string.IsNullOrWhiteSpace(CreateUsernameBox.Text)
            ? "Create profile"
            : "Continue";
        CreateUsernameBox.Focus();
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
        CreatePhotoPanel.Visibility = Visibility.Collapsed;
        RestorePanel.Visibility = Visibility.Collapsed;
        LinkPanel.Visibility = Visibility.Collapsed;
        AwaitingPanel.Visibility = Visibility.Collapsed;
        RevokedPanel.Visibility = Visibility.Collapsed;
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
        menu.Items.Add("Resume Sync", null, async (_, _) =>
        {
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            await RefreshAsync();
        });
        menu.Items.Add("Pause Sync", null, async (_, _) =>
        {
            StopDaemon();
            await RefreshAsync();
        });
        menu.Items.Add("Log out", null, async (_, _) => await LogoutAsync());
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

    private async void LocalNhashResolver_Changed(object sender, RoutedEventArgs e)
    {
        if (settingsUpdating)
        {
            return;
        }

        var enabled = LocalNhashResolverCheckBox.IsChecked == true;
        try
        {
            await service.SetNhashResolverAsync(enabled);
            StopDaemon();
            if (currentStatus is not null)
            {
                EnsureDaemonRunning(currentStatus);
            }
            NoticeText.Text = enabled ? "Local resolver enabled" : "Local resolver disabled";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
            await RefreshAsync();
        }
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

    private static bool ExternalDaemonMode =>
        IsTruthy(Environment.GetEnvironmentVariable("IRIS_DRIVE_EXTERNAL_DAEMON"));

    private static bool IsTruthy(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return false;
        }

        return value.Trim().ToLowerInvariant() is "1" or "true" or "yes" or "on";
    }
}
