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
        return Task.Run(() => IrisDriveStatusData.FromNativeJson(DispatchJson(actionJson)));
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

    public static IrisDriveLinkInputClassification ClassifyLinkInput(string input)
    {
        if (string.IsNullOrWhiteSpace(input))
        {
            return IrisDriveLinkInputClassification.Empty;
        }

        return IrisDriveLinkInputClassification.FromJson(
            TakeString(iris_drive_classify_link_input_json(input.Trim())));
    }

    public static RecoverySecretExport ExportRecoverySecret(string dataDirectory)
    {
        if (string.IsNullOrWhiteSpace(dataDirectory))
        {
            return new RecoverySecretExport(false, "", Array.Empty<string>(), "", "profile is required");
        }

        return RecoverySecretExport.FromJson(
            TakeString(iris_drive_export_recovery_secret_json(dataDirectory)));
    }

    public static GeneratedRecoveryKey GenerateRecoveryKey()
    {
        return GeneratedRecoveryKey.FromJson(TakeString(iris_drive_generate_recovery_key_json()));
    }

    public static GeneratedRecoveryKey RecoveryPubkeyForPhrase(string recoveryPhrase)
    {
        if (string.IsNullOrWhiteSpace(recoveryPhrase))
        {
            return new GeneratedRecoveryKey(Array.Empty<string>(), "", "Recovery phrase is required");
        }

        return GeneratedRecoveryKey.FromJson(
            TakeString(iris_drive_recovery_pubkey_for_phrase_json(recoveryPhrase.Trim())));
    }

    public static IrisDriveUpdateResult CheckUpdate(
        string dataDirectory,
        string currentVersion,
        string mode = "app")
    {
        if (string.IsNullOrWhiteSpace(dataDirectory))
        {
            return IrisDriveUpdateResult.ErrorResult("profile is required");
        }

        return IrisDriveUpdateResult.FromJson(
            TakeString(iris_drive_update_check_json(
                dataDirectory,
                currentVersion ?? "",
                mode ?? "app")));
    }

    public static IrisDriveUpdateResult DownloadUpdate(
        string dataDirectory,
        string currentVersion,
        string downloadDirectory,
        string mode = "app")
    {
        if (string.IsNullOrWhiteSpace(dataDirectory))
        {
            return IrisDriveUpdateResult.ErrorResult("profile is required");
        }

        return IrisDriveUpdateResult.FromJson(
            TakeString(iris_drive_update_download_json(
                dataDirectory,
                currentVersion ?? "",
                mode ?? "app",
                downloadDirectory ?? "")));
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
    private static extern IntPtr iris_drive_classify_link_input_json(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string text);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_export_recovery_secret_json(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataDir);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_generate_recovery_key_json();

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_recovery_pubkey_for_phrase_json(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string recoveryPhrase);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_update_check_json(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataDir,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string currentVersion,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string mode);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr iris_drive_update_download_json(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string dataDir,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string currentVersion,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string mode,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string downloadDir);

    [DllImport("iris_drive_app_core", CallingConvention = CallingConvention.Cdecl)]
    private static extern void iris_drive_string_free(IntPtr value);
}

public sealed record IrisDriveLinkInputClassification(
    string Kind,
    bool IsComplete,
    bool IsValid,
    string NormalizedInput,
    string AppKeyPubkey,
    string AdminAppKeyPubkey,
    bool HasLinkSecret,
    string ShareSourcePath,
    string ShareDisplayName,
    string ShareRecipientNpubHint,
    string ShareRecipientDisplayName,
    string ShareRecipientProfileId,
    string ContentNhash,
    string ContentPathHint,
    string OpenDisplayName,
    string LocalOpenUrl,
    string Error)
{
    public static IrisDriveLinkInputClassification Empty { get; } = new(
        "empty",
        false,
        false,
        "",
        "",
        "",
        false,
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "");

    public static IrisDriveLinkInputClassification FromJson(string json)
    {
        using var document = JsonDocument.Parse(string.IsNullOrWhiteSpace(json) ? "{}" : json);
        var root = document.RootElement;
        return new IrisDriveLinkInputClassification(
            String(root, "kind") ?? "unknown",
            Bool(root, "is_complete"),
            Bool(root, "is_valid"),
            String(root, "normalized_input") ?? "",
            String(root, "app_key_pubkey") ?? "",
            String(root, "admin_app_key_pubkey") ?? "",
            Bool(root, "has_link_secret"),
            String(root, "share_source_path") ?? "",
            String(root, "share_display_name") ?? "",
            String(root, "share_recipient_npub_hint") ?? "",
            String(root, "share_recipient_display_name") ?? "",
            String(root, "share_recipient_profile_id") ?? "",
            String(root, "content_nhash") ?? "",
            String(root, "content_path_hint") ?? "",
            String(root, "open_display_name") ?? "",
            String(root, "local_open_url") ?? "",
            String(root, "error") ?? "");
    }

    private static string? String(JsonElement element, string property)
    {
        return element.ValueKind == JsonValueKind.Object &&
            element.TryGetProperty(property, out var value) &&
            value.ValueKind == JsonValueKind.String
                ? value.GetString()
                : null;
    }

    private static bool Bool(JsonElement element, string property)
    {
        return element.ValueKind == JsonValueKind.Object &&
            element.TryGetProperty(property, out var value) &&
            value.ValueKind == JsonValueKind.True;
    }
}
