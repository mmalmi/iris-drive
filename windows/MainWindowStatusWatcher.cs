using System.IO;
using System.Threading.Tasks;

namespace IrisDrive.WindowsShell;

public partial class MainWindow
{
    private void StartStatusWatcher()
    {
        Directory.CreateDirectory(service.DefaultConfigDirectory);
        statusWatcher = new FileSystemWatcher(service.DefaultConfigDirectory)
        {
            NotifyFilter = NotifyFilters.FileName | NotifyFilters.LastWrite | NotifyFilters.Size,
        };
        statusWatcher.Changed += (_, eventArgs) => StatusFileChanged(eventArgs.Name);
        statusWatcher.Created += (_, eventArgs) => StatusFileChanged(eventArgs.Name);
        statusWatcher.Deleted += (_, eventArgs) => StatusFileChanged(eventArgs.Name);
        statusWatcher.Renamed += (_, eventArgs) => StatusFileChanged(eventArgs.Name);
        statusWatcher.EnableRaisingEvents = true;
    }

    private void StatusFileChanged(string? name)
    {
        var refreshProvider = name is "config.toml" or "provider-root.changed";
        if (!refreshProvider && name is not ("daemon-status.json" or "daemon.lock"))
        {
            return;
        }

        _ = Dispatcher.InvokeAsync(() => ScheduleStatusRefresh(refreshProvider));
    }

    private void ScheduleStatusRefresh(bool refreshProvider)
    {
        refreshPending = true;
        providerRefreshPending |= refreshProvider;
        refreshTimer.Stop();
        refreshTimer.Start();
    }

    private async Task RunPendingRefreshAsync()
    {
        if (refreshing)
        {
            refreshTimer.Start();
            return;
        }

        var refreshProvider = providerRefreshPending;
        refreshPending = false;
        providerRefreshPending = false;
        await RefreshAsync(refreshProvider);
        if (refreshPending)
        {
            refreshTimer.Start();
        }
    }
}
