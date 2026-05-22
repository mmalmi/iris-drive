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
    public string? DeviceLinkRequestUrl { get; init; }
    public int RosterSize { get; init; }
    public int AuthorizedDeviceCount { get; init; }
    public int PublishedDeviceRoots { get; init; }
    public string? WorkingDirectory { get; init; }
    public string? ConfigDirectory { get; init; }
    public string? BlocksDirectory { get; init; }
    public string? RootCid { get; init; }
    public string? SnapshotUrl { get; init; }
    public int FileCount { get; init; }
    public int TopLevelEntries { get; init; }
    public int LocalBlockCount { get; init; }
    public long LocalBlockBytes { get; init; }
    public IReadOnlyList<DriveRow> Drives { get; init; } = Array.Empty<DriveRow>();
    public IReadOnlyList<PeerRow> Peers { get; init; } = Array.Empty<PeerRow>();
    public IReadOnlyList<BackupTargetRow> BackupTargets { get; init; } =
        Array.Empty<BackupTargetRow>();
    public IReadOnlyList<string> Relays { get; init; } = Array.Empty<string>();
    public IReadOnlyList<string> BlossomServers { get; init; } = Array.Empty<string>();
    public FipsDiagnostics Fips { get; init; } = FipsDiagnostics.Empty;
    public IReadOnlyDictionary<string, string> RelayStatuses { get; init; } =
        new Dictionary<string, string>();

    public static IrisDriveStatusData FromJson(JsonElement root, string defaultDriveDirectory)
    {
        var account = Object(root, "account");
        var deviceLinkRequest = account.HasValue
            ? Object(account.Value, "device_link_request")
            : null;
        var hashtree = Object(root, "hashtree");
        var network = Object(root, "network");
        var drives = DriveRows(root, defaultDriveDirectory);

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
            DeviceLinkRequestUrl = deviceLinkRequest.HasValue
                ? String(deviceLinkRequest.Value, "url")
                : null,
            RosterSize = account.HasValue ? Int(account.Value, "roster_size") : 0,
            AuthorizedDeviceCount =
                network.HasValue ? Int(network.Value, "authorized_device_count") : 0,
            PublishedDeviceRoots =
                network.HasValue ? Int(network.Value, "published_device_roots") : 0,
            WorkingDirectory = ExtractWorkingDirectory(root, defaultDriveDirectory),
            ConfigDirectory = String(root, "config_dir"),
            BlocksDirectory = hashtree.HasValue ? String(hashtree.Value, "blocks_dir") : null,
            RootCid = hashtree.HasValue ? String(hashtree.Value, "current_root_cid") : null,
            SnapshotUrl = hashtree.HasValue
                ? String(hashtree.Value, "snapshot_url") ?? String(hashtree.Value, "permalink_url")
                : null,
            FileCount = hashtree.HasValue ? Int(hashtree.Value, "file_count") : 0,
            TopLevelEntries = hashtree.HasValue ? Int(hashtree.Value, "top_level_entries") : 0,
            LocalBlockCount = hashtree.HasValue ? Int(hashtree.Value, "local_block_count") : 0,
            LocalBlockBytes = hashtree.HasValue ? Long(hashtree.Value, "local_block_bytes") : 0,
            Drives = drives,
            Peers = PeerRows(
                root,
                account.HasValue && Bool(account.Value, "has_owner_signing_authority")),
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
                ? RelayStatusMap(network.Value)
                : new Dictionary<string, string>(),
        };
    }

    public bool IsAwaitingLinkedApproval =>
        Initialized &&
        !HasOwnerSigningAuthority &&
        string.Equals(AuthorizationState, "awaiting_approval", StringComparison.Ordinal);

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

    private static string ExtractWorkingDirectory(JsonElement root, string fallback)
    {
        if (root.TryGetProperty("drives", out var drives) && drives.ValueKind == JsonValueKind.Array)
        {
            foreach (var drive in drives.EnumerateArray())
            {
                if (String(drive, "drive_id") == "main")
                {
                    return String(drive, "working_dir") ?? fallback;
                }
            }
        }

        return String(root, "default_working_dir") ?? fallback;
    }

    private static IReadOnlyList<DriveRow> DriveRows(JsonElement root, string fallback)
    {
        var rows = new List<DriveRow>();
        if (root.TryGetProperty("drives", out var drives) && drives.ValueKind == JsonValueKind.Array)
        {
            foreach (var drive in drives.EnumerateArray())
            {
                var name = String(drive, "display_name") ?? String(drive, "name") ??
                    String(drive, "drive_id") ?? "main";
                var path = String(drive, "working_dir") ?? String(drive, "local_path") ?? fallback;
                var state = ShortText(String(drive, "last_root_cid") ?? "configured");
                rows.Add(new DriveRow(name, path, state));
            }
        }

        if (rows.Count == 0)
        {
            rows.Add(new DriveRow("main", fallback, "-"));
        }

        return rows;
    }

    private static IReadOnlyList<PeerRow> PeerRows(JsonElement root, bool canManageDevices)
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
            var title = String(peer, "label") ??
                (!string.IsNullOrWhiteSpace(deviceNpub) ? deviceNpub : String(peer, "device_pubkey")) ??
                "Device";
            var details = new List<string>();
            if (isCurrentDevice)
            {
                details.Add("this device");
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
            var state = isOnline ? "Online" : "Offline";
            rows.Add(new PeerRow(
                deviceNpub,
                title,
                string.Join(" | ", details),
                state,
                isOnline,
                isCurrentDevice,
                canManageDevices && !isCurrentDevice && !string.IsNullOrWhiteSpace(deviceNpub)));
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
            var title = String(target, "label");
            if (string.IsNullOrWhiteSpace(title))
            {
                title = String(target, "kind") == "fips" ? ShortText(value) : value;
            }

            var lastSync = Object(target, "last_sync");
            var state = lastSync.HasValue
                ? String(lastSync.Value, "state") ?? "synced"
                : String(target, "kind") == "fips" ? "Pending" : "Ready";
            var detail = String(target, "kind") == "fips"
                ? ShortText(value)
                : value;
            if (lastSync.HasValue)
            {
                var uploaded = Int(lastSync.Value, "uploaded");
                var total = Int(lastSync.Value, "total_hashes");
                detail = $"{detail} | {uploaded}/{total}";
            }

            rows.Add(new BackupTargetRow(
                String(target, "id") ?? value,
                String(target, "kind") ?? "backup",
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

    private static IReadOnlyDictionary<string, string> RelayStatusMap(JsonElement network)
    {
        var map = new Dictionary<string, string>();
        if (!network.TryGetProperty("relay_statuses", out var statuses) ||
            statuses.ValueKind != JsonValueKind.Array)
        {
            return map;
        }

        foreach (var status in statuses.EnumerateArray())
        {
            var url = String(status, "url");
            if (!string.IsNullOrWhiteSpace(url))
            {
                map[url] = String(status, "status") ?? "saved";
            }
        }

        return map;
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
    string? EndpointNpub,
    string? DiscoveryScope,
    int RosterPeerCount,
    int RosterConnectedPeerCount,
    int ConnectedPeerCount,
    int OtherPeerCount,
    string? Error)
{
    public static FipsDiagnostics Empty { get; } =
        new(false, false, false, null, null, 0, 0, 0, 0, null);

    public string State =>
        !string.IsNullOrWhiteSpace(Error) ? "Error" :
        Enabled && Fresh ? "Running" :
        Enabled || Running ? "Stale" :
        "Stopped";

    public string RosterText => $"{RosterConnectedPeerCount}/{RosterPeerCount} direct";

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
            String(value, "endpoint_npub"),
            String(value, "discovery_scope"),
            Int(value, "roster_peer_count"),
            Int(value, "roster_connected_peer_count"),
            Int(value, "connected_peer_count"),
            Int(value, "other_peer_count"),
            String(value, "error"));
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

public sealed record PeerRow(
    string DeviceNpub,
    string Title,
    string Subtitle,
    string State,
    bool IsOnline,
    bool IsCurrentDevice,
    bool CanRevoke);
