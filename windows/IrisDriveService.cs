using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace IrisDrive.WindowsShell;

public sealed class IrisDriveService
{
    private const string LocalMutationScanEnv = "IRIS_DRIVE_WINDOWS_CLOUD_SCAN_LOCAL_MUTATIONS";
    private static readonly SemaphoreSlim ProviderMutationGate = new(1, 1);

    public string DefaultConfigDirectory =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "iris-drive");

    public async Task<IrisDriveStatusData> StatusAsync()
    {
        using var document = await RunJsonAsync("status");
        return IrisDriveStatusData.FromJson(document.RootElement);
    }

    public Task CreateProfileAsync(string username, string profilePhotoPath)
    {
        var args = new[] { "init", "--force" }.ToList();
        if (!string.IsNullOrWhiteSpace(username))
        {
            args.Add("--username");
            args.Add(username.Trim());
            if (!string.IsNullOrWhiteSpace(profilePhotoPath))
            {
                args.Add("--profile-photo");
                args.Add(profilePhotoPath.Trim());
            }
        }
        return FinishSetupAsync(args.ToArray());
    }

    public Task RestoreProfileAsync(string secret)
    {
        if (string.IsNullOrWhiteSpace(secret))
        {
            throw new InvalidOperationException("Secret key is required.");
        }

        return FinishSetupAsync(new[] { "restore", secret.Trim() });
    }

    public Task LinkDeviceAsync(string owner)
    {
        if (string.IsNullOrWhiteSpace(owner))
        {
            throw new InvalidOperationException("Owner public key is required.");
        }

        return FinishSetupAsync(new[] { "link", owner.Trim() });
    }

    public async Task ApproveDeviceAsync(string device, string label)
    {
        if (string.IsNullOrWhiteSpace(device))
        {
            throw new InvalidOperationException("Device key is required.");
        }

        await RunAsync(BuildLabelArgs(new[] { "approve", device.Trim() }, label));
    }

    public Task RevokeDeviceAsync(string device)
    {
        if (string.IsNullOrWhiteSpace(device))
        {
            throw new InvalidOperationException("Device key is required.");
        }

        return RunAsync("revoke", device.Trim());
    }

    public Task AppointAdminAsync(string device)
    {
        if (string.IsNullOrWhiteSpace(device))
        {
            throw new InvalidOperationException("Device key is required.");
        }

        return RunAsync("devices", "appoint-admin", device.Trim());
    }

    public Task DemoteAdminAsync(string device)
    {
        if (string.IsNullOrWhiteSpace(device))
        {
            throw new InvalidOperationException("Device key is required.");
        }

        return RunAsync("devices", "demote-admin", device.Trim());
    }

    public Task AddRelayAsync(string relay)
    {
        if (string.IsNullOrWhiteSpace(relay))
        {
            return Task.CompletedTask;
        }

        return RunAsync("relays", "add", relay.Trim());
    }

    public Task ResetRelaysAsync()
    {
        return RunAsync("relays", "reset");
    }

    public Task AddBackupTargetAsync(string target, string label)
    {
        if (string.IsNullOrWhiteSpace(target))
        {
            return Task.CompletedTask;
        }

        var trimmedLabel = label.Trim();
        return string.IsNullOrEmpty(trimmedLabel)
            ? RunAsync("backups", "add", target.Trim())
            : RunAsync("backups", "add", target.Trim(), "--label", trimmedLabel);
    }

    public Task SyncBackupsAsync()
    {
        return RunAsync("backups", "sync");
    }

    public Task CheckBackupsAsync()
    {
        return RunAsync("backups", "check");
    }

    public Task SetNhashResolverAsync(bool enabled)
    {
        return RunAsync("nhash-resolver", enabled ? "enable" : "disable");
    }

    public Task LogoutAsync()
    {
        return RunAsync("logout");
    }

    public Process StartDaemonProcess()
    {
        var process = new Process
        {
            StartInfo = CreateStartInfo(
                "daemon",
                "--watch-debounce-ms",
                "100"),
            EnableRaisingEvents = true,
        };
        process.OutputDataReceived += (_, _) => { };
        process.ErrorDataReceived += (_, _) => { };
        process.Start();
        process.BeginOutputReadLine();
        process.BeginErrorReadLine();
        return process;
    }

    public async Task<DriveFolderPreparation> PrepareDriveFolderAsync()
    {
        var entries = await ProviderEntriesAsync();
        WindowsCloudFiles.ReconcilePendingProviderMutations(entries);
        var previousState = WindowsCloudFiles.LoadLocalState(DefaultConfigDirectory);
        if (LocalMutationScanEnabled &&
            await PublishRecentLocalFileMutationsAsync(entries, previousState))
        {
            entries = await ProviderEntriesAsync();
            WindowsCloudFiles.ReconcilePendingProviderMutations(entries);
        }

        var preparation = WindowsCloudFiles.EnsureSyncRoot(
            entries,
            ReadProviderFile,
            QueueProviderDelete,
            QueueProviderRename,
            previousState);
        WindowsCloudFiles.DebugLog(
            $"prepare entries={entries.Count} native={preparation.NativeSyncRootReady} " +
            $"placeholders={preparation.PlaceholderCount} skipped={preparation.SkippedLocalItemCount} " +
            $"warning={preparation.Warning ?? ""}");
        if (!preparation.NativeSyncRootReady)
        {
            return preparation;
        }

        WindowsCloudFiles.RemoveStaleSyncedLocalItems(
            entries,
            previousState,
            preparation.ProtectedLocalItemPaths);
        WindowsCloudFiles.NotifyShellEntriesChanged(entries, previousState);
        WindowsCloudFiles.WriteLocalState(
            DefaultConfigDirectory,
            entries,
            ReadProviderFile,
            previousState,
            preparation.RefreshedPlaceholderPaths,
            preparation.ProtectedLocalItemPaths);
        WriteProviderPathCache(entries);
        return preparation;
    }

    private static bool LocalMutationScanEnabled =>
        string.Equals(
            Environment.GetEnvironmentVariable(LocalMutationScanEnv),
            "1",
            StringComparison.Ordinal);

    public bool DaemonLockIsRunning(IrisDriveStatusData status)
    {
        var pid = DaemonLockPid(status);
        return pid.HasValue && ProcessIsRunning(pid.Value);
    }

    public int? DaemonLockPid(IrisDriveStatusData status)
    {
        var configDirectory = status.ConfigDirectory;
        if (string.IsNullOrWhiteSpace(configDirectory))
        {
            return null;
        }

        var lockPath = Path.Combine(configDirectory, "daemon.lock");
        if (!File.Exists(lockPath))
        {
            return null;
        }

        return int.TryParse(File.ReadAllText(lockPath).Trim(), out var pid) ? pid : null;
    }

    public static bool ProcessIsRunning(int pid)
    {
        try
        {
            using var process = Process.GetProcessById(pid);
            return !process.HasExited;
        }
        catch
        {
            return false;
        }
    }

    public void OpenPath(string path)
    {
        Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
    }

    public void OpenUri(string uri)
    {
        Process.Start(new ProcessStartInfo(uri) { UseShellExecute = true });
    }

    public async Task<string> CurrentAccountValueAsync(string key)
    {
        using var document = await RunJsonAsync("status");
        if (!document.RootElement.TryGetProperty("account", out var account) ||
            account.ValueKind != JsonValueKind.Object ||
            !account.TryGetProperty(key, out var value) ||
            value.ValueKind != JsonValueKind.String)
        {
            throw new InvalidOperationException("No account key available.");
        }

        return value.GetString() ?? "";
    }

    private async Task FinishSetupAsync(string[] arguments)
    {
        await RunAsync(arguments);
    }

    private static string[] BuildLabelArgs(string[] prefix, string label)
    {
        var trimmed = label.Trim();
        return string.IsNullOrEmpty(trimmed)
            ? prefix
            : prefix.Concat(new[] { "--label", trimmed }).ToArray();
    }

    private async Task<JsonDocument> RunJsonAsync(params string[] arguments)
    {
        var output = await RunForOutputAsync(arguments);
        return JsonDocument.Parse(output);
    }

    private Task RunAsync(params string[] arguments)
    {
        return RunForOutputAsync(arguments);
    }

    private async Task RunProviderMutationAsync(params string[] arguments)
    {
        await ProviderMutationGate.WaitAsync();
        try
        {
            await RunAsync(arguments);
        }
        finally
        {
            ProviderMutationGate.Release();
        }
    }

    private async Task<IReadOnlyList<WindowsCloudFileEntry>> ProviderEntriesAsync()
    {
        using var document = await RunJsonAsync("provider", "list");
        if (!document.RootElement.TryGetProperty("entries", out var entries) ||
            entries.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<WindowsCloudFileEntry>();
        }

        return entries
            .EnumerateArray()
            .Select(WindowsCloudFileEntry.FromJson)
            .Where(entry => !string.IsNullOrWhiteSpace(entry.Path))
            .ToArray();
    }

    private void WriteProviderPathCache(IReadOnlyList<WindowsCloudFileEntry> entries)
    {
        try
        {
            Directory.CreateDirectory(DefaultConfigDirectory);
            var paths = entries
                .Select(entry => entry.Path)
                .Where(path => !string.IsNullOrWhiteSpace(path))
                .Where(WindowsCloudFiles.SyncRootEntryExists)
                .Distinct(StringComparer.Ordinal)
                .OrderBy(path => path, StringComparer.Ordinal)
                .ToArray();
            var json = JsonSerializer.Serialize(new { paths });
            File.WriteAllText(
                Path.Combine(DefaultConfigDirectory, "windows-cloud-provider-paths.json"),
                json);
        }
        catch
        {
            // The Rust watcher treats this as an optimization hint; sync still works without it.
        }
    }

    private async Task<bool> PublishRecentLocalFileMutationsAsync(
        IReadOnlyList<WindowsCloudFileEntry> entries,
        IReadOnlyList<WindowsCloudLocalStateEntry> previousState)
    {
        var upserts = WindowsCloudFiles.RecentLocalFileUpserts(entries, previousState);
        var deletes = WindowsCloudFiles.RecentLocalFileDeletes(entries, previousState);

        var changed = false;
        foreach (var delete in deletes)
        {
            if (!WindowsCloudFiles.TryMarkProviderDeletePending(delete.Path))
            {
                WindowsCloudFiles.DebugLog(
                    $"provider delete skipped from local scan because mutation is pending path={delete.Path}");
                continue;
            }

            try
            {
                WindowsCloudFiles.DebugLog($"provider delete start from local scan path={delete.Path}");
                await RunProviderMutationAsync("provider", "delete", delete.Path);
                WindowsCloudFiles.DebugLog($"provider delete published from local scan path={delete.Path}");
                changed = true;
            }
            catch (Exception error)
            {
                WindowsCloudFiles.ClearProviderMutationPending(delete.Path);
                WindowsCloudFiles.DebugLog(
                    $"provider delete local scan failed path={delete.Path} error={error.Message}");
            }
        }

        foreach (var upsert in upserts)
        {
            if (WindowsCloudFiles.ProviderMutationIsPending(upsert.Path))
            {
                WindowsCloudFiles.DebugLog(
                    $"provider write skipped from local scan because mutation is pending path={upsert.Path}");
                continue;
            }

            try
            {
                WindowsCloudFiles.DebugLog($"provider write start from local scan path={upsert.Path}");
                await RunProviderMutationAsync("provider", "write", upsert.Path, upsert.FullPath);
                WindowsCloudFiles.DebugLog($"provider write published from local scan path={upsert.Path}");
                changed = true;
            }
            catch (Exception error)
            {
                WindowsCloudFiles.DebugLog(
                    $"provider write local scan failed path={upsert.Path} error={error.Message}");
            }
        }

        return changed;
    }

    private byte[] ReadProviderFile(string path)
    {
        var tempDirectory = Path.Combine(Path.GetTempPath(), "iris-drive");
        Directory.CreateDirectory(tempDirectory);
        var tempFile = Path.Combine(tempDirectory, $"{Guid.NewGuid():N}.bin");
        try
        {
            RunForOutput("provider", "read", path, tempFile);
            return File.ReadAllBytes(tempFile);
        }
        finally
        {
            try
            {
                File.Delete(tempFile);
            }
            catch
            {
                // Best-effort cleanup for provider callback scratch files.
            }
        }
    }

    private void QueueProviderDelete(string path)
    {
        if (!WindowsCloudFiles.MarkProviderDeletePending(path))
        {
            WindowsCloudFiles.DebugLog($"provider delete skipped from Cloud Files notify path={path}");
            return;
        }

        WindowsCloudFiles.DebugLog($"provider delete queued from Cloud Files notify path={path}");
        _ = Task.Run(async () =>
        {
            var deletePath = path;
            try
            {
                for (var attempt = 0; attempt < 50 && WindowsCloudFiles.SyncRootEntryExists(path); attempt++)
                {
                    await Task.Delay(100);
                }
                if (!WindowsCloudFiles.ProviderDeleteIsPending(path))
                {
                    WindowsCloudFiles.DebugLog(
                        $"provider delete skipped because pending marker was coalesced path={path}");
                    return;
                }

                deletePath = WindowsCloudFiles.PromoteProviderDeleteToMissingAncestor(path);
                if (!WindowsCloudFiles.ProviderDeleteIsPending(deletePath))
                {
                    WindowsCloudFiles.DebugLog(
                        $"provider delete skipped because promoted marker was coalesced path={deletePath}");
                    return;
                }

                if (WindowsCloudFiles.SyncRootEntryExists(deletePath))
                {
                    WindowsCloudFiles.DebugLog(
                        $"provider delete continuing even though local path still exists path={deletePath}");
                }

                WindowsCloudFiles.DebugLog($"provider delete start from Cloud Files notify path={deletePath}");
                await RunProviderMutationAsync("provider", "delete", deletePath);
                WindowsCloudFiles.DebugLog($"provider delete published from Cloud Files notify path={deletePath}");
            }
            catch (Exception error)
            {
                WindowsCloudFiles.ClearProviderMutationPending(path, deletePath);
                WindowsCloudFiles.DebugLog($"provider delete notify failed path={path} error={error.Message}");
            }
        });
    }

    private void QueueProviderRename(string oldPath, string newPath)
    {
        WindowsCloudFiles.MarkProviderRenamePending(oldPath, newPath);
        WindowsCloudFiles.DebugLog(
            $"provider rename queued from Cloud Files notify old={oldPath} new={newPath}");
        _ = Task.Run(async () =>
        {
            try
            {
                WindowsCloudFiles.DebugLog(
                    $"provider rename start from Cloud Files notify old={oldPath} new={newPath}");
                await RunProviderMutationAsync("provider", "rename", oldPath, newPath);
                WindowsCloudFiles.DebugLog(
                    $"provider rename published from Cloud Files notify old={oldPath} new={newPath}");
            }
            catch (Exception error)
            {
                WindowsCloudFiles.ClearProviderMutationPending(oldPath, newPath);
                WindowsCloudFiles.DebugLog(
                    $"provider rename notify failed old={oldPath} new={newPath} error={error.Message}");
            }
        });
    }

    private async Task<string> RunForOutputAsync(params string[] arguments)
    {
        using var process = new Process { StartInfo = CreateStartInfo(arguments) };
        process.Start();
        var stdout = await process.StandardOutput.ReadToEndAsync();
        var stderr = await process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();

        if (process.ExitCode == 0)
        {
            return stdout;
        }

        var message = string.IsNullOrWhiteSpace(stderr) ? stdout : stderr;
        throw new InvalidOperationException(message.Trim());
    }

    private string RunForOutput(params string[] arguments)
    {
        using var process = new Process { StartInfo = CreateStartInfo(arguments) };
        process.Start();
        var stdout = process.StandardOutput.ReadToEnd();
        var stderr = process.StandardError.ReadToEnd();
        process.WaitForExit();

        if (process.ExitCode == 0)
        {
            return stdout;
        }

        var message = string.IsNullOrWhiteSpace(stderr) ? stdout : stderr;
        throw new InvalidOperationException(message.Trim());
    }

    private ProcessStartInfo CreateStartInfo(params string[] arguments)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = ResolveIdrivePath(),
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };

        foreach (var argument in arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private string ResolveIdrivePath()
    {
        var overridePath = Environment.GetEnvironmentVariable("IRIS_DRIVE_CLI");
        if (!string.IsNullOrWhiteSpace(overridePath))
        {
            return overridePath;
        }

        foreach (var candidate in CandidateIdrivePaths())
        {
            if (File.Exists(candidate))
            {
                return candidate;
            }
        }

        return "idrive.exe";
    }

    private static string[] CandidateIdrivePaths()
    {
        var exe = "idrive.exe";
        var current = Directory.GetCurrentDirectory();
        var app = AppContext.BaseDirectory;
        return new[]
        {
            Path.Combine(app, exe),
            Path.Combine(current, exe),
            Path.Combine(current, "..", "target", "debug", exe),
            Path.Combine(current, "..", "target", "release", exe),
            Path.Combine(current, "..", "..", "target", "debug", exe),
            Path.Combine(current, "..", "..", "target", "release", exe),
            Path.Combine(app, "..", "..", "..", "..", "target", "debug", exe),
            Path.Combine(app, "..", "..", "..", "..", "target", "release", exe),
            Path.Combine(app, "..", "..", "..", "..", "..", "target", "debug", exe),
            Path.Combine(app, "..", "..", "..", "..", "..", "target", "release", exe),
        }.Select(Path.GetFullPath).ToArray();
    }
}
