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
    private readonly DispatcherTimer updateTimer;
    private Process? daemon;
    private IrisDriveStatusData? currentStatus;
    private IrisDriveUpdateResult? latestUpdate;
    private bool preparingDriveFolder;
    private string? preparedDriveRefreshKey;
    private DateTimeOffset lastDriveFolderReconciliationAt = DateTimeOffset.MinValue;
    private bool refreshing;
    private bool updateChecking;
    private bool updateInstalling;
    private bool updateAvailable;
    private string updateStatus = "";
    private bool quitRequested;
    private string submittedLinkTarget = "";
    private const int RecoveryPhraseWordCount = 12;
    private readonly string[] recoveryWords = new string[RecoveryPhraseWordCount];
    private int recoveryWordIndex;
    private bool updatingRecoveryWordBox;
    private bool settingsUpdating;
    private Forms.NotifyIcon? trayIcon;
    private string[] pendingLaunchArguments;

    public MainWindow(string[]? launchArguments = null)
    {
        pendingLaunchArguments = launchArguments?
            .Where(argument => !string.IsNullOrWhiteSpace(argument))
            .ToArray() ?? Array.Empty<string>();
        InitializeComponent();
        Icon = WindowsIcon.LoadWindowIcon();
        settingsUpdating = true;
        CloseToTrayCheckBox.IsChecked = ReadCloseToTrayOnClose();
        LocalNhashResolverCheckBox.IsChecked = true;
        AutoCheckUpdatesCheckBox.IsChecked = ReadAutoCheckUpdates();
        AutoInstallUpdatesCheckBox.IsChecked = ReadAutoInstallUpdates();
        UpdateBannerAutoInstallCheckBox.IsChecked = AutoInstallUpdatesCheckBox.IsChecked;
        settingsUpdating = false;
        refreshTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) };
        refreshTimer.Tick += async (_, _) => await RefreshAsync();
        updateTimer = new DispatcherTimer { Interval = LoadUpdatePollInterval() };
        updateTimer.Tick += async (_, _) =>
        {
            if (AutoCheckUpdatesCheckBox.IsChecked == true)
            {
                await CheckUpdatesAsync(manual: false);
            }
        };
        SelectPage("Drive");
        RenderLoading();
        RenderUpdateState();
    }

    private async void Window_Loaded(object sender, RoutedEventArgs e)
    {
        InstallTraySafely();
        _ = Task.Run(WindowsIcon.RefreshShortcutIcons);
        refreshTimer.Start();
        await RefreshAsync();
        ApplyLaunchArguments(pendingLaunchArguments);
        if (AutoCheckUpdatesCheckBox.IsChecked == true)
        {
            _ = CheckUpdatesAsync(manual: false);
        }
        updateTimer.Start();
    }

    internal void ApplyLaunchArguments(string[] launchArguments)
    {
        pendingLaunchArguments = Array.Empty<string>();
        foreach (var argument in launchArguments)
        {
            OpenShareDialogFromLink(argument);
        }
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
        updateTimer.Stop();
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
        AppKeyValue.Text = "-";
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
        AwaitingAppKeyBox.Text = status.CurrentAppKeyNpub ?? "";
        AwaitingDeviceBox.Text = status.DeviceNpub ?? "";
        SetupNotice.Text = notice ?? status.PrimaryStatusLabel;
    }

    private void RenderRevokedDevice(IrisDriveStatusData status, string? notice)
    {
        SetupRoot.Visibility = Visibility.Visible;
        MainRoot.Visibility = Visibility.Collapsed;
        ShowSetupPanel(RevokedPanel);
        RevokedAppKeyBox.Text = status.CurrentAppKeyNpub ?? "";
        RevokedDeviceBox.Text = status.DeviceNpub ?? "";
        RevokedRelinkButton.IsEnabled = !string.IsNullOrWhiteSpace(status.CurrentAppKeyNpub);
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

        AppKeyValue.Text = status.CurrentAppKeyNpub ?? "-";
        DeviceValue.Text = status.DeviceNpub ?? "-";
        AuthValue.Text = status.SetupLabel;
        RecoveryPhraseButton.Visibility =
            status.CanExportRecoveryPhrase ? Visibility.Visible : Visibility.Collapsed;
        ApprovePanel.Visibility =
            status.CanAdminProfile ? Visibility.Visible : Visibility.Collapsed;

        RenderDrives(status);
        RenderPeers(status);
        RenderShares(status);
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
        AppKeyValue.Text = "-";
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
        SharesList.Items.Clear();
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
            PeersList.Items.Add(Row("No devices yet", "", ""));
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
            BackupsList.Items.Add(BackupListRow(target));
        }
    }

    private Border BackupListRow(BackupTargetRow target)
    {
        var titleBlock = new TextBlock
        {
            Text = target.Title,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        var subtitleBlock = new TextBlock
        {
            Text = target.Subtitle,
            Foreground = WpfBrushes.Gray,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        var stateBlock = new TextBlock
        {
            Text = target.State,
            Foreground = WpfBrushes.Gray,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(0, 0, 8, 0),
        };

        var text = new StackPanel { Orientation = WpfOrientation.Vertical };
        text.Children.Add(titleBlock);
        text.Children.Add(subtitleBlock);

        var check = new WpfButton { Content = "Check", Tag = target.Target, Margin = new Thickness(0, 0, 6, 0) };
        check.Click += CheckBackupTarget_Click;
        var remove = new WpfButton { Content = "Remove backup", Tag = target.Target };
        remove.Click += RemoveBackupTarget_Click;

        var actions = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };
        actions.Children.Add(stateBlock);
        actions.Children.Add(check);
        actions.Children.Add(remove);

        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        Grid.SetColumn(text, 0);
        Grid.SetColumn(actions, 1);
        grid.Children.Add(text);
        grid.Children.Add(actions);

        return new Border
        {
            Padding = new Thickness(12, 9, 12, 9),
            Child = grid,
        };
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
                Text = $"Device key: {peer.DeviceNpub}",
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
            var delete = PeerActionButton("\uE74D", "Remove Device", peer.DeviceNpub);
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

    private void CopyAppKey_Click(object sender, RoutedEventArgs e)
    {
        CopyText(currentStatus?.CurrentAppKeyNpub, "Device key copied");
    }

    private void CopyDevice_Click(object sender, RoutedEventArgs e)
    {
        CopyText(currentStatus?.DeviceNpub, "Device key copied");
    }

    private void RecoveryPhrase_Click(object sender, RoutedEventArgs e)
    {
        var dataDir = currentStatus?.ConfigDirectory;
        var export = IrisDriveNativeCore.ExportRecoverySecret(dataDir ?? "");
        ShowRecoveryPhraseDialog(export);
    }

    private void ShowRecoveryPhraseDialog(RecoverySecretExport export)
    {
        var wordIndex = 0;
        var wordTitle = new TextBlock
        {
            Text = "Recovery phrase",
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(0, 0, 0, 12),
        };
        var wordLabel = new TextBlock
        {
            Text = $"Word 1 of {RecoveryPhraseWordCount}",
            Style = (Style)FindResource("FieldName"),
            Margin = new Thickness(0, 0, 0, 8),
        };
        var wordValue = new TextBlock
        {
            Text = export.Words.Count == RecoveryPhraseWordCount ? export.Words[0] : export.Error,
            FontSize = 32,
            FontWeight = FontWeights.Bold,
            TextAlignment = TextAlignment.Center,
            Margin = new Thickness(0, 8, 0, 16),
        };
        var back = new WpfButton { Content = "Back", MinWidth = 92 };
        var next = new WpfButton { Content = export.Words.Count == RecoveryPhraseWordCount ? "Next" : "Done", MinWidth = 92 };
        var copyPhrase = new WpfButton { Content = "Copy recovery phrase", MinWidth = 148 };
        var copyKey = new WpfButton { Content = "Copy key", MinWidth = 92 };

        void RenderWord()
        {
            wordLabel.Text = $"Word {wordIndex + 1} of {RecoveryPhraseWordCount}";
            wordValue.Text = export.Words.Count == RecoveryPhraseWordCount ? export.Words[wordIndex] : export.Error;
            back.IsEnabled = wordIndex > 0;
            next.Content = wordIndex == RecoveryPhraseWordCount - 1 || export.Words.Count != RecoveryPhraseWordCount ? "Done" : "Next";
            copyPhrase.IsEnabled = !string.IsNullOrWhiteSpace(export.RecoveryPhrase);
            copyKey.IsEnabled = !string.IsNullOrWhiteSpace(export.SecretKey);
        }

        var buttons = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };
        buttons.Children.Add(back);
        buttons.Children.Add(next);

        var copyButtons = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Left,
            Margin = new Thickness(0, 0, 0, 12),
        };
        copyButtons.Children.Add(copyPhrase);
        copyButtons.Children.Add(copyKey);

        var body = new StackPanel { Margin = new Thickness(20) };
        body.Children.Add(wordTitle);
        body.Children.Add(wordLabel);
        body.Children.Add(wordValue);
        body.Children.Add(copyButtons);
        body.Children.Add(buttons);

        var dialog = new Window
        {
            Title = "Recovery phrase",
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            SizeToContent = SizeToContent.WidthAndHeight,
            ResizeMode = ResizeMode.NoResize,
            Content = body,
        };

        back.Click += (_, _) =>
        {
            wordIndex = Math.Max(0, wordIndex - 1);
            RenderWord();
        };
        next.Click += (_, _) =>
        {
            if (wordIndex >= RecoveryPhraseWordCount - 1 || export.Words.Count != RecoveryPhraseWordCount)
            {
                dialog.Close();
            }
            else
            {
                wordIndex += 1;
                RenderWord();
            }
        };
        copyPhrase.Click += (_, _) => CopyText(export.RecoveryPhrase, "Recovery phrase copied");
        copyKey.Click += (_, _) => CopyText(export.SecretKey, "Secret key copied");

        RenderWord();
        dialog.ShowDialog();
    }

    private void CopyAwaitingDevice_Click(object sender, RoutedEventArgs e)
    {
        CopySetupText(currentStatus?.DeviceNpub, "Device key copied");
    }

    private void CopyRevokedDevice_Click(object sender, RoutedEventArgs e)
    {
        CopySetupText(currentStatus?.DeviceNpub, "Device key copied");
    }

    private async void RelinkRevokedDevice_Click(object sender, RoutedEventArgs e)
    {
        var target = currentStatus?.CurrentAppKeyNpub;
        if (string.IsNullOrWhiteSpace(target))
        {
            SetupNotice.Text = "Device key unavailable";
            return;
        }

        try
        {
            RevokedRelinkButton.IsEnabled = false;
            SetupNotice.Text = "Linking device";
            await service.RelinkDeviceAsync(target);
            await RefreshAsync();
        }
        catch (Exception error)
        {
            SetupNotice.Text = error.Message;
            RevokedRelinkButton.IsEnabled = true;
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

    private async void CheckBackupTarget_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as WpfButton)?.Tag is not string target)
        {
            return;
        }

        try
        {
            await service.CheckBackupsAsync(target);
            NoticeText.Text = "Backup checked";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void RemoveBackupTarget_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as WpfButton)?.Tag is not string target)
        {
            return;
        }

        try
        {
            await service.RemoveBackupTargetAsync(target);
            NoticeText.Text = "Backup removed";
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

    private async void RecoveryNext_Click(object sender, RoutedEventArgs e)
    {
        if (recoveryWordIndex >= RecoveryPhraseWordCount - 1)
        {
            await RunSetupAsync(() => service.RestoreProfileAsync(
                string.Join(" ", recoveryWords.Select(word => word.Trim().ToLowerInvariant()))));
            return;
        }

        if (!string.IsNullOrWhiteSpace(recoveryWords[recoveryWordIndex]))
        {
            recoveryWordIndex = Math.Min(RecoveryPhraseWordCount - 1, recoveryWordIndex + 1);
            UpdateRecoveryPhrasePanel();
            RecoveryWordBox.Focus();
        }
    }

    private void RecoveryBack_Click(object sender, RoutedEventArgs e)
    {
        recoveryWordIndex = Math.Max(0, recoveryWordIndex - 1);
        UpdateRecoveryPhrasePanel();
        RecoveryWordBox.Focus();
    }

    private void RecoveryWordBox_KeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
        {
            return;
        }
        e.Handled = true;
        RecoveryNext_Click(sender, e);
    }

    private void RecoveryPaste_Click(object sender, RoutedEventArgs e)
    {
        ApplyRecoveryWordInput(WpfClipboard.ContainsText() ? WpfClipboard.GetText() : "");
        UpdateRecoveryPhrasePanel();
        RecoveryWordBox.Focus();
    }

    private void RecoveryWordBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        if (updatingRecoveryWordBox)
        {
            return;
        }
        ApplyRecoveryWordInput(RecoveryWordBox.Text);
        UpdateRecoveryPhrasePanel();
    }

    private void ApplyRecoveryWordInput(string input)
    {
        var parts = input
            .Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .Select(word => word.ToLowerInvariant())
            .ToArray();
        if (parts.Length <= 1)
        {
            recoveryWords[recoveryWordIndex] = input.Trim().ToLowerInvariant();
            return;
        }

        for (var offset = 0; offset < parts.Length && recoveryWordIndex + offset < recoveryWords.Length; offset++)
        {
            recoveryWords[recoveryWordIndex + offset] = parts[offset];
        }
        recoveryWordIndex = Math.Min(RecoveryPhraseWordCount - 1, recoveryWordIndex + parts.Length - 1);
    }

    private bool CanAdvanceRecoveryWord() =>
        recoveryWordIndex == RecoveryPhraseWordCount - 1
            ? recoveryWords.All(word => !string.IsNullOrWhiteSpace(word))
            : !string.IsNullOrWhiteSpace(recoveryWords[recoveryWordIndex]);

    private void UpdateRecoveryPhrasePanel(bool updateTextBox = true)
    {
        RecoveryWordLabel.Text = $"Word {recoveryWordIndex + 1} of {RecoveryPhraseWordCount}";
        RecoveryBackButton.IsEnabled = recoveryWordIndex > 0;
        RecoveryNextText.Text = recoveryWordIndex == RecoveryPhraseWordCount - 1 ? "Restore" : "Next";
        RecoveryNextButton.IsEnabled = CanAdvanceRecoveryWord();
        if (updateTextBox)
        {
            updatingRecoveryWordBox = true;
            RecoveryWordBox.Text = recoveryWords[recoveryWordIndex];
            RecoveryWordBox.CaretIndex = RecoveryWordBox.Text.Length;
            updatingRecoveryWordBox = false;
        }
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
        RecoveryNextButton.IsEnabled = enabled && CanAdvanceRecoveryWord();
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
        ShowSetupPanel(RestoreOptionsPanel);
    }

    private void ShowLink_Click(object sender, RoutedEventArgs e)
    {
        ShowSetupPanel(LinkPanel);
        LinkTargetBox.Focus();
    }

    private void ShowRecoveryPhrase_Click(object sender, RoutedEventArgs e)
    {
        recoveryWordIndex = 0;
        Array.Fill(recoveryWords, "");
        RecoveryWordBox.Text = "";
        UpdateRecoveryPhrasePanel();
        ShowSetupPanel(RecoveryPhrasePanel);
        RecoveryWordBox.Focus();
    }

    private void ShowSecretKey_Click(object sender, RoutedEventArgs e)
    {
        ShowSetupPanel(RestorePanel);
        RestoreSecretBox.Focus();
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
        RestoreOptionsPanel.Visibility = Visibility.Collapsed;
        RecoveryPhrasePanel.Visibility = Visibility.Collapsed;
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
        SharesPage.Visibility = page == "Shares" ? Visibility.Visible : Visibility.Collapsed;
        BackupsPage.Visibility = page == "Backups" ? Visibility.Visible : Visibility.Collapsed;
        NetworkPage.Visibility = page == "Network" ? Visibility.Visible : Visibility.Collapsed;
        SettingsPage.Visibility = page == "Settings" ? Visibility.Visible : Visibility.Collapsed;

        foreach (var button in new[]
        {
            NavDriveButton,
            NavDevicesButton,
            NavSharesButton,
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

    private void AutoCheckUpdates_Changed(object sender, RoutedEventArgs e)
    {
        if (settingsUpdating)
        {
            return;
        }

        WriteAutoCheckUpdates(AutoCheckUpdatesCheckBox.IsChecked == true);
        if (AutoCheckUpdatesCheckBox.IsChecked == true)
        {
            _ = CheckUpdatesAsync(manual: false);
        }
    }

    private void AutoInstallUpdates_Changed(object sender, RoutedEventArgs e)
    {
        if (settingsUpdating)
        {
            return;
        }

        var enabled = sender == UpdateBannerAutoInstallCheckBox
            ? UpdateBannerAutoInstallCheckBox.IsChecked == true
            : AutoInstallUpdatesCheckBox.IsChecked == true;
        settingsUpdating = true;
        AutoInstallUpdatesCheckBox.IsChecked = enabled;
        UpdateBannerAutoInstallCheckBox.IsChecked = enabled;
        settingsUpdating = false;
        WriteAutoInstallUpdates(enabled);
        if (enabled && CanInstallUpdate)
        {
            _ = InstallUpdateAsync();
        }
    }

    private async void CheckUpdates_Click(object sender, RoutedEventArgs e)
    {
        await CheckUpdatesAsync();
    }

    private async void InstallUpdate_Click(object sender, RoutedEventArgs e)
    {
        await InstallUpdateAsync();
    }

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

    private async Task InstallUpdateAsync()
    {
        if (!CanInstallUpdate || updateInstalling)
        {
            return;
        }

        updateInstalling = true;
        updateStatus = $"Downloading {UpdateVersionText}";
        RenderUpdateState();
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

            updateStatus = $"Downloaded {IOPath.GetFileName(result.Path)}";
            if (!IsTruthy(Environment.GetEnvironmentVariable("IRIS_DRIVE_UPDATE_SKIP_OPEN")))
            {
                _ = Process.Start(new ProcessStartInfo(result.Path) { UseShellExecute = true });
            }
        }
        catch (Exception error)
        {
            updateStatus = error.Message;
        }
        finally
        {
            updateInstalling = false;
            RenderUpdateState();
        }
    }

    private void RenderUpdateState()
    {
        UpdateBanner.Visibility = updateAvailable ? Visibility.Visible : Visibility.Collapsed;
        UpdateBannerText.Text = UpdateStripeText();
        UpdateStatusText.Text = updateStatus;
        CheckUpdatesButton.IsEnabled = !updateChecking && !updateInstalling;
        InstallUpdateButton.IsEnabled = CanInstallUpdate;
        UpdateBannerInstallButton.IsEnabled = CanInstallUpdate;
        settingsUpdating = true;
        UpdateBannerAutoInstallCheckBox.IsChecked = AutoInstallUpdatesCheckBox.IsChecked;
        settingsUpdating = false;
    }

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

    private static bool ReadAutoCheckUpdates()
    {
        using var key = Registry.CurrentUser.OpenSubKey(@"Software\Iris Drive");
        return key?.GetValue("AutoCheckUpdates") is not int value || value != 0;
    }

    private static void WriteAutoCheckUpdates(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(@"Software\Iris Drive");
        key?.SetValue("AutoCheckUpdates", enabled ? 1 : 0, RegistryValueKind.DWord);
    }

    private static bool ReadAutoInstallUpdates()
    {
        using var key = Registry.CurrentUser.OpenSubKey(@"Software\Iris Drive");
        return key?.GetValue("AutoInstallUpdates") is int value && value != 0;
    }

    private static void WriteAutoInstallUpdates(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(@"Software\Iris Drive");
        key?.SetValue("AutoInstallUpdates", enabled ? 1 : 0, RegistryValueKind.DWord);
    }

    private static TimeSpan LoadUpdatePollInterval()
    {
        var raw = Environment.GetEnvironmentVariable("IRIS_DRIVE_UPDATE_POLL_SECONDS");
        return double.TryParse(raw, out var seconds) && seconds > 0
            ? TimeSpan.FromSeconds(seconds)
            : TimeSpan.FromHours(6);
    }

    private static string UpdateDownloadDirectory()
    {
        var configured = Environment.GetEnvironmentVariable("IRIS_DRIVE_UPDATE_DOWNLOAD_DIR");
        return string.IsNullOrWhiteSpace(configured)
            ? IOPath.Combine(IOPath.GetTempPath(), "IrisDriveDownloads")
            : configured;
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
