using System;
using System.Collections.Generic;
using System.Text.Json;

namespace IrisDrive.WindowsShell;

public sealed class IrisDriveStatusData
{
    public bool Initialized { get; init; }
    public string DriveName { get; init; } = "My Drive";
    public string? OwnerNpub { get; init; }
    public string? DeviceNpub { get; init; }
    public bool HasOwnerSigningAuthority { get; init; }
    public string? AuthorizationState { get; init; }
    public string SetupState { get; init; } = "not_configured";
    public string SetupLabel { get; init; } = "Not linked";
    public bool SetupComplete { get; init; }
    public bool AwaitingApproval { get; init; }
    public bool Revoked { get; init; }
    public string PrimaryStatus { get; init; } = "not_setup";
    public string PrimaryStatusLabel { get; init; } = "Ready";
    public string? DeviceLinkRequestUrl { get; init; }
    public int AuthorizedDeviceCount { get; init; }
    public int OnlineDeviceCount { get; init; }
    public string? ConfigDirectory { get; init; }
    public string? CurrentRootCid { get; init; }
    public string ProviderRefreshKey { get; init; } = "";
    public string? SnapshotUrl { get; init; }
    public int FileCount { get; init; }
    public long VisibleFileBytes { get; init; }
    public bool LocalNhashResolverEnabled { get; init; } = true;
    public IReadOnlyList<DriveRow> Drives { get; init; } = Array.Empty<DriveRow>();
    public IReadOnlyList<PeerRow> Peers { get; init; } = Array.Empty<PeerRow>();
    public IReadOnlyList<BackupTargetRow> BackupTargets { get; init; } =
        Array.Empty<BackupTargetRow>();
    public IReadOnlyList<string> Relays { get; init; } = Array.Empty<string>();
    public IReadOnlyList<string> BlossomServers { get; init; } = Array.Empty<string>();
    public FipsDiagnostics Fips { get; init; } = FipsDiagnostics.Empty;
    public IReadOnlyList<RelayStatusRow> RelayStatuses { get; init; } =
        Array.Empty<RelayStatusRow>();

    public static IrisDriveStatusData FromJson(JsonElement root)
    {
        var account = Object(root, "account");
        var deviceLinkRequest = account.HasValue
            ? Object(account.Value, "device_link_request")
            : null;
        var hashtree = Object(root, "hashtree");
        var network = Object(root, "network");
        var summary = Object(root, "summary");
        var mountPath = ExtractDrivePath(root);
        var drives = DriveRows(root, mountPath);

        return new IrisDriveStatusData
        {
            Initialized = Bool(root, "initialized"),
            DriveName = ExtractDriveName(root),
            OwnerNpub = account.HasValue ? String(account.Value, "owner_npub") : null,
            DeviceNpub = account.HasValue ? String(account.Value, "device_npub") : null,
            HasOwnerSigningAuthority =
                account.HasValue && Bool(account.Value, "has_owner_signing_authority"),
            AuthorizationState =
                account.HasValue ? String(account.Value, "authorization_state") : null,
            SetupState = summary.HasValue ? String(summary.Value, "setup_state") ?? "not_configured" : "not_configured",
            SetupLabel = summary.HasValue ? String(summary.Value, "setup_label") ?? "Not linked" : "Not linked",
            SetupComplete = summary.HasValue && Bool(summary.Value, "setup_complete"),
            AwaitingApproval = summary.HasValue && Bool(summary.Value, "awaiting_approval"),
            Revoked = summary.HasValue && Bool(summary.Value, "revoked"),
            PrimaryStatus = summary.HasValue ? String(summary.Value, "primary_status") ?? "not_setup" : "not_setup",
            PrimaryStatusLabel = summary.HasValue ? String(summary.Value, "primary_status_label") ?? "Ready" : "Ready",
            DeviceLinkRequestUrl = deviceLinkRequest.HasValue
                ? String(deviceLinkRequest.Value, "url")
                : null,
            AuthorizedDeviceCount = summary.HasValue
                ? Int(summary.Value, "authorized_device_count")
                : 0,
            OnlineDeviceCount = summary.HasValue ? Int(summary.Value, "online_device_count") : 0,
            ConfigDirectory = String(root, "config_dir"),
            CurrentRootCid = hashtree.HasValue ? String(hashtree.Value, "current_root_cid") : null,
            ProviderRefreshKey = summary.HasValue
                ? String(summary.Value, "provider_refresh_key") ?? ""
                : "",
            SnapshotUrl = hashtree.HasValue
                ? String(hashtree.Value, "snapshot_url") ?? String(hashtree.Value, "permalink_url")
                : null,
            FileCount = summary.HasValue ? Int(summary.Value, "file_count") : 0,
            VisibleFileBytes = summary.HasValue
                ? Long(summary.Value, "visible_file_bytes")
                : 0,
            LocalNhashResolverEnabled = ExtractLocalNhashResolverEnabled(root),
            Drives = drives,
            Peers = PeerRows(root),
            BackupTargets = network.HasValue
                ? BackupTargetRows(network.Value)
                : Array.Empty<BackupTargetRow>(),
            Relays = network.HasValue ? StringArray(network.Value, "relays") : Array.Empty<string>(),
            BlossomServers = network.HasValue
                ? StringArray(network.Value, "blossom_servers")
                : Array.Empty<string>(),
            Fips = network.HasValue
                ? FipsDiagnostics.FromJson(Object(network.Value, "fips"))
                : FipsDiagnostics.Empty,
            RelayStatuses = network.HasValue
                ? RelayStatusesFromJson(network.Value)
                : Array.Empty<RelayStatusRow>(),
        };
    }

