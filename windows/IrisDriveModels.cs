using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace IrisDrive.WindowsShell;

public sealed class IrisDriveStatusData
{
    public bool Initialized { get; init; }
    public string DriveName { get; init; } = "My Drive";
    public string? CurrentAppKeyNpub { get; init; }
    public string? DeviceNpub { get; init; }
    public bool CanAdminProfile { get; init; }
    public bool CanExportRecoveryPhrase { get; init; }
    public string? AuthorizationState { get; init; }
    public string SetupState { get; init; } = "not_configured";
    public string SetupLabel { get; init; } = "Not linked";
    public bool SetupComplete { get; init; }
    public bool AwaitingApproval { get; init; }
    public bool Revoked { get; init; }
    public string PrimaryStatus { get; init; } = "not_setup";
    public string PrimaryStatusLabel { get; init; } = "Ready";
    public string? AppKeyLinkRequestUrl { get; init; }
    public IReadOnlyList<AppKeyLinkRequestRow> AppKeyLinkRequests { get; init; } =
        Array.Empty<AppKeyLinkRequestRow>();
    public int AuthorizedDeviceCount { get; init; }
    public int OnlineDeviceCount { get; init; }
    public string? ConfigDirectory { get; init; }
    public string? CurrentRootCid { get; init; }
    public string ProviderRefreshKey { get; init; } = "";
    public string? SnapshotUrl { get; init; }
    public string? LastShareInviteUrl { get; init; }
    public string? LastShareRecipientEvidence { get; init; }
    public int FileCount { get; init; }
    public long VisibleFileBytes { get; init; }
    public bool LocalNhashResolverEnabled { get; init; } = true;
    public IReadOnlyList<DriveRow> Drives { get; init; } = Array.Empty<DriveRow>();
    public IReadOnlyList<PeerRow> Peers { get; init; } = Array.Empty<PeerRow>();
    public IReadOnlyList<BackupTargetRow> BackupTargets { get; init; } =
        Array.Empty<BackupTargetRow>();
    public IReadOnlyList<ShareRow> Shares { get; init; } = Array.Empty<ShareRow>();
    public IReadOnlyList<string> Relays { get; init; } = Array.Empty<string>();
    public IReadOnlyList<string> BlossomServers { get; init; } = Array.Empty<string>();
    public FipsDiagnostics Fips { get; init; } = FipsDiagnostics.Empty;
    public IReadOnlyList<RelayStatusRow> RelayStatuses { get; init; } =
        Array.Empty<RelayStatusRow>();

    public static IrisDriveStatusData FromNativeJson(string json)
    {
        using var document = JsonDocument.Parse(json);
        return FromNativeJson(document.RootElement);
    }

