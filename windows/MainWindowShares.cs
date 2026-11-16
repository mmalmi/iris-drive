using System;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Media;
using WpfBrush = System.Windows.Media.Brush;
using WpfButton = System.Windows.Controls.Button;
using WpfClipboard = System.Windows.Clipboard;
using WpfComboBox = System.Windows.Controls.ComboBox;
using WpfHorizontalAlignment = System.Windows.HorizontalAlignment;
using WpfOrientation = System.Windows.Controls.Orientation;
using WpfRadioButton = System.Windows.Controls.RadioButton;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private static readonly string[] ShareRoles = ["reader", "editor", "admin"];

    private enum ContentLinkDialogChoice
    {
        Cancel,
        Open,
        Save,
    }

    private void RenderShares(IrisDriveStatusData status)
    {
        SharesList.Items.Clear();
        foreach (var join in status.PendingShareJoins)
        {
            SharesList.Items.Add(PendingShareJoinListRow(join));
        }

        foreach (var share in status.Shares)
        {
            SharesList.Items.Add(ShareListRow(share));
        }

        if (SharesList.Items.Count == 0)
        {
            SharesList.Items.Add(Row("No shared folders", "", ""));
        }
    }

    /// Share actions report here: the Drive page notice is not visible from Shares.
    private void ShowSharesNotice(string text)
    {
        SharesNotice.Text = text;
        SharesNotice.Visibility = string.IsNullOrWhiteSpace(text)
            ? Visibility.Collapsed
            : Visibility.Visible;
    }

    private Border ShareListRow(ShareRow share)
    {
        var titleBlock = new TextBlock
        {
            Text = ShareName(share),
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        var stack = new StackPanel { Orientation = WpfOrientation.Vertical };
        stack.Children.Add(titleBlock);
        stack.Children.Add(ShareDetail(ShareSummary(share)));
        if (!string.IsNullOrWhiteSpace(ShareRepairText(share)))
        {
            stack.Children.Add(ShareDetail(ShareRepairText(share)));
        }

        foreach (var member in share.Members)
        {
            stack.Children.Add(ShareMemberRow(share, member));
        }
        foreach (var invite in share.PendingInvites)
        {
            stack.Children.Add(PendingShareInviteText(invite));
        }

        var actions = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Top,
        };

        if (string.IsNullOrWhiteSpace(share.Location))
        {
            var shortcut = ShareActionButton("Add to My Drive", share.ShareId);
            shortcut.Click += AddShareShortcut_Click;
            actions.Children.Add(shortcut);
        }
        else
        {
            var open = ShareActionButton("Open", share.Location);
            open.Click += OpenShare_Click;
            actions.Children.Add(open);
        }

        if (share.CanAdmin)
        {
            var shareByLink = new WpfButton
            {
                Content = "Share",
                Tag = share,
                Margin = new Thickness(8, 0, 0, 0),
            };
            shareByLink.Click += ShowInviteShareMember_Click;
            actions.Children.Add(shareByLink);
        }

        if (share.RepairNeeded || share.MissingKeyWrapCount > 0)
        {
            var repair = ShareActionButton("Repair", share.ShareId);
            repair.Click += RepairShare_Click;
            actions.Children.Add(repair);
        }

        actions.Children.Add(ShareMoreButton(share));

        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        Grid.SetColumn(stack, 0);
        grid.Children.Add(stack);
        if (actions.Children.Count > 0)
        {
            Grid.SetColumn(actions, 1);
            grid.Children.Add(actions);
        }

        var row = ListRow(grid, ShareName(share));
        AutomationProperties.SetHelpText(row, ShareSummary(share));
        return row;
    }

    private Grid ShareMemberRow(ShareRow share, ShareMemberRow member)
    {
        var row = new Grid { Margin = new Thickness(0, 7, 0, 0) };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var text = ShareDetail($"{member.Name} \u00b7 {member.Detail}");
        var tag = new ShareMemberActionTag(share.ShareId, member.ProfileId, member.Name, ShareName(share));
        Grid.SetColumn(text, 0);
        row.Children.Add(text);

        var actions = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };
        if (member.CanChangeRole)
        {
            var roleBox = new WpfComboBox
            {
                MinWidth = 86,
                Tag = tag,
                Margin = new Thickness(8, 0, 0, 0),
            };
            foreach (var item in ShareRoles)
            {
                roleBox.Items.Add(item);
            }
            roleBox.SelectedItem = string.IsNullOrWhiteSpace(member.Role) ? "reader" : member.Role;
            roleBox.SelectionChanged += ShareMemberRole_Changed;
            actions.Children.Add(roleBox);
        }
        if (member.CanRevoke)
        {
            var revoke = new WpfButton
            {
                Content = "Remove",
                Tag = tag,
                Margin = new Thickness(8, 0, 0, 0),
            };
            revoke.Click += RevokeShareMember_Click;
            actions.Children.Add(revoke);
        }

        if (actions.Children.Count > 0)
        {
            Grid.SetColumn(actions, 1);
            row.Children.Add(actions);
        }

        return row;
    }

    private TextBlock PendingShareInviteText(PendingShareInviteRow invite)
    {
        var display = string.IsNullOrWhiteSpace(invite.DisplayName)
            ? "Pending contact"
            : invite.DisplayName;
        var role = string.IsNullOrWhiteSpace(invite.RoleLabel) ? invite.Role : invite.RoleLabel;
        var status = string.IsNullOrWhiteSpace(invite.StatusLabel) ? invite.Status : invite.StatusLabel;
        return ShareDetail(
            $"{display} \u00b7 {role} \u00b7 {status} \u00b7 {IrisDriveStatusData.ShortText(invite.RepresentativeNpubHint)}");
    }

    private TextBlock ShareDetail(string text) =>
        new()
        {
            Text = text,
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextTrimming = TextTrimming.CharacterEllipsis,
            FontSize = 12,
        };

    private static string ShareName(ShareRow share) =>
        string.IsNullOrWhiteSpace(share.DisplayName) ? "Shared folder" : share.DisplayName.Trim();

    private static string ShareSummary(ShareRow share)
    {
        var role = string.IsNullOrWhiteSpace(share.RoleLabel) ? share.Role : share.RoleLabel;
        var people = $"{share.ParticipantCount} {(share.ParticipantCount == 1 ? "person" : "people")}";
        var place = string.IsNullOrWhiteSpace(share.Location)
            ? "Not in My Drive"
            : $"My Drive/{share.Location}";
        return $"{role} \u00b7 {people} \u00b7 {place}";
    }

    private static string ShareRepairText(ShareRow share)
    {
        if (!share.RepairNeeded && share.MissingKeyWrapCount == 0)
        {
            return "";
        }
        if (share.MissingKeyWrapCount > 0)
        {
            var noun = share.MissingKeyWrapCount == 1 ? "access wrap" : "access wraps";
            return $"{share.MissingKeyWrapCount} missing {noun}";
        }
        return "Repair needed";
    }

    private WpfButton ShareActionButton(string text, string shareId) =>
        new()
        {
            Content = text,
            Tag = shareId,
            Margin = new Thickness(8, 0, 0, 0),
        };

    private bool OpenShareDialogFromLink(string input)
    {
        var classification = IrisDriveNativeCore.ClassifyLinkInput(input);
        if (!string.Equals(classification.Kind, "share_dialog", StringComparison.Ordinal))
        {
            return false;
        }

        OpenShareDialogFromLink(classification);
        return true;
    }

    private void OpenShareDialogFromLink(IrisDriveLinkInputClassification classification)
    {
        SelectPage("Shares");
        Show();
        if (WindowState == System.Windows.WindowState.Minimized)
        {
            WindowState = System.Windows.WindowState.Normal;
        }
        Activate();
        ShareSourceBox.Focus();

        if (!classification.IsValid || string.IsNullOrWhiteSpace(classification.ShareSourcePath))
        {
            ShowSharesNotice(string.IsNullOrWhiteSpace(classification.Error)
                ? "Share folder path is required."
                : classification.Error.Trim());
            return;
        }

        ShareSourceBox.Text = classification.ShareSourcePath.Trim();

        var recipient = FirstNonEmpty(
            classification.ShareRecipientDisplayName,
            classification.ShareRecipientNpubHint,
            classification.ShareRecipientProfileId);
        ShowSharesNotice(string.IsNullOrWhiteSpace(recipient)
            ? "Share folder selected"
            : $"Share folder selected for {recipient}");
    }

    private bool OpenContentLinkFromLink(string input)
    {
        var classification = IrisDriveNativeCore.ClassifyLinkInput(input);
        if (!string.Equals(classification.Kind, "nhash_file", StringComparison.Ordinal)
            && !string.Equals(classification.Kind, "mutable_file", StringComparison.Ordinal))
        {
            return false;
        }

        Show();
        if (WindowState == System.Windows.WindowState.Minimized)
        {
            WindowState = System.Windows.WindowState.Normal;
        }
        Activate();

        var label = string.IsNullOrWhiteSpace(classification.OpenDisplayName)
            ? "file"
            : classification.OpenDisplayName.Trim();
        if (!classification.IsValid || string.IsNullOrWhiteSpace(classification.LocalOpenUrl))
        {
            NoticeText.Text = string.IsNullOrWhiteSpace(classification.Error)
                ? $"Could not open {label}"
                : classification.Error.Trim();
            return true;
        }

        var choice = ShowContentLinkDialog(label);
        if (choice == ContentLinkDialogChoice.Open)
        {
            NoticeText.Text = $"Opening {label}";
            service.OpenUri(classification.LocalOpenUrl.Trim());
        }
        else if (choice == ContentLinkDialogChoice.Save)
        {
            _ = ImportContentLinkAsync(classification.NormalizedInput, label);
        }
        return true;
    }

    private ContentLinkDialogChoice ShowContentLinkDialog(string label)
    {
        var choice = ContentLinkDialogChoice.Cancel;
        var title = new TextBlock
        {
            Text = $"Open {label}?",
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 10),
        };
        var detail = new TextBlock
        {
            Text = "Open it now or save a copy to Iris Drive.",
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 18),
        };
        var cancel = new WpfButton { Content = "Cancel", MinWidth = 92 };
        var save = new WpfButton
        {
            Content = "Save to Drive",
            MinWidth = 116,
            Margin = new Thickness(8, 0, 0, 0),
        };
        var open = new WpfButton
        {
            Content = "Open",
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
        buttons.Children.Add(save);
        buttons.Children.Add(open);

        var body = new StackPanel { Margin = new Thickness(18), Width = 400 };
        body.Children.Add(title);
        body.Children.Add(detail);
        body.Children.Add(buttons);

        var dialog = new Window
        {
            Title = "Open Iris Drive link",
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            SizeToContent = SizeToContent.WidthAndHeight,
            Content = body,
        };

        cancel.Click += (_, _) => dialog.Close();
        save.Click += (_, _) =>
        {
            choice = ContentLinkDialogChoice.Save;
            dialog.Close();
        };
        open.Click += (_, _) =>
        {
            choice = ContentLinkDialogChoice.Open;
            dialog.Close();
        };
        dialog.ShowDialog();
        return choice;
    }

    private async Task ImportContentLinkAsync(string link, string label)
    {
        if (string.IsNullOrWhiteSpace(link))
        {
            NoticeText.Text = $"Could not save {label}";
            return;
        }

        try
        {
            NoticeText.Text = $"Saving {label}";
            await service.ImportContentLinkAsync(link);
            NoticeText.Text = $"Saved {label} to Iris Drive";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private static string FirstNonEmpty(params string[] values)
    {
        foreach (var value in values)
        {
            if (!string.IsNullOrWhiteSpace(value))
            {
                return value.Trim();
            }
        }
        return "";
    }

    private async void CreateShare_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.CreateShareAsync(ShareSourceBox.Text);
            ShareSourceBox.Text = "";
            ShowSharesNotice("Share created");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    private async void AcceptShareInvite_Click(object sender, RoutedEventArgs e)
    {
        var link = ShareInviteBox.Text.Trim();
        if (link.Length == 0)
        {
            ShowSharesNotice("Paste a share link");
            return;
        }

        try
        {
            var asksToJoin = string.Equals(
                IrisDriveNativeCore.ClassifyLinkInput(link).Kind,
                "share_join",
                StringComparison.Ordinal);
            await service.AcceptShareInviteAsync(link);
            ShareInviteBox.Text = "";
            ShowSharesNotice(asksToJoin
                ? "Asked to join. It appears in Shares once you're let in."
                : "Joined");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    private async void AddShareShortcut_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string shareId })
        {
            return;
        }

        try
        {
            await service.AddShareShortcutAsync(shareId);
            ShowSharesNotice("Added to My Drive");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    private void OpenShare_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string path })
        {
            return;
        }

        try
        {
            service.OpenDrivePath(path);
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    private async void RepairShare_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string shareId })
        {
            return;
        }

        try
        {
            await service.RepairShareWrapsAsync(shareId);
            ShowSharesNotice("Share access repaired");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    /// Share by link: anyone who opens it can ask to join, and this PC lets
    /// them in while Iris Drive runs. No app is needed to open the link.
    private void ShowInviteShareMember_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: ShareRow share })
        {
            return;
        }

        var shareName = ShareName(share);
        var view = new WpfRadioButton
        {
            Content = "View",
            GroupName = "ShareLinkRole",
            IsChecked = true,
            Margin = new Thickness(0, 0, 18, 0),
        };
        var edit = new WpfRadioButton { Content = "Edit", GroupName = "ShareLinkRole" };
        var roles = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            Margin = new Thickness(0, 6, 0, 14),
        };
        roles.Children.Add(view);
        roles.Children.Add(edit);

        var notice = new TextBlock
        {
            Foreground = (WpfBrush)FindResource("IrisSuccessBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
            Visibility = Visibility.Collapsed,
        };
        var disable = new WpfButton { Content = "Turn off links", MinWidth = 108 };
        var done = new WpfButton { Content = "Done", MinWidth = 80 };
        var copy = new WpfButton
        {
            Content = "Copy link",
            Style = (Style)FindResource("PrimaryButton"),
            MinWidth = 100,
            Margin = new Thickness(8, 0, 0, 0),
            IsDefault = true,
        };
        var buttons = new DockPanel { LastChildFill = false };
        DockPanel.SetDock(disable, Dock.Left);
        DockPanel.SetDock(copy, Dock.Right);
        DockPanel.SetDock(done, Dock.Right);
        buttons.Children.Add(disable);
        buttons.Children.Add(copy);
        buttons.Children.Add(done);

        var body = new StackPanel { Margin = new Thickness(18), Width = 420 };
        body.Children.Add(new TextBlock
        {
            Text = $"Share \u201c{shareName}\u201d",
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Margin = new Thickness(0, 0, 0, 12),
        });
        body.Children.Add(new TextBlock
        {
            Text = "People with the link can",
            Style = (Style)FindResource("FieldName"),
        });
        body.Children.Add(roles);
        body.Children.Add(new TextBlock
        {
            Text = "Anyone with this link can join. This PC lets them in while Iris Drive is running. "
                + "They don't need the app \u2014 the link opens in a browser.",
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 14),
        });
        body.Children.Add(notice);
        body.Children.Add(buttons);

        var dialog = new Window
        {
            Title = "Share",
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            SizeToContent = SizeToContent.WidthAndHeight,
            Content = body,
        };

        void ShowDialogNotice(string text, bool failed)
        {
            notice.Text = text;
            notice.Foreground = (WpfBrush)FindResource(failed ? "IrisMutedBrush" : "IrisSuccessBrush");
            notice.Visibility = Visibility.Visible;
        }

        done.Click += (_, _) => dialog.Close();
        copy.Click += async (_, _) =>
        {
            copy.IsEnabled = false;
            try
            {
                var status = await service.CreateShareJoinLinkAsync(
                    share.ShareId,
                    edit.IsChecked == true ? "editor" : "reader");
                if (string.IsNullOrWhiteSpace(status.LastShareInviteUrl))
                {
                    ShowDialogNotice("Link unavailable", failed: true);
                }
                else
                {
                    WpfClipboard.SetText(status.LastShareInviteUrl);
                    ShowDialogNotice("Link copied", failed: false);
                    ShowSharesNotice("Link copied");
                }
            }
            catch (Exception error)
            {
                ShowDialogNotice(error.Message, failed: true);
            }
            finally
            {
                copy.IsEnabled = true;
            }
        };
        disable.Click += async (_, _) =>
        {
            disable.IsEnabled = false;
            try
            {
                await service.DisableShareJoinLinksAsync(share.ShareId);
                ShowSharesNotice("Links turned off. People already in keep access.");
                dialog.Close();
            }
            catch (Exception error)
            {
                ShowDialogNotice(error.Message, failed: true);
                disable.IsEnabled = true;
            }
        };
        dialog.ShowDialog();
    }

    private async void RevokeShareMember_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: ShareMemberActionTag tag })
        {
            return;
        }

        if (!ConfirmShareAction(
                "Remove access?",
                $"{tag.MemberName} will no longer get changes to \u201c{tag.ShareName}\u201d.",
                "Remove"))
        {
            return;
        }

        try
        {
            await service.RevokeShareMemberAsync(tag.ShareId, tag.ProfileId);
            ShowSharesNotice("Access removed");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }

    private async void ShareMemberRole_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (sender is not WpfComboBox { Tag: ShareMemberActionTag tag, SelectedItem: string role })
        {
            return;
        }

        try
        {
            await service.SetShareMemberRoleAsync(tag.ShareId, tag.ProfileId, role);
            ShowSharesNotice("Share role updated");
            await RefreshAsync();
        }
        catch (Exception error)
        {
            ShowSharesNotice(error.Message);
        }
    }
}

public sealed record ShareMemberActionTag(
    string ShareId,
    string ProfileId,
    string MemberName,
    string ShareName);