    public bool IsAwaitingLinkedApproval =>
        Initialized && AwaitingApproval;

    public bool IsSetupComplete =>
        Initialized && SetupComplete;

    public bool IsRevoked =>
        Initialized && Revoked;

    private static string ExtractDriveName(JsonElement root)
    {
        if (root.TryGetProperty("drives", out var drives) && drives.ValueKind == JsonValueKind.Array)
        {
            foreach (var drive in drives.EnumerateArray())
            {
                if (String(drive, "drive_id") == "main")
                {
                    return String(drive, "display_name") ?? String(drive, "name") ?? "My Drive";
                }
            }

            foreach (var drive in drives.EnumerateArray())
            {
                return String(drive, "display_name") ?? String(drive, "name") ?? "My Drive";
            }
        }

        return "My Drive";
    }

    private static string? ExtractDrivePath(JsonElement root)
    {
        var summary = Object(root, "summary");
        return summary.HasValue && Bool(summary.Value, "setup_complete")
            ? WindowsCloudFiles.SyncRootPath
            : null;
    }

    private static bool ExtractLocalNhashResolverEnabled(JsonElement root)
    {
        if (Object(root, "settings") is { } settings &&
            settings.TryGetProperty("local_nhash_resolver_enabled", out var enabled) &&
            (enabled.ValueKind == JsonValueKind.True || enabled.ValueKind == JsonValueKind.False))
        {
            return enabled.GetBoolean();
        }

        if (Object(root, "hashtree") is { } hashtree &&
            Object(hashtree, "local_gateway") is { } gateway &&
            gateway.TryGetProperty("enabled", out var gatewayEnabled) &&
            (gatewayEnabled.ValueKind == JsonValueKind.True ||
                gatewayEnabled.ValueKind == JsonValueKind.False))
        {
            return gatewayEnabled.GetBoolean();
        }

        return true;
    }

    private static IReadOnlyList<DriveRow> DriveRows(JsonElement root, string? mountPath)
    {
        var rows = new List<DriveRow>();
        if (root.TryGetProperty("drives", out var drives) && drives.ValueKind == JsonValueKind.Array)
        {
            foreach (var drive in drives.EnumerateArray())
            {
                var name = String(drive, "display_name") ?? String(drive, "name") ??
                    String(drive, "drive_id") ?? "main";
                var path = mountPath ?? "Not ready";
                var state = ShortText(String(drive, "last_root_cid") ?? "configured");
                rows.Add(new DriveRow(name, path, state));
            }
        }

        if (rows.Count == 0)
        {
            rows.Add(new DriveRow("main", mountPath ?? "Not ready", "-"));
        }

        return rows;
    }