    public static IrisDriveStatusData FromNativeJson(JsonElement root)
    {
        var error = String(root, "error");
        if (!string.IsNullOrWhiteSpace(error))
        {
            throw new InvalidOperationException(error);
        }

        JsonElement ui = Object(root, "ui") ?? default;
        var profile = ui.ValueKind == JsonValueKind.Object ? Object(ui, "profile") : null;
        var paths = ui.ValueKind == JsonValueKind.Object ? Object(ui, "paths") : null;
        var setupComplete = ui.ValueKind == JsonValueKind.Object && Bool(ui, "setup_complete");

        var backupTargets = NativeBackupRows(ui);
        return new IrisDriveStatusData
        {
            Initialized = profile.HasValue,
            DriveName = NativeDriveName(ui),
            CurrentAppKeyNpub = profile.HasValue ? String(profile.Value, "current_app_key_npub") : null,
            DeviceNpub = profile.HasValue ? String(profile.Value, "current_app_key_npub") : null,
            CanAdminProfile =
                profile.HasValue && Bool(profile.Value, "can_admin_profile"),
            CanExportRecoveryPhrase =
                profile.HasValue && Bool(profile.Value, "can_export_recovery_phrase"),
            AuthorizationState =
                profile.HasValue ? String(profile.Value, "authorization_state") : null,
            SetupState = String(ui, "setup_state") ?? "not_configured",
            SetupLabel = String(ui, "setup_label") ?? "Not linked",
            SetupComplete = setupComplete,
            AwaitingApproval = Bool(ui, "awaiting_approval"),
            Revoked = Bool(ui, "revoked"),
            PrimaryStatus = String(ui, "primary_status") ?? "not_setup",
            PrimaryStatusLabel = String(ui, "primary_status_label") ?? "Ready",
            AppKeyLinkRequestUrl = profile.HasValue
                ? EmptyToNull(String(profile.Value, "app_key_link_request"))
                : null,
            AppKeyLinkRequests = profile.HasValue
                ? NativeAppKeyLinkRequests(profile.Value)
                : Array.Empty<AppKeyLinkRequestRow>(),
            AuthorizedDeviceCount = Int(ui, "authorized_device_count"),
            OnlineDeviceCount = Int(ui, "online_device_count"),
            ConfigDirectory = paths.HasValue ? String(paths.Value, "data_dir") : null,
            ProviderRefreshKey = String(ui, "provider_change_key") ?? "",
            SnapshotUrl = EmptyToNull(String(ui, "snapshot_link")),
            LastShareInviteUrl = EmptyToNull(String(ui, "last_share_invite")),
            LastShareRecipientEvidence = EmptyToNull(String(ui, "last_share_recipient_evidence")),
            FileCount = Int(ui, "file_count"),
            VisibleFileBytes = Long(ui, "visible_file_bytes"),
            LocalNhashResolverEnabled = true,
            Drives = NativeDriveRows(ui, setupComplete),
            Peers = NativePeerRows(ui),
            BackupTargets = backupTargets,
            Shares = NativeShareRows(ui),
            Relays = StringArray(ui, "relays"),
            BlossomServers = backupTargets
                .Where(target => target.Kind == "blossom")
                .Select(target => target.Target)
                .Where(target => !string.IsNullOrWhiteSpace(target))
                .ToArray(),
            Fips = FipsDiagnostics.FromJson(Object(ui, "fips")),
            RelayStatuses = RelayStatusesFromJson(ui),
        };
    }

    public bool IsAwaitingLinkedApproval =>
        Initialized && AwaitingApproval;

    public bool IsSetupComplete =>
        Initialized && SetupComplete;

    public bool IsRevoked =>
        Initialized && Revoked;

    private static string NativeDriveName(JsonElement ui)
    {
        if (ui.ValueKind == JsonValueKind.Object &&
            ui.TryGetProperty("roots", out var roots) &&
            roots.ValueKind == JsonValueKind.Array)
        {
            foreach (var root in roots.EnumerateArray())
            {
                return String(root, "name") ?? "My Drive";
            }
        }

        return "My Drive";
    }

    private static IReadOnlyList<DriveRow> NativeDriveRows(JsonElement ui, bool setupComplete)
    {
        var rows = new List<DriveRow>();
        var fallbackPath = setupComplete ? WindowsCloudFiles.SyncRootPath : null;
        if (ui.ValueKind == JsonValueKind.Object &&
            ui.TryGetProperty("roots", out var roots) &&
            roots.ValueKind == JsonValueKind.Array)
        {
            foreach (var root in roots.EnumerateArray())
            {
                rows.Add(new DriveRow(
                    String(root, "name") ?? "main",
                    String(root, "local_path") ?? fallbackPath ?? "Not ready",
                    ShortText(String(root, "status") ?? "configured")));
            }
        }

        if (rows.Count == 0)
        {
            rows.Add(new DriveRow("main", fallbackPath ?? "Not ready", "-"));
        }

        return rows;
    }

