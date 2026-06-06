using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using WpfBrush = System.Windows.Media.Brush;
using WpfButton = System.Windows.Controls.Button;
using WpfComboBox = System.Windows.Controls.ComboBox;
using WpfHorizontalAlignment = System.Windows.HorizontalAlignment;
using WpfMessageBox = System.Windows.MessageBox;
using WpfOrientation = System.Windows.Controls.Orientation;
using WpfTextBox = System.Windows.Controls.TextBox;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private static readonly string[] ShareRoles = ["reader", "editor", "admin"];

    private void RenderShares(IrisDriveStatusData status)
    {
        SharesList.Items.Clear();
        if (status.Shares.Count == 0)
        {
            SharesList.Items.Add(Row("No shared folders", "", ""));
            return;
        }

        foreach (var share in status.Shares)
        {
            SharesList.Items.Add(ShareListRow(share));
        }
    }

    private Border ShareListRow(ShareRow share)
    {
        var titleBlock = new TextBlock
        {
            Text = string.IsNullOrWhiteSpace(share.DisplayName) ? "Shared folder" : share.DisplayName,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        var stack = new StackPanel { Orientation = WpfOrientation.Vertical };
        stack.Children.Add(titleBlock);

        if (!string.IsNullOrWhiteSpace(share.SourcePath))
        {
            stack.Children.Add(ShareDetail(share.SourcePath));
        }

        stack.Children.Add(ShareDetail(ShareMetadata(share)));
        if (!string.IsNullOrWhiteSpace(ShareRepairText(share)))
        {
            stack.Children.Add(ShareDetail(ShareRepairText(share)));
        }
        if (share.ShortcutPaths.Count > 0)
        {
            stack.Children.Add(ShareDetail($"Shortcut: {share.ShortcutPaths[0]}"));
        }

        foreach (var member in share.Members)
        {
            stack.Children.Add(ShareMemberRow(share.ShareId, member));
        }

        var actions = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };

        if (share.CanAdmin)
        {
            var invite = ShareActionButton("Invite", share.ShareId);
            invite.Click += ShowInviteShareMember_Click;
            actions.Children.Add(invite);
        }

        if (share.RepairNeeded || share.MissingKeyWrapCount > 0)
        {
            var repair = ShareActionButton("Repair", share.ShareId);
            repair.Click += RepairShare_Click;
            actions.Children.Add(repair);
        }

        if (share.ShortcutPaths.Count == 0)
        {
            var shortcut = ShareActionButton("Shortcut", share.ShareId);
            shortcut.Click += AddShareShortcut_Click;
            actions.Children.Add(shortcut);
        }

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

        return new Border
        {
            Padding = new Thickness(12, 9, 12, 9),
            Child = grid,
        };
    }

    private Grid ShareMemberRow(string shareId, ShareMemberRow member)
    {
        var row = new Grid { Margin = new Thickness(0, 7, 0, 0) };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var display = string.IsNullOrWhiteSpace(member.DisplayName)
            ? "IrisProfile"
            : member.DisplayName;
        var role = string.IsNullOrWhiteSpace(member.RoleLabel) ? member.Role : member.RoleLabel;
        var status = string.IsNullOrWhiteSpace(member.StatusLabel) ? member.Status : member.StatusLabel;
        var identity = string.IsNullOrWhiteSpace(member.RepresentativeNpubHint)
            ? member.ProfileId
            : member.RepresentativeNpubHint;
        var text = ShareDetail($"{display} | {role} | {status} | {IrisDriveStatusData.ShortText(identity)}");
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
                Tag = new ShareMemberActionTag(shareId, member.ProfileId),
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
                Tag = new ShareMemberActionTag(shareId, member.ProfileId),
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

    private TextBlock ShareDetail(string text) =>
        new()
        {
            Text = text,
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextTrimming = TextTrimming.CharacterEllipsis,
            FontSize = 12,
        };

    private static string ShareMetadata(ShareRow share)
    {
        var role = string.IsNullOrWhiteSpace(share.RoleLabel) ? share.Role : share.RoleLabel;
        var key = string.IsNullOrWhiteSpace(share.KeyStatusLabel) ? share.KeyStatus : share.KeyStatusLabel;
        var participants = $"{share.ParticipantCount} {(share.ParticipantCount == 1 ? "person" : "people")}";
        return $"{role} | {key} | {participants}";
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
            NoticeText.Text = string.IsNullOrWhiteSpace(classification.Error)
                ? "Share folder path is required."
                : classification.Error.Trim();
            return;
        }

        ShareSourceBox.Text = classification.ShareSourcePath.Trim();
        ShareNameBox.Text = classification.ShareDisplayName.Trim();

        var recipient = FirstNonEmpty(
            classification.ShareRecipientDisplayName,
            classification.ShareRecipientNpubHint,
            classification.ShareRecipientProfileId);
        NoticeText.Text = string.IsNullOrWhiteSpace(recipient)
            ? "Share folder selected"
            : $"Share folder selected for {recipient}";
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
            await service.CreateShareAsync(ShareSourceBox.Text, ShareNameBox.Text);
            ShareSourceBox.Text = "";
            ShareNameBox.Text = "";
            NoticeText.Text = "Share created";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private async void AcceptShareInvite_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            await service.AcceptShareInviteAsync(ShareInviteBox.Text);
            ShareInviteBox.Text = "";
            NoticeText.Text = "Share accepted";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
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
            NoticeText.Text = "Shortcut added";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
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
            NoticeText.Text = "Share access repaired";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private void ShowInviteShareMember_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: string shareId })
        {
            return;
        }

        var evidenceBox = new WpfTextBox
        {
            Tag = "Signed recipient evidence JSON",
            MinHeight = 72,
            MinWidth = 460,
            TextWrapping = TextWrapping.Wrap,
            AcceptsReturn = true,
            Margin = new Thickness(0, 4, 0, 10),
        };
        var displayNameBox = new WpfTextBox
        {
            Tag = "Name",
            MinHeight = 34,
            MinWidth = 460,
            Margin = new Thickness(0, 4, 0, 10),
        };
        var roleBox = new WpfComboBox
        {
            MinWidth = 140,
            Margin = new Thickness(0, 4, 0, 14),
        };
        foreach (var role in ShareRoles)
        {
            roleBox.Items.Add(role);
        }
        roleBox.SelectedItem = "reader";

        var notice = new TextBlock
        {
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
        };
        var cancel = new WpfButton { Content = "Cancel", MinWidth = 92 };
        var invite = new WpfButton
        {
            Content = "Invite",
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
        buttons.Children.Add(invite);

        var body = new StackPanel { Margin = new Thickness(18), Width = 500 };
        body.Children.Add(new TextBlock
        {
            Text = "Invite to share",
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(0, 0, 0, 10),
        });
        body.Children.Add(notice);
        body.Children.Add(new TextBlock { Text = "Recipient identity evidence", Style = (Style)FindResource("FieldName") });
        body.Children.Add(evidenceBox);
        body.Children.Add(new TextBlock { Text = "Name", Style = (Style)FindResource("FieldName") });
        body.Children.Add(displayNameBox);
        body.Children.Add(new TextBlock { Text = "Role", Style = (Style)FindResource("FieldName") });
        body.Children.Add(roleBox);
        body.Children.Add(buttons);

        var dialog = new Window
        {
            Title = "Invite to share",
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            SizeToContent = SizeToContent.WidthAndHeight,
            Content = body,
        };

        async Task SubmitAsync()
        {
            invite.IsEnabled = false;
            try
            {
                await service.InviteShareMemberFromEvidenceAsync(
                    shareId,
                    evidenceBox.Text,
                    roleBox.SelectedItem as string ?? "reader",
                    displayNameBox.Text);
                NoticeText.Text = "Share invite created";
                dialog.Close();
                await RefreshAsync();
            }
            catch (Exception error)
            {
                notice.Text = error.Message;
                invite.IsEnabled = true;
            }
        }

        cancel.Click += (_, _) => dialog.Close();
        invite.Click += async (_, _) => await SubmitAsync();
        evidenceBox.KeyDown += async (_, key) =>
        {
            if (key.Key == Key.Enter && Keyboard.Modifiers == ModifierKeys.Control)
            {
                key.Handled = true;
                await SubmitAsync();
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

        if (WpfMessageBox.Show(
                this,
                $"Remove this IrisProfile from the share?\n\n{tag.ProfileId}",
                "Remove share member",
                MessageBoxButton.YesNo,
                MessageBoxImage.Warning) != MessageBoxResult.Yes)
        {
            return;
        }

        try
        {
            await service.RevokeShareMemberAsync(tag.ShareId, tag.ProfileId);
            NoticeText.Text = "Share member removed";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
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
            NoticeText.Text = "Share role updated";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }
}

public sealed record ShareMemberActionTag(string ShareId, string ProfileId);
