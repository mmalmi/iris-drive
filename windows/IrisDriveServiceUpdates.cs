using System.Threading.Tasks;

namespace IrisDrive.WindowsShell;

public sealed partial class IrisDriveService
{
    public Task<IrisDriveUpdateResult> CheckUpdateAsync()
    {
        return Task.Run(() => IrisDriveNativeCore.CheckUpdate(DefaultConfigDirectory, AppVersion));
    }

    public Task<IrisDriveUpdateResult> DownloadUpdateAsync(string downloadDirectory)
    {
        return Task.Run(() =>
            IrisDriveNativeCore.DownloadUpdate(DefaultConfigDirectory, AppVersion, downloadDirectory));
    }

    /// The install step the core reports for this profile.
    public IrisDriveUpdateProgress UpdateProgress()
    {
        return IrisDriveNativeCore.UpdateProgress(DefaultConfigDirectory);
    }

    /// Records a step only the window takes and returns what to show.
    public IrisDriveUpdateProgress ReportUpdateStage(string stage, string reason = "")
    {
        return IrisDriveNativeCore.ReportUpdateStage(DefaultConfigDirectory, stage, reason);
    }
}
