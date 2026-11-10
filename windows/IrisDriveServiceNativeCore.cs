using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace IrisDrive.WindowsShell;

public sealed partial class IrisDriveService
{
    public string? SignerInteraction(bool cancel)
    {
        using var result = JsonDocument.Parse(NativeCore.DispatchJson(cancel
            ? "{\"type\":\"cancel_signer_recovery\"}" : "{\"type\":\"signer_approval_link\"}"));
        return result.RootElement.TryGetProperty("signer_approval_url", out var url) ? url.GetString() : null;
    }

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