    private static IReadOnlyList<PeerRow> PeerRows(JsonElement root)
    {
        var rows = new List<PeerRow>();
        if (!root.TryGetProperty("peers", out var peers) || peers.ValueKind != JsonValueKind.Array)
        {
            return rows;
        }

        foreach (var peer in peers.EnumerateArray())
        {
            var deviceNpub = String(peer, "device_npub") ?? "";
            var isCurrentDevice = Bool(peer, "is_current_device");
            var role = String(peer, "role") ?? "member";
            var roleLabel = String(peer, "role_label") ?? "";
            var title = String(peer, "display_label") ?? "";
            var details = new List<string>();
            if (isCurrentDevice)
            {
                details.Add("this device");
            }

            if (!string.IsNullOrWhiteSpace(roleLabel))
            {
                details.Add(roleLabel);
            }
            var syncState = String(peer, "sync_state");
            if (!string.IsNullOrWhiteSpace(syncState))
            {
                details.Add(syncState);
            }

            if (Object(peer, "last_block_sync") is { } blockSync)
            {
                var transport = String(blockSync, "transport");
                var fetched = Int(blockSync, "fetched");
                var total = Int(blockSync, "total_hashes");
                if (!string.IsNullOrWhiteSpace(transport) && total > 0)
                {
                    details.Add($"{transport} {fetched}/{total}");
                }
            }

            var rootCid = String(peer, "root_cid");
            if (!string.IsNullOrWhiteSpace(rootCid))
            {
                details.Add(ShortText(rootCid));
            }

            var dck = Int(peer, "dck_generation");
            if (dck > 0)
            {
                details.Add($"DCK {dck}");
            }

            var isOnline = Bool(peer, "fips_online");
            var state = String(peer, "connection_label") ?? "";
            var canManagePeer = !string.IsNullOrWhiteSpace(deviceNpub);
            rows.Add(new PeerRow(
                deviceNpub,
                title,
                role,
                string.Join(" | ", details),
                state,
                isOnline,
                isCurrentDevice,
                canManagePeer && Bool(peer, "can_revoke"),
                canManagePeer && Bool(peer, "can_appoint_admin"),
                canManagePeer && Bool(peer, "can_demote_admin")));
        }

        return rows;
    }

    private static IReadOnlyList<BackupTargetRow> BackupTargetRows(JsonElement network)
    {
        var rows = new List<BackupTargetRow>();
        if (!network.TryGetProperty("backup_targets", out var targets) ||
            targets.ValueKind != JsonValueKind.Array)
        {
            return rows;
        }

        foreach (var target in targets.EnumerateArray())
        {
            var value = String(target, "target") ?? "";
            var kind = String(target, "kind") ?? "backup";
            var title = String(target, "title") ?? "Backup";
            var detail = String(target, "detail") ?? value;
            var state = String(target, "state") ?? "";

            rows.Add(new BackupTargetRow(
                String(target, "id") ?? value,
                kind,
                title ?? "Backup",
                detail,
                state));
        }

        return rows;
    }

    private static IReadOnlyList<string> StringArray(JsonElement root, string name)
    {
        if (!root.TryGetProperty(name, out var array) || array.ValueKind != JsonValueKind.Array)
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
        if (!network.TryGetProperty("relay_statuses", out var statuses) ||
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
        return root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.Object
            ? value
            : null;
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

    private static long Long(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) &&
            value.ValueKind == JsonValueKind.Number &&
            value.TryGetInt64(out var result)
            ? result
            : 0;
    }

    private static bool Bool(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) &&
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
    string Title,
    string Subtitle,
    string State);

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
