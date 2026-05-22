using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace IrisDrive.WindowsShell;

public sealed class IrisDriveService
{
    public string DefaultDriveDirectory =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
            "Iris Drive");

    public string DefaultConfigDirectory =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "iris-drive");

    public async Task<(IrisDriveStatusData Status, string? Notice)> StatusWithLocalImportAsync()
    {
        var status = await StatusAsync();
        if (!status.Initialized)
        {
            return (status, null);
        }
        if (status.IsAwaitingLinkedApproval)
        {
            return (status, null);
        }

        try
        {
            await ImportDriveFolderAsync(status.WorkingDirectory ?? DefaultDriveDirectory);
            status = await StatusAsync();
            return (status, null);
        }
        catch (Exception error)
        {
            return (status, $"Could not scan drive folder: {error.Message}");
        }
    }

    public async Task<IrisDriveStatusData> StatusAsync()
    {
        using var document = await RunJsonAsync("status");
        return IrisDriveStatusData.FromJson(document.RootElement, DefaultDriveDirectory);
    }

    public Task CreateProfileAsync(string label)
    {
        return FinishSetupAsync(BuildLabelArgs(new[] { "init", "--force" }, label));
    }

    public Task RestoreProfileAsync(string secret, string label)
    {
        if (string.IsNullOrWhiteSpace(secret))
        {
            throw new InvalidOperationException("Secret key is required.");
        }

        return FinishSetupAsync(BuildLabelArgs(new[] { "restore", secret.Trim() }, label));
    }

    public Task LinkDeviceAsync(string owner, string label)
    {
        if (string.IsNullOrWhiteSpace(owner))
        {
            throw new InvalidOperationException("Owner public key is required.");
        }

        return FinishSetupAsync(BuildLabelArgs(new[] { "link", owner.Trim() }, label));
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

    public async Task ImportDriveFolderAsync(string folder)
    {
        Directory.CreateDirectory(folder);
        await RunAsync("import", folder);
    }

    public Process StartDaemonProcess()
    {
        var process = new Process
        {
            StartInfo = CreateStartInfo("daemon", "--watch-interval", "10"),
            EnableRaisingEvents = true,
        };
        process.StartInfo.Environment["IRIS_DRIVE_PARENT_PID"] =
            Environment.ProcessId.ToString();
        process.OutputDataReceived += (_, _) => { };
        process.ErrorDataReceived += (_, _) => { };
        process.Start();
        process.BeginOutputReadLine();
        process.BeginErrorReadLine();
        return process;
    }

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
        Directory.CreateDirectory(path);
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
        Directory.CreateDirectory(DefaultDriveDirectory);
        await RunAsync(arguments);
        await ImportDriveFolderAsync(DefaultDriveDirectory);
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
