using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;
using System.Windows.Shapes;
using WpfBrush = System.Windows.Media.Brush;
using WpfButton = System.Windows.Controls.Button;
using WpfColor = System.Windows.Media.Color;
using WpfHorizontalAlignment = System.Windows.HorizontalAlignment;
using WpfOrientation = System.Windows.Controls.Orientation;
using WpfProgressBar = System.Windows.Controls.ProgressBar;

namespace IrisDrive.WindowsShell;

/// The Settings page's Storage section: each of the profile's devices and how
/// much of the Drive it holds, as Iris Drive web's Settings > Storage shows it.
/// The app core words all of it (`ui.device_storage`); this only lays it out.
public partial class MainWindow
{
    private const string ChevronRightGlyph = "";
    private const string ChevronDownGlyph = "";
    // Fluent's caution text color, readable on the white panel.
    private static readonly WpfBrush DeviceStorageWarningBrush =
        FrozenBrush(WpfColor.FromRgb(0x9D, 0x5D, 0x00));
    // Devices whose folders are open; kept across refreshes.
    private readonly HashSet<string> openDeviceStorageRows = new(StringComparer.Ordinal);
    private string? shownDeviceStorage;

    private void RenderDeviceStorage(DeviceStorageReport storage)
    {
        if (shownDeviceStorage == storage.Fingerprint)
        {
            return;
        }

        shownDeviceStorage = storage.Fingerprint;
        DeviceStorageList.Children.Clear();
        DeviceStoragePanel.Visibility =
            storage.Devices.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        DeviceStorageSummary.Text = storage.Summary;
        DeviceStorageSummary.Visibility =
            string.IsNullOrWhiteSpace(storage.Summary) ? Visibility.Collapsed : Visibility.Visible;
        foreach (var device in storage.Devices)
        {
            DeviceStorageList.Children.Add(DeviceStorageListRow(device));
        }
    }

    private UIElement DeviceStorageListRow(DeviceStorageRow device)
    {
        var text = new StackPanel { Orientation = WpfOrientation.Vertical };
        text.Children.Add(DeviceStorageHeader(device));
        if (!device.IsCurrent)
        {
            text.Children.Add(DeviceStorageStatus(device));
        }
        if (device.HasReport)
        {
            text.Children.Add(DeviceStorageBar(device.HeldPermille, 6, new Thickness(0, 7, 0, 0)));
            text.Children.Add(DeviceStorageSpace(device));
        }
        else if (device.IsCurrent && !string.IsNullOrWhiteSpace(device.AgeLabel))
        {
            text.Children.Add(DeviceStorageSmallText(
                device.AgeLabel,
                (WpfBrush)FindResource("IrisMutedBrush"),
                new Thickness(0, 4, 0, 0)));
        }

        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(new TextBlock
        {
            Text = DeviceStorageGlyph(device.Icon),
            Style = (Style)FindResource("IconGlyph"),
            HorizontalAlignment = WpfHorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Top,
            Margin = new Thickness(0, 2, 0, 0),
        });
        Grid.SetColumn(text, 1);
        grid.Children.Add(text);

        if (!device.HasReport)
        {
            return new Border
            {
                Padding = new Thickness(12, 9, 12, 9),
                Child = grid,
            };
        }

        var chevron = new TextBlock
        {
            Style = (Style)FindResource("IconGlyph"),
            FontSize = 10,
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            VerticalAlignment = VerticalAlignment.Top,
            Margin = new Thickness(12, 5, 0, 0),
        };
        Grid.SetColumn(chevron, 2);
        grid.Children.Add(chevron);

        var details = DeviceStorageDetails(device);
        var toggle = new WpfButton
        {
            Content = grid,
            Style = (Style)FindResource("FlatButton"),
            HorizontalContentAlignment = WpfHorizontalAlignment.Stretch,
            Padding = new Thickness(12, 9, 12, 9),
        };
        AutomationProperties.SetName(toggle, device.Name);

        void ShowFolders(bool open)
        {
            details.Visibility = open ? Visibility.Visible : Visibility.Collapsed;
            chevron.Text = open ? ChevronDownGlyph : ChevronRightGlyph;
            AutomationProperties.SetHelpText(toggle, open ? "Hide folders" : "Show folders");
        }

        ShowFolders(openDeviceStorageRows.Contains(device.Pubkey));
        toggle.Click += (_, _) =>
        {
            var open = details.Visibility != Visibility.Visible;
            if (open)
            {
                openDeviceStorageRows.Add(device.Pubkey);
            }
            else
            {
                openDeviceStorageRows.Remove(device.Pubkey);
            }
            ShowFolders(open);
        };

        var row = new StackPanel { Orientation = WpfOrientation.Vertical };
        row.Children.Add(toggle);
        row.Children.Add(details);
        return row;
    }