    private static IReadOnlyList<AppKeyLinkRequestRow> NativeAppKeyLinkRequests(JsonElement account)
    {
        if (!account.TryGetProperty("inbound_app_key_link_requests", out var requests) ||
            requests.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<AppKeyLinkRequestRow>();
        }

        var rows = new List<AppKeyLinkRequestRow>();
        foreach (var request in requests.EnumerateArray())
        {
            var device = String(request, "app_key_pubkey") ?? "";
            rows.Add(new AppKeyLinkRequestRow(
                device,
                String(request, "label") ?? "",
                String(request, "request_link") ?? device));
        }

        return rows;
    }

    private static IReadOnlyList<PeerRow> NativePeerRows(JsonElement ui)
    {
        if (ui.ValueKind != JsonValueKind.Object ||
            !ui.TryGetProperty("devices", out var devices) ||
            devices.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<PeerRow>();
        }

        var rows = new List<PeerRow>();
        foreach (var device in devices.EnumerateArray())
        {
            var pubkey = String(device, "pubkey") ?? "";
            var metadata = new List<string>();
            foreach (var value in new[]
            {
                String(device, "role_label"),
                String(device, "state_label"),
                String(device, "detail"),
            })
            {
                if (!string.IsNullOrWhiteSpace(value))
                {
                    metadata.Add(value);
                }
            }

            var canManagePeer = !string.IsNullOrWhiteSpace(pubkey);
            rows.Add(new PeerRow(
                pubkey,
                String(device, "display_label") ?? "",
                String(device, "role") ?? "member",
                string.Join(" | ", metadata),
                String(device, "connection_label") ?? "",
                Bool(device, "is_online"),
                Bool(device, "is_current_device"),
                canManagePeer && Bool(device, "can_revoke"),
                canManagePeer && Bool(device, "can_appoint_admin"),
                canManagePeer && Bool(device, "can_demote_admin")));
        }

        return rows;
    }

    private static IReadOnlyList<BackupTargetRow> NativeBackupRows(JsonElement ui)
    {
        if (ui.ValueKind != JsonValueKind.Object ||
            !ui.TryGetProperty("backups", out var backups) ||
            backups.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<BackupTargetRow>();
        }

        var rows = new List<BackupTargetRow>();
        foreach (var backup in backups.EnumerateArray())
        {
            var title = String(backup, "label") ?? "Backup";
            rows.Add(new BackupTargetRow(
                String(backup, "id") ?? title,
                String(backup, "kind") ?? "backup",
                String(backup, "target") ?? "",
                title,
                String(backup, "detail") ?? "",
                String(backup, "state") ?? ""));
        }

        return rows;
    }

    private static IReadOnlyList<ShareRow> NativeShareRows(JsonElement ui)
    {
        if (ui.ValueKind != JsonValueKind.Object ||
            !ui.TryGetProperty("shares", out var shares) ||
            shares.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<ShareRow>();
        }

        var rows = new List<ShareRow>();
        foreach (var share in shares.EnumerateArray())
        {
            rows.Add(new ShareRow(
                String(share, "share_id") ?? "",
                String(share, "display_name") ?? "Shared folder",
                String(share, "source_path") ?? "",
                String(share, "shared_with_me_path") ?? "",
                String(share, "role") ?? "",
                String(share, "role_label") ?? "",
                String(share, "key_status") ?? "",
                String(share, "key_status_label") ?? "",
                String(share, "write_authorization") ?? "",
                String(share, "write_authorization_label") ?? "",
                Bool(share, "can_write"),
                Bool(share, "can_admin"),
                NullableInt(share, "current_key_epoch"),
                Bool(share, "has_current_key_wrap"),
                Bool(share, "key_unavailable"),
                Bool(share, "repair_needed"),
                Int(share, "missing_key_wrap_count"),
                StringArray(share, "missing_key_wraps"),
                Int(share, "participant_count"),
                Int(share, "app_key_count"),
                NativeShareMemberRows(share),
                NativePendingShareInviteRows(share),
                StringArray(share, "shortcut_paths")));
        }

        return rows;
    }

