using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Threading.Tasks;

namespace IrisDrive.WindowsShell;

public sealed class IrisDriveNativeCore : IDisposable
{
    private IntPtr handle;

    public IrisDriveNativeCore(string dataDirectory, string appVersion)
    {
        handle = iris_drive_app_new(dataDirectory, appVersion);
        if (handle == IntPtr.Zero)
        {
            throw new InvalidOperationException("Native app-core did not initialize.");
        }
    }

    public string RefreshJson()
    {
        return TakeString(iris_drive_app_refresh_json(handle));
    }

    public string DispatchJson(string actionJson)
    {
        return TakeString(iris_drive_app_dispatch_json(handle, actionJson));
    }

    public Task<IrisDriveStatusData> DispatchActionAsync(IReadOnlyDictionary<string, object> action)
    {
        var actionJson = JsonSerializer.Serialize(action);
        return Task.FromResult(IrisDriveStatusData.FromNativeJson(DispatchJson(actionJson)));
    }

    public static bool IsCompleteLinkInput(string input)
    {
        if (string.IsNullOrWhiteSpace(input))
        {
            return false;
        }

        using var document = JsonDocument.Parse(
            TakeString(iris_drive_validate_link_input_json(input.Trim())));
        return document.RootElement.TryGetProperty("is_complete", out var isComplete) &&
            isComplete.ValueKind == JsonValueKind.True;
    }

    public void Dispose()
    {
        if (handle != IntPtr.Zero)
        {
            iris_drive_app_free(handle);
            handle = IntPtr.Zero;
        }
    }

    private static string TakeString(IntPtr pointer)
    {
        if (pointer == IntPtr.Zero)
        {
            return "{\"error\":\"native app-core returned null\"}";
        }

        try
        {
            return Marshal.PtrToStringUTF8(pointer) ?? "";
        }
        finally
        {
            iris_drive_string_free(pointer);
        }
    }

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_app_new(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataDir,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string appVersion);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern void iris_drive_app_free(IntPtr handle);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_app_refresh_json(IntPtr handle);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_app_dispatch_json(
        IntPtr handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string actionJson);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_validate_link_input_json(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string text);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern void iris_drive_string_free(IntPtr value);
}