    private DockPanel DeviceStorageHeader(DeviceStorageRow device)
    {
        var header = new DockPanel { LastChildFill = true };
        var held = new TextBlock
        {
            Text = device.HeldLabel,
            Foreground = (WpfBrush)FindResource(device.HoldsAll ? "IrisSuccessBrush" : "IrisMutedBrush"),
            Margin = new Thickness(12, 0, 0, 0),
            VerticalAlignment = VerticalAlignment.Center,
        };
        DockPanel.SetDock(held, Dock.Right);
        header.Children.Add(held);

        var title = new TextBlock
        {
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center,
        };
        title.Inlines.Add(new Run(device.Name) { FontWeight = FontWeights.SemiBold });
        if (device.IsCurrent)
        {
            title.Inlines.Add(new Run("  This device")
            {
                Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
                FontSize = 12,
            });
        }
        header.Children.Add(title);
        return header;
    }

    private StackPanel DeviceStorageStatus(DeviceStorageRow device)
    {
        var muted = (WpfBrush)FindResource("IrisMutedBrush");
        var status = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            Margin = new Thickness(0, 3, 0, 0),
        };
        status.Children.Add(new Ellipse
        {
            Width = 8,
            Height = 8,
            Fill = device.Connected ? (WpfBrush)FindResource("IrisSuccessBrush") : muted,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(0, 0, 6, 0),
        });
        status.Children.Add(DeviceStorageSmallText(
            device.ConnectionLabel,
            muted,
            new Thickness(0),
            device.ConnectionDetail));
        if (!string.IsNullOrWhiteSpace(device.SyncLabel))
        {
            status.Children.Add(DeviceStorageSmallText("·", muted, new Thickness(6, 0, 6, 0)));
            status.Children.Add(DeviceStorageSmallText(
                device.SyncLabel,
                DeviceStorageSyncBrush(device.SyncState),
                new Thickness(0),
                device.SyncDetail));
        }
        return status;
    }

    private Grid DeviceStorageSpace(DeviceStorageRow device)
    {
        var muted = (WpfBrush)FindResource("IrisMutedBrush");
        var line = new Grid { Margin = new Thickness(0, 4, 0, 0) };
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        line.Children.Add(DeviceStorageSmallText(
            device.SpaceLabel,
            muted,
            new Thickness(0),
            device.SpaceDetail));
        var age = DeviceStorageSmallText(device.AgeLabel, muted, new Thickness(12, 0, 0, 0));
        Grid.SetColumn(age, 1);
        line.Children.Add(age);
        return line;
    }

    private StackPanel DeviceStorageDetails(DeviceStorageRow device)
    {
        var muted = (WpfBrush)FindResource("IrisMutedBrush");
        // Under the row's text: its padding plus the icon column.
        var details = new StackPanel { Margin = new Thickness(40, 0, 12, 10) };
        foreach (var folder in device.Folders)
        {
            var line = new StackPanel
            {
                Margin = new Thickness(Math.Max(folder.Depth, 0) * 12, 6, 0, 0),
            };
            var labels = new Grid();
            labels.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            labels.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            labels.Children.Add(new TextBlock
            {
                Text = folder.Label,
                FontSize = 12,
                TextTrimming = TextTrimming.CharacterEllipsis,
            });
            var detail = DeviceStorageSmallText(folder.Detail, muted, new Thickness(12, 0, 0, 0));
            Grid.SetColumn(detail, 1);
            labels.Children.Add(detail);
            line.Children.Add(labels);
            line.Children.Add(DeviceStorageBar(folder.HeldPermille, 4, new Thickness(0, 3, 0, 0)));
            details.Children.Add(line);
        }

        foreach (var note in device.Notes)
        {
            var noteText = DeviceStorageSmallText(note, muted, new Thickness(0, 6, 0, 0));
            noteText.TextWrapping = TextWrapping.Wrap;
            details.Children.Add(noteText);
        }

        return details;
    }

    private static TextBlock DeviceStorageSmallText(
        string text,
        WpfBrush foreground,
        Thickness margin,
        string hint = "")
    {
        var block = new TextBlock
        {
            Text = text,
            Foreground = foreground,
            FontSize = 12,
            Margin = margin,
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center,
        };
        if (!string.IsNullOrWhiteSpace(hint))
        {
            block.ToolTip = hint;
        }
        return block;
    }

    // A thin bar filled to `permille / 1000`; the label beside it says the same.
    private static WpfProgressBar DeviceStorageBar(int permille, double height, Thickness margin)
    {
        return new WpfProgressBar
        {
            Minimum = 0,
            Maximum = 1000,
            Value = Math.Clamp(permille, 0, 1000),
            Height = height,
            Margin = margin,
        };
    }

    private WpfBrush DeviceStorageSyncBrush(string syncState)
    {
        return syncState switch
        {
            "in-sync" => (WpfBrush)FindResource("IrisSuccessBrush"),
            "behind" or "apart" => DeviceStorageWarningBrush,
            _ => (WpfBrush)FindResource("IrisMutedBrush"),
        };
    }

    private static string DeviceStorageGlyph(string icon)
    {
        return icon switch
        {
            "web" => "",
            "phone" => "",
            "computer" => "",
            _ => "",
        };
    }

    private static WpfBrush FrozenBrush(WpfColor color)
    {
        var brush = new SolidColorBrush(color);
        brush.Freeze();
        return brush;
    }
}