    private static IReadOnlyList<ShareMemberRow> NativeShareMemberRows(JsonElement share)
    {
        if (!share.TryGetProperty("members", out var members) ||
            members.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<ShareMemberRow>();
        }

        var rows = new List<ShareMemberRow>();
        foreach (var member in members.EnumerateArray())
        {
            rows.Add(new ShareMemberRow(
                String(member, "profile_id") ?? "",
                String(member, "display_name") ?? "",
                String(member, "representative_npub_hint") ?? "",
                String(member, "role") ?? "",
                String(member, "role_label") ?? "",
                String(member, "status") ?? "",
                String(member, "status_label") ?? "",
                Int(member, "app_key_count"),
                Bool(member, "can_revoke"),
                Bool(member, "can_change_role")));
        }

        return rows;
    }

    private static IReadOnlyList<PendingShareInviteRow> NativePendingShareInviteRows(JsonElement share)
    {
        if (!share.TryGetProperty("pending_invites", out var invites) ||
            invites.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<PendingShareInviteRow>();
        }

        var rows = new List<PendingShareInviteRow>();
        foreach (var invite in invites.EnumerateArray())
        {
            rows.Add(new PendingShareInviteRow(
                String(invite, "representative_npub_hint") ?? "",
                String(invite, "display_name") ?? "",
                String(invite, "role") ?? "",
                String(invite, "role_label") ?? "",
                String(invite, "status") ?? "",
                String(invite, "status_label") ?? ""));
        }

        return rows;
    }

    private static string? EmptyToNull(string? value)
    {
        return string.IsNullOrWhiteSpace(value) ? null : value;
    }

    private static IReadOnlyList<string> StringArray(JsonElement root, string name)
    {
        if (root.ValueKind != JsonValueKind.Object ||
            !root.TryGetProperty(name, out var array) ||
            array.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<string>();
        }

        var values = new List<string>();
        foreach (var item in array.EnumerateArray())
        {
            if (item.ValueKind == JsonValueKind.String)
            {
                values.Add(item.GetString() ?? "");
            }
        }

        return values;
    }

    private static IReadOnlyList<RelayStatusRow> RelayStatusesFromJson(JsonElement network)
    {
        var rows = new List<RelayStatusRow>();
        if (network.ValueKind != JsonValueKind.Object ||
            !network.TryGetProperty("relay_statuses", out var statuses) ||
            statuses.ValueKind != JsonValueKind.Array)
        {
            return rows;
        }

        foreach (var status in statuses.EnumerateArray())
        {
            var url = String(status, "url");
            if (!string.IsNullOrWhiteSpace(url))
            {
                rows.Add(new RelayStatusRow(
                    url,
                    String(status, "status") ?? "unknown",
                    String(status, "status_label") ?? "",
                    String(status, "health") ?? "unknown"));
            }
        }

        return rows;
    }

    private static JsonElement? Object(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.Object
            ? value
            : null;
    }

    private static string? String(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
    }

    private static int Int(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.Number &&
            value.TryGetInt32(out var result)
            ? result
            : 0;
    }

    private static int? NullableInt(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.Number &&
            value.TryGetInt32(out var result)
            ? result
            : null;
    }

    private static long Long(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.Number &&
            value.TryGetInt64(out var result)
            ? result
            : 0;
    }

    private static bool Bool(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.True;
    }

    public static string ShortText(string value)
    {
        if (value.Length <= 32)
        {
            return value;
        }

        return $"{value[..14]}...{value[^10..]}";
    }

}

public sealed record DriveRow(string Name, string Path, string State);

public sealed record BackupTargetRow(
    string Id,
    string Kind,
    string Target,
    string Title,
    string Subtitle,
    string State);

public sealed record ShareRow(
    string ShareId,
    string DisplayName,
    string SourcePath,
    string SharedWithMePath,
    string Role,
    string RoleLabel,
    string KeyStatus,
    string KeyStatusLabel,
    string WriteAuthorization,
    string WriteAuthorizationLabel,
    bool CanWrite,
    bool CanAdmin,
    int? CurrentKeyEpoch,
    bool HasCurrentKeyWrap,
    bool KeyUnavailable,
    bool RepairNeeded,
    int MissingKeyWrapCount,
    IReadOnlyList<string> MissingKeyWraps,
    int ParticipantCount,
    int AppKeyCount,
    IReadOnlyList<ShareMemberRow> Members,
    IReadOnlyList<PendingShareInviteRow> PendingInvites,
    IReadOnlyList<string> ShortcutPaths);

