using System;
using System.Linq;
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
            Tag = "Request link or device ID",
            MinHeight = 34,
            MinWidth = 360,
            Margin = new Thickness(0, 4, 0, 10),
        };
        var notice = new TextBlock
        {
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
            Text = "",
        };
        var cancel = new WpfButton { Content = "Cancel", MinWidth = 92 };
        var buttons = new StackPanel
        {
            Orientation = WpfOrientation.Horizontal,
            HorizontalAlignment = WpfHorizontalAlignment.Right,
        };
        buttons.Children.Add(cancel);

        var body = new StackPanel { Margin = new Thickness(18), Width = 400 };
        body.Children.Add(new TextBlock
        {
            Text = "Add a Device",
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(0, 0, 0, 10),
        });

        Window? dialog = null;
        if (currentStatus?.AppKeyLinkRequests.Count > 0)
        {
            body.Children.Add(new TextBlock
            {
                Text = "Device requests",
                FontWeight = FontWeights.SemiBold,
                Margin = new Thickness(0, 0, 0, 6),
            });

            foreach (var request in currentStatus.AppKeyLinkRequests)
            {
                AddDeviceRequestRow(body, notice, request, () => dialog?.Close());
            }
        }

        body.Children.Add(notice);
        body.Children.Add(new TextBlock { Text = "Request link or device ID", Style = (Style)FindResource("FieldName") });
        body.Children.Add(deviceBox);
        body.Children.Add(buttons);

        dialog = new Window
        {
            Title = "Add a Device",
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            SizeToContent = SizeToContent.WidthAndHeight,
            Content = body,
        };

        var deviceValidationSequence = 0;
        var lastPromptedDevice = "";
        async Task RefreshAddDeviceInputAsync()
        {
            var sequence = ++deviceValidationSequence;
            var isComplete = await service.IsCompleteDeviceApprovalInputAsync(deviceBox.Text);
            var trimmed = deviceBox.Text.Trim();
            if (sequence == deviceValidationSequence && isComplete && lastPromptedDevice != trimmed)
            {
                lastPromptedDevice = trimmed;
                await ConfirmAndApproveDeviceAsync(trimmed, notice, () => dialog?.Close());
            }
        }

        async Task SubmitDeviceAsync()
        {
            await ConfirmAndApproveDeviceAsync(deviceBox.Text, notice, () => dialog?.Close());
        }

        cancel.Click += (_, _) => dialog?.Close();
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
        dialog?.ShowDialog();
    }

    private void AddDeviceRequestRow(
        WpfPanel body,
        TextBlock notice,
        AppKeyLinkRequestRow request,
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
            Content = "Review",
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
            await ConfirmAndApproveDeviceAsync(request.RequestUrl, notice, closeDialog);
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

    private async Task ConfirmAndApproveDeviceAsync(
        string device,
        TextBlock notice,
        Action closeDialog)
    {
        var trimmed = device.Trim();
        if (string.IsNullOrWhiteSpace(trimmed) ||
            !await service.IsCompleteDeviceApprovalInputAsync(trimmed))
        {
            notice.Text = "Enter a complete request link or device ID.";
            return;
        }

        var confirmed = WpfMessageBox.Show(
            this,
            "This will add the joining device to Iris Drive.",
            "Approve this device?",
            MessageBoxButton.YesNo,
            MessageBoxImage.Question) == MessageBoxResult.Yes;
        if (!confirmed)
        {
            return;
        }

        try
        {
            await ApproveDeviceAsync(trimmed, "");
            closeDialog();
        }
        catch (Exception error)
        {
            notice.Text = error.Message;
        }
    }

    private async Task RejectDeviceAsync(string request)
    {
        await service.RejectDeviceAsync(request);
        NoticeText.Text = "Device request rejected";
        await RefreshAsync();
    }

    private void RenameDevice_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not WpfButton { Tag: PeerRow peer } ||
            string.IsNullOrWhiteSpace(peer.DeviceNpub))
        {
            return;
        }

        var nameBox = new WpfTextBox
        {
            Text = peer.Title,
            MinHeight = 34,
            MinWidth = 320,
            Margin = new Thickness(0, 4, 0, 10),
        };
        var notice = new TextBlock
        {
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
        };
        var cancel = new WpfButton { Content = "Cancel", MinWidth = 92 };
        var save = new WpfButton
        {
            Content = "Save",
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

        var body = new StackPanel { Margin = new Thickness(18), Width = 380 };
        body.Children.Add(new TextBlock
        {
            Text = "Rename Device",
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(0, 0, 0, 10),
        });
        body.Children.Add(new TextBlock { Text = "Device name", Style = (Style)FindResource("FieldName") });
        body.Children.Add(nameBox);
        body.Children.Add(notice);
        body.Children.Add(buttons);

        var dialog = new Window
        {
            Title = "Rename Device",
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            SizeToContent = SizeToContent.WidthAndHeight,
            Content = body,
        };

        async Task SaveAsync()
        {
            try
            {
                await service.RenameDeviceAsync(peer.DeviceNpub, nameBox.Text);
                NoticeText.Text = "Device renamed";
                await RefreshAsync();
                dialog.Close();
            }
            catch (Exception error)
            {
                notice.Text = error.Message;
            }
        }

        cancel.Click += (_, _) => dialog.Close();
        save.Click += async (_, _) => await SaveAsync();
        nameBox.KeyDown += async (_, key) =>
        {
            if (key.Key != Key.Enter)
            {
                return;
            }
            key.Handled = true;
            await SaveAsync();
        };
        dialog.ShowDialog();
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
                "Remove Device",
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
            NoticeText.Text = "Device made admin";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }

    private void ShowAddRecoveryKey_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Window
        {
            Title = "Add Recovery Key",
            Owner = this,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ResizeMode = ResizeMode.NoResize,
            SizeToContent = SizeToContent.WidthAndHeight,
        };

        var body = new StackPanel { Margin = new Thickness(18), Width = 420 };
        var notice = new TextBlock
        {
            Foreground = (WpfBrush)FindResource("IrisMutedBrush"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
        };

        void ResetBody(string title)
        {
            body.Children.Clear();
            body.Children.Add(new TextBlock
            {
                Text = title,
                FontSize = 20,
                FontWeight = FontWeights.SemiBold,
                Margin = new Thickness(0, 0, 0, 10),
            });
            body.Children.Add(notice);
        }

        StackPanel Buttons(params WpfButton[] buttons)
        {
            var row = new StackPanel
            {
                Orientation = WpfOrientation.Horizontal,
                HorizontalAlignment = WpfHorizontalAlignment.Right,
            };
            foreach (var button in buttons)
            {
                button.Margin = new Thickness(8, 0, 0, 0);
                row.Children.Add(button);
            }
            return row;
        }

        async Task AddRecoveryPubkeyAsync(string recoveryPubkey)
        {
            await service.AddRecoveryDeviceAsync(recoveryPubkey);
            NoticeText.Text = "Recovery key added";
            await RefreshAsync();
            dialog.Close();
        }

        void ShowChoices()
        {
            ResetBody("Add Recovery Key");
            notice.Text = "";

            var generate = new WpfButton
            {
                Content = "Generate New",
                Style = (Style)FindResource("PrimaryButton"),
                MinWidth = 140,
                Margin = new Thickness(0, 0, 0, 8),
            };
            var import = new WpfButton
            {
                Content = "Import Existing",
                MinWidth = 140,
                Margin = new Thickness(0, 0, 0, 12),
            };
            var cancel = new WpfButton { Content = "Cancel", MinWidth = 92 };

            generate.Click += (_, _) => ShowGeneratedRecoveryKey();
            import.Click += (_, _) => ShowImportRecoveryKey();
            cancel.Click += (_, _) => dialog.Close();

            body.Children.Add(generate);
            body.Children.Add(import);
            body.Children.Add(Buttons(cancel));
        }

        void ShowGeneratedRecoveryKey()
        {
            var generated = service.GenerateRecoveryKey();
            if (!string.IsNullOrWhiteSpace(generated.Error) ||
                generated.Words.Count != RecoveryPhraseWordCount ||
                string.IsNullOrWhiteSpace(generated.RecoveryPubkey))
            {
                ResetBody("Generate Recovery Key");
                notice.Text = string.IsNullOrWhiteSpace(generated.Error)
                    ? "Recovery key generation failed"
                    : generated.Error;
                var errorBack = new WpfButton { Content = "Back", MinWidth = 92 };
                var close = new WpfButton { Content = "Close", MinWidth = 92 };
                errorBack.Click += (_, _) => ShowChoices();
                close.Click += (_, _) => dialog.Close();
                body.Children.Add(Buttons(errorBack, close));
                return;
            }

            var wordIndex = 0;
            ResetBody("Generate Recovery Key");
            notice.Text = "Write down each word. Iris Drive will only save the public recovery key.";
            var wordLabel = new TextBlock
            {
                Style = (Style)FindResource("FieldName"),
                Margin = new Thickness(0, 0, 0, 8),
            };
            var wordValue = new TextBlock
            {
                FontSize = 30,
                FontWeight = FontWeights.Bold,
                TextAlignment = TextAlignment.Center,
                Margin = new Thickness(0, 8, 0, 16),
            };
            var cancel = new WpfButton { Content = "Cancel", MinWidth = 92 };
            var back = new WpfButton { Content = "Back", MinWidth = 92 };
            var next = new WpfButton
            {
                Content = "Next",
                Style = (Style)FindResource("PrimaryButton"),
                MinWidth = 132,
            };

            void RenderWord()
            {
                wordLabel.Text = $"Word {wordIndex + 1} of {RecoveryPhraseWordCount}";
                wordValue.Text = generated.Words[wordIndex];
                back.IsEnabled = wordIndex > 0;
                next.Content = wordIndex == RecoveryPhraseWordCount - 1 ? "Add Recovery Key" : "Next";
            }

            cancel.Click += (_, _) => dialog.Close();
            back.Click += (_, _) =>
            {
                wordIndex = Math.Max(0, wordIndex - 1);
                RenderWord();
            };
            next.Click += async (_, _) =>
            {
                if (wordIndex >= RecoveryPhraseWordCount - 1)
                {
                    next.IsEnabled = false;
                    try
                    {
                        await AddRecoveryPubkeyAsync(generated.RecoveryPubkey);
                    }
                    catch (Exception error)
                    {
                        notice.Text = error.Message;
                        next.IsEnabled = true;
                    }
                    return;
                }

                wordIndex = Math.Min(RecoveryPhraseWordCount - 1, wordIndex + 1);
                RenderWord();
            };

            body.Children.Add(wordLabel);
            body.Children.Add(wordValue);
            body.Children.Add(Buttons(cancel, back, next));
            RenderWord();
        }

        void ShowImportRecoveryKey()
        {
            var words = new string[RecoveryPhraseWordCount];
            var wordIndex = 0;
            ResetBody("Import Recovery Key");
            notice.Text = "Enter the recovery phrase one word at a time.";
            var wordLabel = new TextBlock
            {
                Style = (Style)FindResource("FieldName"),
                Margin = new Thickness(0, 0, 0, 4),
            };
            var wordBox = new WpfTextBox
            {
                Tag = "Word",
                MinHeight = 34,
                Margin = new Thickness(0, 0, 0, 14),
            };
            var cancel = new WpfButton { Content = "Cancel", MinWidth = 92 };
            var back = new WpfButton { Content = "Back", MinWidth = 92 };
            var next = new WpfButton
            {
                Content = "Next",
                Style = (Style)FindResource("PrimaryButton"),
                MinWidth = 132,
            };

            void StoreCurrentWord()
            {
                words[wordIndex] = wordBox.Text.Trim().ToLowerInvariant();
            }

            void UpdateImportButtons()
            {
                back.IsEnabled = wordIndex > 0;
                next.Content = wordIndex == RecoveryPhraseWordCount - 1 ? "Add Recovery Key" : "Next";
                next.IsEnabled = !string.IsNullOrWhiteSpace(wordBox.Text) &&
                    (wordIndex < RecoveryPhraseWordCount - 1 ||
                        words.Select((word, index) => index == wordIndex ? wordBox.Text : word)
                            .All(word => !string.IsNullOrWhiteSpace(word)));
            }

            void RenderWord()
            {
                wordLabel.Text = $"Word {wordIndex + 1} of {RecoveryPhraseWordCount}";
                wordBox.Text = words[wordIndex];
                wordBox.CaretIndex = wordBox.Text.Length;
                UpdateImportButtons();
            }

            async Task AdvanceOrSubmitAsync()
            {
                StoreCurrentWord();
                if (string.IsNullOrWhiteSpace(words[wordIndex]))
                {
                    RenderWord();
                    return;
                }

                if (wordIndex < RecoveryPhraseWordCount - 1)
                {
                    wordIndex = Math.Min(RecoveryPhraseWordCount - 1, wordIndex + 1);
                    RenderWord();
                    wordBox.Focus();
                    return;
                }

                var phrase = string.Join(" ", words.Select(word => word.Trim().ToLowerInvariant()));
                var recovery = service.RecoveryPubkeyForPhrase(phrase);
                if (!string.IsNullOrWhiteSpace(recovery.Error) ||
                    string.IsNullOrWhiteSpace(recovery.RecoveryPubkey))
                {
                    notice.Text = string.IsNullOrWhiteSpace(recovery.Error)
                        ? "Recovery key import failed"
                        : recovery.Error;
                    return;
                }

                next.IsEnabled = false;
                try
                {
                    await AddRecoveryPubkeyAsync(recovery.RecoveryPubkey);
                }
                catch (Exception error)
                {
                    notice.Text = error.Message;
                    next.IsEnabled = true;
                }
            }

            cancel.Click += (_, _) => dialog.Close();
            back.Click += (_, _) =>
            {
                StoreCurrentWord();
                wordIndex = Math.Max(0, wordIndex - 1);
                RenderWord();
                wordBox.Focus();
            };
            next.Click += async (_, _) => await AdvanceOrSubmitAsync();
            wordBox.TextChanged += (_, _) =>
            {
                StoreCurrentWord();
                UpdateImportButtons();
            };
            wordBox.KeyDown += async (_, key) =>
            {
                if (key.Key != Key.Enter)
                {
                    return;
                }
                key.Handled = true;
                await AdvanceOrSubmitAsync();
            };

            body.Children.Add(wordLabel);
            body.Children.Add(wordBox);
            body.Children.Add(Buttons(cancel, back, next));
            RenderWord();
            wordBox.Focus();
        }

        dialog.Content = body;
        ShowChoices();
        dialog.ShowDialog();
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
            NoticeText.Text = "Admin removed";
            await RefreshAsync();
        }
        catch (Exception error)
        {
            NoticeText.Text = error.Message;
        }
    }
}
