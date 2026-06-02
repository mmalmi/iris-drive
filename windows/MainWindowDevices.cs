using System;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using WpfBrush = System.Windows.Media.Brush;
using WpfButton = System.Windows.Controls.Button;
using WpfHorizontalAlignment = System.Windows.HorizontalAlignment;
using WpfMessageBox = System.Windows.MessageBox;
using WpfOrientation = System.Windows.Controls.Orientation;
using WpfPanel = System.Windows.Controls.Panel;
using WpfTextBox = System.Windows.Controls.TextBox;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
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

        Window? dialog = null;
        if (currentStatus?.DeviceLinkRequests.Count > 0)
        {
            body.Children.Add(new TextBlock
            {
                Text = "Devices asking to join",
                FontWeight = FontWeights.SemiBold,
                Margin = new Thickness(0, 0, 0, 6),
            });

            foreach (var request in currentStatus.DeviceLinkRequests)
            {
                AddDeviceRequestRow(body, notice, request, () => dialog?.Close());
            }
        }

        body.Children.Add(notice);
        body.Children.Add(new TextBlock { Text = "Device ID", Style = (Style)FindResource("FieldName") });
        body.Children.Add(deviceBox);
        body.Children.Add(new TextBlock { Text = "Name (optional)", Style = (Style)FindResource("FieldName") });
        body.Children.Add(labelBox);
        body.Children.Add(buttons);

        dialog = new Window
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
                dialog?.Close();
            }
            catch (Exception error)
            {
                notice.Text = error.Message;
                await RefreshAddDeviceInputAsync();
            }
        }

        cancel.Click += (_, _) => dialog?.Close();
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
        dialog?.ShowDialog();
    }

    private void AddDeviceRequestRow(
        WpfPanel body,
        TextBlock notice,
        DeviceLinkRequestRow request,
        Action closeDialog)
    {
        var row = new Grid { Margin = new Thickness(0, 0, 0, 8) };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var labels = new StackPanel { Orientation = WpfOrientation.Vertical };
        labels.Children.Add(new TextBlock
        {
            Text = string.IsNullOrWhiteSpace(request.Label) ? "New device" : request.Label,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        labels.Children.Add(new TextBlock
        {
            Text = request.DeviceNpub,
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextTrimming = TextTrimming.CharacterEllipsis,
            FontSize = 12,
        });
        row.Children.Add(labels);

        var requestButtons = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };
        var rejectRequest = new WpfButton
        {
            Content = "Reject",
            Tag = request.RequestUrl,
            MinWidth = 74,
            Margin = new Thickness(8, 0, 0, 0),
        };
        var addRequest = new WpfButton
        {
            Content = "Add",
            Tag = request,
            Style = (Style)FindResource("PrimaryButton"),
            MinWidth = 74,
            Margin = new Thickness(8, 0, 0, 0),
        };
        rejectRequest.Click += async (_, _) =>
        {
            try
            {
                await RejectDeviceAsync(request.RequestUrl);
                closeDialog();
            }
            catch (Exception error)
            {
                notice.Text = error.Message;
            }
        };
        addRequest.Click += async (_, _) =>
        {
            try
            {
                await ApproveDeviceAsync(request.RequestUrl, request.Label);
                closeDialog();
            }
            catch (Exception error)
            {
                notice.Text = error.Message;
            }
        };
        requestButtons.Children.Add(rejectRequest);
        requestButtons.Children.Add(addRequest);
        Grid.SetColumn(requestButtons, 1);
        row.Children.Add(requestButtons);
        body.Children.Add(row);
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

    private async Task RejectDeviceAsync(string request)
    {
        await service.RejectDeviceAsync(request);
        NoticeText.Text = "Device request rejected";
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

        if (WpfMessageBox.Show(
                this,
                $"Remove this device from Iris Drive?\n\n{deviceNpub}",
                "Remove device",
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
            NoticeText.Text = "Device removed";
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
}