public sealed record PendingShareInviteRow(
    string RepresentativeNpubHint,
    string DisplayName,
    string Role,
    string RoleLabel,
    string Status,
    string StatusLabel);

public sealed record ShareMemberRow(
    string ProfileId,
    string DisplayName,
    string RepresentativeNpubHint,
    string Role,
    string RoleLabel,
    string Status,
    string StatusLabel,
    int AppKeyCount,
    bool CanRevoke,
    bool CanChangeRole);

public sealed record FipsDiagnostics(
    bool Enabled,
    bool Running,
    bool Fresh,
    string State,
    string StateLabel,
    string? EndpointNpub,
    string? DiscoveryScope,
    string RosterLabel,
    int RosterPeerCount,
    int RosterOnlineDeviceCount,
    int RosterDirectDeviceCount,
    int OnlineDeviceCount,
    int DirectDeviceCount,
    int MeshDeviceCount,
    int OtherPeerCount,
    IReadOnlyList<FipsPeerDiagnostic> Peers,
    string? Error)
{
    public static FipsDiagnostics Empty { get; } =
        new(
            false,
            false,
            false,
            "paused",
            "Paused",
            null,
            null,
            "0/0 online",
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            Array.Empty<FipsPeerDiagnostic>(),
            null);

    public static FipsDiagnostics FromJson(JsonElement? fips)
    {
        if (!fips.HasValue)
        {
            return Empty;
        }

        var value = fips.Value;
        return new FipsDiagnostics(
            Bool(value, "enabled"),
            Bool(value, "running"),
            Bool(value, "fresh"),
            String(value, "state") ?? "paused",
            String(value, "state_label") ?? "Paused",
            String(value, "endpoint_npub"),
            String(value, "discovery_scope"),
            String(value, "roster_label") ?? "0/0 online",
            Int(value, "roster_peer_count"),
            Int(value, "roster_online_device_count"),
            Int(value, "roster_direct_device_count"),
            Int(value, "online_device_count"),
            Int(value, "direct_device_count"),
            Int(value, "mesh_device_count"),
            Int(value, "other_peer_count"),
            PeerDiagnostics(value),
            String(value, "error"));
    }

    private static IReadOnlyList<FipsPeerDiagnostic> PeerDiagnostics(JsonElement fips)
    {
        if (!fips.TryGetProperty("peer_statuses", out var peers) ||
            peers.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<FipsPeerDiagnostic>();
        }

        var rows = new List<FipsPeerDiagnostic>();
        foreach (var peer in peers.EnumerateArray())
        {
            var npub = String(peer, "npub") ?? "peer";
            var label = String(peer, "connection_label") ?? "";
            rows.Add(new FipsPeerDiagnostic(npub, label));
        }
        return rows;
    }

    private static string? String(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
    }

    private static int Int(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.Number &&
            value.TryGetInt32(out var result)
            ? result
            : 0;
    }

    private static bool Bool(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.True;
    }
}

public sealed record FipsPeerDiagnostic(string Npub, string Subtitle);

public sealed record RelayStatusRow(
    string Url,
    string Status,
    string StatusLabel,
    string Health);

