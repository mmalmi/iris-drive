using System.Text.Json;

namespace IrisDrive.WindowsShell;

/// One step of an update install, worded by the core. The window only renders it.
public sealed record IrisDriveUpdateProgress(
    string Stage,
    string Text,
    double? Fraction,
    bool Active,
    bool Shown,
    bool CanRetry)
{
    public static readonly IrisDriveUpdateProgress Idle = new("idle", "", null, false, false, false);

    public static IrisDriveUpdateProgress FromJson(string json)
    {
        using var document = JsonDocument.Parse(string.IsNullOrWhiteSpace(json) ? "{}" : json);
        var root = document.RootElement;
        if (root.ValueKind != JsonValueKind.Object)
        {
            return Idle;
        }
        // A snapshot has a stage; a failed call has only an error.
        if (!root.TryGetProperty("stage", out _) &&
            root.TryGetProperty("error", out var error) &&
            error.ValueKind == JsonValueKind.String)
        {
            return new("failed", error.GetString() ?? "", null, false, true, true);
        }
        return new(
            String(root, "stage") ?? "idle",
            String(root, "text") ?? "",
            root.TryGetProperty("fraction", out var fraction) && fraction.ValueKind == JsonValueKind.Number
                ? fraction.GetDouble()
                : null,
            Flag(root, "active"),
            Flag(root, "shown"),
            Flag(root, "can_retry"));
    }

    private static string? String(JsonElement root, string name) =>
        root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private static bool Flag(JsonElement root, string name) =>
        root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.True;
}
