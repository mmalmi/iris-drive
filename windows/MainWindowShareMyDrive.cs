using System;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using WpfBrush = System.Windows.Media.Brush;
using WpfButton = System.Windows.Controls.Button;
using WpfContextMenu = System.Windows.Controls.ContextMenu;
using WpfHorizontalAlignment = System.Windows.HorizontalAlignment;
using WpfMenuItem = System.Windows.Controls.MenuItem;
using WpfOrientation = System.Windows.Controls.Orientation;

namespace IrisDrive.WindowsShell;

/// Where shared folders sit in My Drive, and leaving them: a shared folder is
/// on disk only as its place in My Drive, as in Google Drive for desktop.
public partial class MainWindow
{
    private const int ShareFolderMenuLimit = 300;

    private Border PendingShareJoinListRow(PendingShareJoinRow join)
    {
        var name = string.IsNullOrWhiteSpace(join.DisplayName) ? "Shared folder" : join.DisplayName.Trim();
        var stack = new StackPanel { Orientation = WpfOrientation.Vertical };
        stack.Children.Add(new TextBlock
        {
            Text = $"Waiting to join “{name}”",
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        stack.Children.Add(ShareDetail("You get access when someone who manages it has Iris Drive open."));

        var cancel = new WpfButton
        {
            Content = "Cancel",
            Tag = join.ShareId,
            Margin = new Thickness(8, 0, 0, 0),
            VerticalAlignment = VerticalAlignment.Top,
        };
        AutomationProperties.SetName(cancel, $"Stop waiting to join {name}");
        cancel.Click += CancelShareJoin_Click;

        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        Grid.SetColumn(stack, 0);
        grid.Children.Add(stack);
        Grid.SetColumn(cancel, 1);
        grid.Children.Add(cancel);

        return new Border
        {
            Padding = new Thickness(12, 9, 12, 9),
            Child = grid,
        };
    }

    /// The row's less common actions: take it out of My Drive, stop sharing, leave.
    private WpfButton ShareMoreButton(ShareRow share)
    {
        var menu = new WpfContextMenu();
        if (!share.Owned && !string.IsNullOrWhiteSpace(share.Location))
        {
            var remove = new WpfMenuItem { Header = "Remove from My Drive" };
            remove.Click += async (_, _) => await RemoveShareFromMyDriveAsync(share.ShareId);
            menu.Items.Add(remove);
        }

        var end = new WpfMenuItem { Header = share.Owned ? "Stop sharing…" : "Leave…" };
        end.Click += async (_, _) => await StopSharingOrLeaveAsync(share);
        menu.Items.Add(end);

        var more = new WpfButton
        {
            Content = new TextBlock { Text = "", Style = (Style)FindResource("IconGlyph") },
            ToolTip = "More",
            Margin = new Thickness(8, 0, 0, 0),
        };
        AutomationProperties.SetName(more, $"More for {ShareName(share)}");
        more.Click += (_, _) =>
        {
            menu.PlacementTarget = more;
            menu.Placement = PlacementMode.Bottom;
            menu.IsOpen = true;
        };
        return more;
    }

    private async Task RemoveShareFromMyDriveAsync(string shareId)
    {
        try
        {
            await service.RemoveShareShortcutsAsync(shareId);
            ShowSharesNotice("Removed from My Drive. It stays under Shares.");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    private async Task StopSharingOrLeaveAsync(ShareRow share)
    {
        var name = ShareName(share);
        var confirmed = share.Owned
            ? ConfirmShareAction(
                "Stop sharing?",
                $"“{name}” stays in your My Drive. People you shared it with keep what they have and get no further changes.",
                "Stop sharing")
            : ConfirmShareAction(
                "Leave shared folder?",
                $"“{name}” is removed from this PC. You can join again with a link.",
                "Leave");
        if (!confirmed)
        {
            return;
        }

        try
        {
            await service.DeleteShareAsync(share.ShareId);
            ShowSharesNotice(share.Owned
                ? $"Stopped sharing “{name}”"
                : $"Left “{name}”");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    private async void CancelShareJoin_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string shareId })
        {
            return;
        }

        try
        {
            await service.CancelShareJoinAsync(shareId);
            ShowSharesNotice("No longer waiting to join");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    /// Lists the My Drive folders that can be shared, to fill in the folder box.
    private void ChooseShareFolder_Click(object sender, RoutedEventArgs e)
    {
        var menu = new WpfContextMenu
        {
            PlacementTarget = ChooseShareFolderButton,
            Placement = PlacementMode.Bottom,
            MaxHeight = 360,
        };
        var folders = currentStatus?.ShareableFolders ?? Array.Empty<string>();
        if (folders.Count == 0)
        {
            menu.Items.Add(new WpfMenuItem { Header = "No folders in My Drive yet", IsEnabled = false });
        }

        foreach (var folder in folders.Take(ShareFolderMenuLimit))
        {
            // A text block, so underscores in folder names are not read as access keys.
            var item = new WpfMenuItem { Header = new TextBlock { Text = folder } };
            AutomationProperties.SetName(item, folder);
            item.Click += (_, _) =>
            {
                ShareSourceBox.Text = folder;
                ShareSourceBox.Focus();
            };
            menu.Items.Add(item);
        }

        menu.IsOpen = true;
    }

    /// Asks before an action that takes access away; Cancel is the default.
    private bool ConfirmShareAction(string heading, string detail, string confirmLabel)
    {
        var confirmed = false;
        var cancel = new WpfButton
        {
            Content = "Cancel",
            MinWidth = 92,
            IsCancel = true,
            IsDefault = true,
        };
        var confirm = new WpfButton
        {
            Content = confirmLabel,
            Style = (Style)FindResource("PrimaryButton"),
            MinWidth = 92,
            Margin = new Thickness(8, 0, 0, 0),
        };
        var buttons = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };
        buttons.Children.Add(cancel);
        buttons.Children.Add(confirm);

        var body = new StackPanel { Margin = new Thickness(18), Width = 400 };
        body.Children.Add(new TextBlock
        {
            Text = heading,
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 10),
        });
        body.Children.Add(new TextBlock
        {
            Text = detail,
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 18),
        });
        body.Children.Add(buttons);

        var dialog = new Window
        {
            Title = confirmLabel,
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            SizeToContent = SizeToContent.WidthAndHeight,
            Content = body,
        };
        cancel.Click += (_, _) => dialog.Close();
        confirm.Click += (_, _) =>
        {
            confirmed = true;
            dialog.Close();
        };
        dialog.ShowDialog();
        return confirmed;
    }
}