public sealed record RecoverySecretExport(
    bool CanExport,
    string RecoveryPhrase,
    IReadOnlyList<string> Words,
    string SecretKey,
    string Error)
{
    public static RecoverySecretExport FromJson(string json)
    {
        using var document = JsonDocument.Parse(json);
        var root = document.RootElement;
        return new RecoverySecretExport(
            Bool(root, "can_export"),
            String(root, "recovery_phrase") ?? "",
            StringArray(root, "words"),
            String(root, "secret_key") ?? "",
            String(root, "error") ?? "");
    }

    private static string? String(JsonElement root, string name)
    {
        if (root.ValueKind != JsonValueKind.Object ||
            !root.TryGetProperty(name, out var value) ||
            value.ValueKind != JsonValueKind.String)
        {
            return null;
        }

        return value.GetString();
    }

    private static bool Bool(JsonElement root, string name) =>
        root.ValueKind == JsonValueKind.Object &&
        root.TryGetProperty(name, out var value) &&
        value.ValueKind is JsonValueKind.True or JsonValueKind.False &&
        value.GetBoolean();

    private static IReadOnlyList<string> StringArray(JsonElement root, string name)
    {
        if (root.ValueKind != JsonValueKind.Object ||
            !root.TryGetProperty(name, out var value) ||
            value.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<string>();
        }

        var items = new List<string>();
        foreach (var item in value.EnumerateArray())
        {
            if (item.ValueKind == JsonValueKind.String)
            {
                items.Add(item.GetString() ?? "");
            }
        }

        return items;
    }
}

public sealed record GeneratedRecoveryKey(
    IReadOnlyList<string> Words,
    string RecoveryPubkey,
    string Error)
{
    public static GeneratedRecoveryKey FromJson(string json)
    {
        using var document = JsonDocument.Parse(json);
        var root = document.RootElement;
        return new GeneratedRecoveryKey(
            StringArray(root, "words"),
            String(root, "recovery_pubkey") ?? "",
            String(root, "error") ?? "");
    }

    private static string? String(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.String
                ? value.GetString()
                : null;
    }

    private static IReadOnlyList<string> StringArray(JsonElement root, string name)
    {
        if (root.ValueKind != JsonValueKind.Object ||
            !root.TryGetProperty(name, out var value) ||
            value.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<string>();
        }

        var items = new List<string>();
        foreach (var item in value.EnumerateArray())
        {
            if (item.ValueKind == JsonValueKind.String)
            {
                items.Add(item.GetString() ?? "");
            }
        }

        return items;
    }
}

public sealed record IrisDriveUpdateResult(
    bool Available,
    string CurrentVersion,
    string LatestVersion,
    string Tag,
    string Asset,
    string Source,
    bool Verified,
    string? Path,
    string? RootCid,
    string? ReleaseCid,
    string Error)
{
    public static IrisDriveUpdateResult ErrorResult(string error)
    {
        return new IrisDriveUpdateResult(
            false,
            "",
            "",
            "",
            "",
            "",
            false,
            null,
            null,
            null,
            error);
    }

    public static IrisDriveUpdateResult FromJson(string json)
    {
        using var document = JsonDocument.Parse(string.IsNullOrWhiteSpace(json) ? "{}" : json);
        var root = document.RootElement;
        return new IrisDriveUpdateResult(
            Bool(root, "available"),
            String(root, "current_version") ?? "",
            String(root, "latest_version") ?? "",
            String(root, "tag") ?? "",
            String(root, "asset") ?? "",
            String(root, "source") ?? "",
            Bool(root, "verified"),
            String(root, "path"),
            String(root, "root_cid"),
            String(root, "release_cid"),
            String(root, "error") ?? "");
    }

    private static string? String(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.String
                ? value.GetString()
                : null;
    }

    private static bool Bool(JsonElement root, string name)
    {
        return root.ValueKind == JsonValueKind.Object &&
            root.TryGetProperty(name, out var value) &&
            value.ValueKind is JsonValueKind.True or JsonValueKind.False &&
            value.GetBoolean();
    }
}

public sealed record AppKeyLinkRequestRow(
    string DeviceNpub,
    string Label,
    string RequestUrl);

public sealed record PeerRow(
    string DeviceNpub,
    string Title,
    string Role,
    string Subtitle,
    string State,
    bool IsOnline,
    bool IsCurrentDevice,
    bool CanRevoke,
    bool CanAppointAdmin,
    bool CanDemoteAdmin);
