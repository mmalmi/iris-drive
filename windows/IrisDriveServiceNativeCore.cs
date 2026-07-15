using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace IrisDrive.WindowsShell;

public sealed partial class IrisDriveService
{
    private readonly SemaphoreSlim nativeCoreGate = new(1, 1);

    private async Task<T> RunNativeCoreAsync<T>(Func<IrisDriveNativeCore, T> operation)
    {
        await nativeCoreGate.WaitAsync().ConfigureAwait(false);
        try
        {
            return await Task.Run(() => operation(NativeCore)).ConfigureAwait(false);
        }
        finally
        {
            nativeCoreGate.Release();
        }
    }

    private Task<IrisDriveStatusData> DispatchNativeActionAsync(
        IReadOnlyDictionary<string, object> action
    )
    {
        return RunNativeCoreAsync(core => core.DispatchAction(action));
    }
}
