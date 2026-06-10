using System;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using WpfBrushes = System.Windows.Media.Brushes;
using WpfButton = System.Windows.Controls.Button;
using WpfHorizontalAlignment = System.Windows.HorizontalAlignment;
using WpfOrientation = System.Windows.Controls.Orientation;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
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

        var check = new WpfButton
        {
            Content = checkingBackups ? "Checking 0 of 1" : "Check",
            Tag = target.Target,
            Margin = new Thickness(0, 0, 6, 0),
            IsEnabled = !checkingBackups,
        };
        check.Click += CheckBackupTarget_Click;
        var removeText = target.Kind == "blossom" ? "Remove file server" : "Remove target";
        var remove = new WpfButton { Content = removeText, Tag = target.Target };
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
        if (checkingBackups)
        {
            return;
        }

        var targets = currentStatus?.BackupTargets
            .Select(target => target.Target.Trim())
            .Where(target => !string.IsNullOrWhiteSpace(target))
            .ToList() ?? new List<string>();
        if (targets.Count == 0)
        {
            NoticeText.Text = "No backup targets";
            return;
        }

        checkingBackups = true;
        SetBackupCheckProgress(0, targets.Count);
        try
        {
            for (var index = 0; index < targets.Count; index++)
            {
                SetBackupCheckProgress(index, targets.Count);
                await service.CheckBackupsAsync(targets[index]);
                SetBackupCheckProgress(index + 1, targets.Count);
            }
            NoticeText.Text = "Backups checked";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
        finally
        {
            checkingBackups = false;
            SetBackupCheckProgress(0, 0);
            if (currentStatus is not null)
            {
                RenderBackups(currentStatus);
            }
        }
    }

    private async void CheckBackupTarget_Click(object sender, RoutedEventArgs e)
    {
        if (checkingBackups || (sender as WpfButton)?.Tag is not string target)
        {
            return;
        }

        checkingBackups = true;
        SetBackupCheckProgress(0, 1);
        try
        {
            SetBackupCheckProgress(0, 1);
            await service.CheckBackupsAsync(target);
            SetBackupCheckProgress(1, 1);
            NoticeText.Text = "Target checked";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
        finally
        {
            checkingBackups = false;
            SetBackupCheckProgress(0, 0);
            if (currentStatus is not null)
            {
                RenderBackups(currentStatus);
            }
        }
    }

    private void SetBackupCheckProgress(int checkedCount, int total)
    {
        if (total <= 0)
        {
            BackupCheckProgressPanel.Visibility = Visibility.Collapsed;
            CheckBackupsButton.IsEnabled = true;
            SyncBackupsButton.IsEnabled = true;
            return;
        }

        BackupCheckProgressPanel.Visibility = Visibility.Visible;
        BackupCheckProgressBar.Maximum = Math.Max(total, 1);
        BackupCheckProgressBar.Value = Math.Clamp(checkedCount, 0, total);
        BackupCheckProgressText.Text = $"Checking {checkedCount} of {total}";
        CheckBackupsButton.IsEnabled = false;
        SyncBackupsButton.IsEnabled = false;
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
            NoticeText.Text = "Target removed";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }
}