/// `ui.device_storage` as the app core words it. Missing (an older core) is empty.
public sealed record DeviceStorageReport(
    string Summary,
    IReadOnlyList<DeviceStorageRow> Devices,
    // The raw JSON, so an unchanged refresh leaves the section alone.
    string Fingerprint)
{
    public static DeviceStorageReport Empty { get; } =
        new("", Array.Empty<DeviceStorageRow>(), "");

    public static DeviceStorageReport FromJson(JsonElement? storage)
    {
        if (!storage.HasValue || storage.Value.ValueKind != JsonValueKind.Object)
        {
            return Empty;
        }

        var value = storage.Value;
        var devices = new List<DeviceStorageRow>();
        foreach (var device in Objects(value, "devices"))
        {
            devices.Add(new DeviceStorageRow(
                String(device, "pubkey") ?? "",
                String(device, "name") ?? "",
                Bool(device, "is_current"),
                String(device, "icon") ?? "",
                Bool(device, "has_report"),
                String(device, "held_label") ?? "",
                Bool(device, "holds_all"),
                Int(device, "held_permille"),
                Bool(device, "connected"),
                String(device, "connection_label") ?? "",
                String(device, "connection_detail") ?? "",
                String(device, "sync_state") ?? "",
                String(device, "sync_label") ?? "",
                String(device, "sync_detail") ?? "",
                String(device, "space_label") ?? "",
                String(device, "space_detail") ?? "",
                String(device, "age_label") ?? "",
                Folders(device),
                Strings(device, "notes")));
        }

        return new DeviceStorageReport(String(value, "summary") ?? "", devices, value.GetRawText());
    }

    private static IReadOnlyList<DeviceStorageFolder> Folders(JsonElement device)
    {
        var folders = new List<DeviceStorageFolder>();
        foreach (var folder in Objects(device, "folders"))
        {
            folders.Add(new DeviceStorageFolder(
                String(folder, "label") ?? "",
                Int(folder, "depth"),
                String(folder, "detail") ?? "",
                Int(folder, "held_permille")));
        }
        return folders;
    }

    private static IEnumerable<JsonElement> Objects(JsonElement root, string name)
    {
        if (!root.TryGetProperty(name, out var array) || array.ValueKind != JsonValueKind.Array)
        {
            yield break;
        }

        foreach (var item in array.EnumerateArray())
        {
            if (item.ValueKind == JsonValueKind.Object)
            {
                yield return item;
            }
        }
    }

    private static IReadOnlyList<string> Strings(JsonElement root, string name)
    {
        var values = new List<string>();
        if (root.TryGetProperty(name, out var array) && array.ValueKind == JsonValueKind.Array)
        {
            foreach (var item in array.EnumerateArray())
            {
                if (item.ValueKind == JsonValueKind.String)
                {
                    values.Add(item.GetString() ?? "");
                }
            }
        }
        return values;
    }

    private static string? String(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
    }

    private static int Int(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.Number &&
            value.TryGetInt32(out var result)
            ? result
            : 0;
    }

    private static bool Bool(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.True;
    }
}

public sealed record DeviceStorageRow(
    string Pubkey,
    string Name,
    bool IsCurrent,
    // `web`, `phone`, `computer` or `drive`.
    string Icon,
    bool HasReport,
    string HeldLabel,
    bool HoldsAll,
    int HeldPermille,
    bool Connected,
    string ConnectionLabel,
    string ConnectionDetail,
    // `in-sync`, `behind`, `ahead`, `apart`, or empty.
    string SyncState,
    string SyncLabel,
    string SyncDetail,
    string SpaceLabel,
    string SpaceDetail,
    string AgeLabel,
    IReadOnlyList<DeviceStorageFolder> Folders,
    IReadOnlyList<string> Notes);

public sealed record DeviceStorageFolder(string Label, int Depth, string Detail, int HeldPermille);
