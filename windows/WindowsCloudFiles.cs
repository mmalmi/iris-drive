using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Threading;

namespace IrisDrive.WindowsShell;

public sealed class DriveFolderPreparation
{
    public DriveFolderPreparation(
        string path,
        bool nativeSyncRootReady,
        string? warning,
        int placeholderCount = 0,
        int skippedLocalItemCount = 0,
        IReadOnlyCollection<string>? refreshedPlaceholderPaths = null,
        IReadOnlyCollection<string>? protectedLocalItemPaths = null)
    {
        Path = path;
        NativeSyncRootReady = nativeSyncRootReady;
        Warning = warning;
        PlaceholderCount = placeholderCount;
        SkippedLocalItemCount = skippedLocalItemCount;
        RefreshedPlaceholderPaths = refreshedPlaceholderPaths ?? Array.Empty<string>();
        ProtectedLocalItemPaths = protectedLocalItemPaths ?? Array.Empty<string>();
    }

    public string Path { get; }
    public bool NativeSyncRootReady { get; }
    public string? Warning { get; }
    public int PlaceholderCount { get; }
    public int SkippedLocalItemCount { get; }
    public IReadOnlyCollection<string> RefreshedPlaceholderPaths { get; }
    public IReadOnlyCollection<string> ProtectedLocalItemPaths { get; }
}

public sealed record WindowsCloudFileEntry(string Path, string Kind, long Size, string? Version)
{
    public bool IsDirectory =>
        string.Equals(Kind, "directory", StringComparison.OrdinalIgnoreCase);

    public static WindowsCloudFileEntry FromJson(JsonElement element)
    {
        var path = element.TryGetProperty("path", out var pathValue) &&
            pathValue.ValueKind == JsonValueKind.String
                ? pathValue.GetString() ?? ""
                : "";
        var kind = element.TryGetProperty("kind", out var kindValue) &&
            kindValue.ValueKind == JsonValueKind.String
                ? kindValue.GetString() ?? "file"
                : "file";
        var size = element.TryGetProperty("size", out var sizeValue) &&
            sizeValue.ValueKind == JsonValueKind.Number &&
            sizeValue.TryGetInt64(out var parsedSize)
                ? parsedSize
                : 0;
        var version = element.TryGetProperty("version", out var versionValue) &&
            versionValue.ValueKind == JsonValueKind.String
                ? versionValue.GetString()
                : null;

        return new WindowsCloudFileEntry(path, kind, size, version);
    }
}

public sealed record WindowsCloudLocalStateEntry(
    string Path,
    string Kind,
    long Size,
    string? Sha256,
    string? ProviderVersion = null)
{
    public bool IsDirectory =>
        string.Equals(Kind, "directory", StringComparison.OrdinalIgnoreCase);

    public static WindowsCloudLocalStateEntry? FromJson(JsonElement element)
    {
        var path = TryGetString(element, "path", "Path") is { } parsedPath
            ? NormalizePath(parsedPath)
            : "";
        if (string.IsNullOrWhiteSpace(path))
        {
            return null;
        }

        var kind = TryGetString(element, "kind", "Kind") ?? "file";
        var size = TryGetInt64(element, "size", "Size") ?? 0;
        var sha256 = TryGetString(element, "sha256", "Sha256");
        var providerVersion = TryGetString(element, "providerVersion", "ProviderVersion");

        return new WindowsCloudLocalStateEntry(path, kind, size, sha256, providerVersion);
    }

    private static string? TryGetString(JsonElement element, string lowerName, string upperName)
    {
        if (element.TryGetProperty(lowerName, out var lowerValue) &&
            lowerValue.ValueKind == JsonValueKind.String)
        {
            return lowerValue.GetString();
        }

        return element.TryGetProperty(upperName, out var upperValue) &&
            upperValue.ValueKind == JsonValueKind.String
                ? upperValue.GetString()
                : null;
    }

    private static long? TryGetInt64(JsonElement element, string lowerName, string upperName)
    {
        if (element.TryGetProperty(lowerName, out var lowerValue) &&
            lowerValue.ValueKind == JsonValueKind.Number &&
            lowerValue.TryGetInt64(out var lowerParsed))
        {
            return lowerParsed;
        }

        return element.TryGetProperty(upperName, out var upperValue) &&
            upperValue.ValueKind == JsonValueKind.Number &&
            upperValue.TryGetInt64(out var upperParsed)
                ? upperParsed
                : null;
    }

    private static string NormalizePath(string path) =>
        path.Replace('\\', '/').Trim('/');
}

public sealed record WindowsCloudLocalUpsert(string Path, string FullPath);

public sealed record WindowsCloudLocalDelete(string Path);

public static partial class WindowsCloudFiles
{
    private const string ProviderName = "Iris Drive";
    private const string ProviderVersion = "0.1";
    private const int CfRegisterFlagUpdate = 0x00000001;
    private const int CfRegisterFlagDisableOnDemandPopulationOnRoot = 0x00000002;
    private const int CfRegisterFlagMarkInSyncOnRoot = 0x00000004;
    private const ushort CfHydrationPolicyFull = 2;
    private const ushort CfPopulationPolicyAlwaysFull = 3;
    private const int CfCallbackTypeFetchData = 0;
    private const int CfCallbackTypeNotifyDelete = 9;
    private const int CfCallbackTypeNotifyRename = 11;
    private const int CfCallbackTypeNone = -1;
    private const int CfConnectFlagRequireFullFilePath = 0x00000004;
    private const int CfCreateFlagStopOnError = 0x00000001;
    private const int CfPlaceholderCreateFlagDisableOnDemandPopulation = 0x00000001;
    private const int CfPlaceholderCreateFlagMarkInSync = 0x00000002;
    private const int CfPlaceholderCreateFlagSupersede = 0x00000004;
    private const int CfOperationTypeTransferData = 0;
    private const int CfOperationTypeAckDelete = 6;
    private const int CfOperationTypeAckRename = 7;
    private const int CfOperationTransferDataFlagNone = 0;
    private const int CfOperationAckDeleteFlagNone = 0;
    private const int CfOperationAckRenameFlagNone = 0;
    private const int StatusSuccess = 0;
    private const int StatusUnsuccessful = unchecked((int)0xC0000001);
    private const uint FileAttributeDirectory = 0x00000010;
    private const uint FileAttributeNormal = 0x00000080;
    private const uint ShcneCreate = 0x00000002;
    private const uint ShcneDelete = 0x00000004;
    private const uint ShcneMkdir = 0x00000008;
    private const uint ShcneRmdir = 0x00000010;
    private const uint ShcneUpdateDir = 0x00001000;
    private const uint ShcneUpdateItem = 0x00002000;
    private const uint ShcnfPathW = 0x0005;
    private const uint ShcnfFlushNowait = 0x2000;
    private const int PendingProviderMutationTtlSeconds = 120;
    private const int PendingProviderCleanupDeleteTtlSeconds = 30;
    private const int StalePlaceholderGraceSeconds = 15;
    private const int DeleteRetryCount = 50;
    private const int DeleteRetryDelayMs = 100;
    private const string LocalStateFileName = "windows-cloud-local-state.json";
    private const string CleanupDeleteFileName = "windows-cloud-cleanup-deletes.json";
    private static readonly Guid ProviderId = new("2b58fb5d-b823-4d84-bd52-fcf9bd297fd4");
    private static readonly object ConnectionLock = new();
    private static readonly object PendingProviderMutationLock = new();
    private static readonly Dictionary<string, DateTimeOffset> PendingProviderDeletes =
        new(StringComparer.Ordinal);
    private static readonly Dictionary<string, DateTimeOffset> PendingProviderPreserves =
        new(StringComparer.Ordinal);
    private static readonly Dictionary<string, DateTimeOffset> PendingProviderCleanupDeletes =
        new(StringComparer.Ordinal);
    private static CloudFilesConnection? activeConnection;

    public static string SyncRootPath =>
        System.IO.Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
            "Iris Drive");

    private static string ConfigDirectoryPath =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "iris-drive");

    public static void DebugLog(string message)
    {
        if (!string.Equals(
                Environment.GetEnvironmentVariable("IRIS_DRIVE_WINDOWS_CLOUD_DEBUG"),
                "1",
                StringComparison.Ordinal))
        {
            return;
        }

        try
        {
            var configDirectory = ConfigDirectoryPath;
            Directory.CreateDirectory(configDirectory);
            File.AppendAllText(
                Path.Combine(configDirectory, "windows-cloud-files.log"),
                $"{DateTimeOffset.Now:O} pid={Environment.ProcessId} {message}{Environment.NewLine}");
        }
        catch
        {
        }
    }

    public static bool SyncRootEntryExists(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        if (string.IsNullOrWhiteSpace(normalized) || PathHasIgnoredComponent(normalized))
        {
            return false;
        }

        var fullPath = Path.Combine(SyncRootPath, FromVirtualPath(normalized));
        return File.Exists(fullPath) || Directory.Exists(fullPath);
    }

    public static IReadOnlyList<WindowsCloudLocalStateEntry> LoadLocalState(string configDirectory)
    {
        try
        {
            var path = Path.Combine(configDirectory, LocalStateFileName);
            if (!File.Exists(path))
            {
                return Array.Empty<WindowsCloudLocalStateEntry>();
            }

            using var document = JsonDocument.Parse(File.ReadAllText(path));
            if (!document.RootElement.TryGetProperty("entries", out var entries) ||
                entries.ValueKind != JsonValueKind.Array)
            {
                return Array.Empty<WindowsCloudLocalStateEntry>();
            }

            return entries
                .EnumerateArray()
                .Select(WindowsCloudLocalStateEntry.FromJson)
                .Where(entry => entry is not null)
                .Cast<WindowsCloudLocalStateEntry>()
                .Where(entry => !PathHasIgnoredComponent(entry.Path))
                .ToArray();
        }
        catch
        {
            return Array.Empty<WindowsCloudLocalStateEntry>();
        }
    }

    public static void WriteLocalState(
        string configDirectory,
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        Func<string, byte[]> readFile,
        IReadOnlyCollection<WindowsCloudLocalStateEntry>? previousState = null,
        IReadOnlyCollection<string>? refreshedPlaceholderPaths = null,
        IReadOnlyCollection<string>? protectedLocalItemPaths = null)
    {
        try
        {
            Directory.CreateDirectory(configDirectory);
            var state = SnapshotLocalState(
                entries,
                readFile,
                previousState ?? Array.Empty<WindowsCloudLocalStateEntry>(),
                refreshedPlaceholderPaths ?? Array.Empty<string>(),
                protectedLocalItemPaths ?? Array.Empty<string>());
            var json = JsonSerializer.Serialize(new { entries = state });
            File.WriteAllText(Path.Combine(configDirectory, LocalStateFileName), json);
        }
        catch
        {
            // This is a safety cache for remote-delete cleanup; sync still works without it.
        }
    }

    public static bool MarkProviderDeletePending(string path)
    {
        return MarkProviderDeletePendingCore(path);
    }

    public static bool TryMarkProviderDeletePending(string path)
    {
        return MarkProviderDeletePendingCore(path);
    }

    private static bool MarkProviderDeletePendingCore(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        if (string.IsNullOrEmpty(normalized) || PathHasIgnoredComponent(normalized))
        {
            return false;
        }

        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            if (PendingProviderPreserves.Keys.Any(existing =>
                    PathContainsOrEquals(existing, normalized) ||
                    PathContainsOrEquals(normalized, existing)) ||
                PendingProviderDeletes.Keys.Any(existing => PathContainsOrEquals(existing, normalized)))
            {
                return false;
            }

            foreach (var existing in PendingProviderDeletes.Keys
                .Where(existing => PathContainsOrEquals(normalized, existing))
                .ToArray())
            {
                PendingProviderDeletes.Remove(existing);
            }

            PendingProviderDeletes[normalized] = DateTimeOffset.UtcNow;
        }

        DebugLog($"provider delete pending path={normalized}");
        return true;
    }

    public static void MarkProviderRenamePending(string oldPath, string newPath)
    {
        var normalizedOldPath = NormalizeVirtualPath(oldPath);
        var normalizedNewPath = NormalizeVirtualPath(newPath);
        if (string.IsNullOrEmpty(normalizedOldPath) ||
            string.IsNullOrEmpty(normalizedNewPath) ||
            PathHasIgnoredComponent(normalizedOldPath) ||
            PathHasIgnoredComponent(normalizedNewPath))
        {
            return;
        }

        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            PendingProviderDeletes[normalizedOldPath] = DateTimeOffset.UtcNow;
            PendingProviderPreserves[normalizedNewPath] = DateTimeOffset.UtcNow;
        }

        DebugLog($"provider rename pending old={normalizedOldPath} new={normalizedNewPath}");
    }

    public static void ClearProviderMutationPending(params string[] paths)
    {
        lock (PendingProviderMutationLock)
        {
            foreach (var path in paths.Select(NormalizeVirtualPath))
            {
                PendingProviderDeletes.Remove(path);
                PendingProviderPreserves.Remove(path);
                PendingProviderCleanupDeletes.Remove(path);
            }

            PersistProviderCleanupDeletesLocked();
        }
    }

    public static bool ProviderMutationIsPending(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        if (string.IsNullOrEmpty(normalized))
        {
            return false;
        }

        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            return PendingProviderDeletes.ContainsKey(normalized) ||
                PendingProviderPreserves.ContainsKey(normalized);
        }
    }

    public static bool ProviderDeleteIsPending(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        if (string.IsNullOrEmpty(normalized))
        {
            return false;
        }

        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            return PendingProviderDeletes.ContainsKey(normalized);
        }
    }

    public static string PromoteProviderDeleteToMissingAncestor(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        if (string.IsNullOrEmpty(normalized) || PathHasIgnoredComponent(normalized))
        {
            return normalized;
        }

        var promoted = ShallowestMissingAncestorPath(normalized);
        if (string.Equals(promoted, normalized, StringComparison.Ordinal))
        {
            return normalized;
        }

        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            if (PendingProviderPreserves.Keys.Any(existing =>
                    PathContainsOrEquals(existing, promoted) ||
                    PathContainsOrEquals(promoted, existing)))
            {
                return normalized;
            }

            foreach (var existing in PendingProviderDeletes.Keys
                .Where(existing =>
                    PathContainsOrEquals(existing, promoted) ||
                    PathContainsOrEquals(promoted, existing))
                .ToArray())
            {
                PendingProviderDeletes.Remove(existing);
            }

            PendingProviderDeletes[promoted] = DateTimeOffset.UtcNow;
        }

        DebugLog($"provider delete promoted original={normalized} ancestor={promoted}");
        return promoted;
    }

    public static void ReconcilePendingProviderMutations(
        IReadOnlyCollection<WindowsCloudFileEntry> entries)
    {
        var entryPaths = new HashSet<string>(
            PlaceholderEntries(entries).Select(entry => entry.Path),
            StringComparer.Ordinal);

        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);

            foreach (var path in PendingProviderDeletes.Keys.ToArray())
            {
                if (!entryPaths.Contains(path))
                {
                    PendingProviderDeletes.Remove(path);
                }
            }

            foreach (var path in PendingProviderPreserves.Keys.ToArray())
            {
                if (entryPaths.Contains(path))
                {
                    PendingProviderPreserves.Remove(path);
                }
            }
        }
    }

    public static IReadOnlyList<WindowsCloudLocalUpsert> RecentLocalFileUpserts(
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState)
    {
        var providerEntries = PlaceholderEntries(entries)
            .ToDictionary(entry => entry.Path, StringComparer.Ordinal);
        var previousByPath = previousState
            .GroupBy(entry => NormalizeVirtualPath(entry.Path), StringComparer.Ordinal)
            .ToDictionary(group => group.Key, group => group.Last(), StringComparer.Ordinal);
        var pendingDeletes = PendingProviderDeletePaths();
        var pendingPreserves = PendingProviderPreservePaths();
        var upserts = new List<WindowsCloudLocalUpsert>();
        if (!Directory.Exists(SyncRootPath))
        {
            return upserts;
        }

        IEnumerable<string> topLevelEntries;
        try
        {
            topLevelEntries = Directory.EnumerateFileSystemEntries(SyncRootPath).ToArray();
        }
        catch
        {
            return upserts;
        }

        foreach (var topLevel in topLevelEntries)
        {
            CollectRecentLocalFileUpserts(
                topLevel,
                providerEntries,
                previousByPath,
                pendingDeletes,
                pendingPreserves,
                upserts);
        }

        return upserts
            .GroupBy(upsert => upsert.Path, StringComparer.Ordinal)
            .Select(group => group.Last())
            .OrderBy(upsert => upsert.Path, StringComparer.Ordinal)
            .ToArray();
    }

    public static IReadOnlyList<WindowsCloudLocalDelete> RecentLocalFileDeletes(
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState)
    {
        if (!Directory.Exists(SyncRootPath))
        {
            return Array.Empty<WindowsCloudLocalDelete>();
        }

        var providerEntries = PlaceholderEntries(entries)
            .ToDictionary(entry => entry.Path, StringComparer.Ordinal);
        var pendingDeletes = PendingProviderDeletePaths();
        var pendingPreserves = PendingProviderPreservePaths();
        var deletes = new List<WindowsCloudLocalDelete>();

        foreach (var previous in previousState)
        {
            var path = NormalizeVirtualPath(previous.Path);
            if (string.IsNullOrEmpty(path) ||
                PathHasIgnoredComponent(path) ||
                PathCoveredByPendingProviderDelete(path, pendingDeletes) ||
                pendingPreserves.Contains(path) ||
                !providerEntries.TryGetValue(path, out var providerEntry) ||
                previous.IsDirectory != providerEntry.IsDirectory)
            {
                continue;
            }

            var fullPath = Path.Combine(SyncRootPath, FromVirtualPath(path));
            var parentPath = Path.GetDirectoryName(fullPath);
            if (string.IsNullOrWhiteSpace(parentPath) ||
                !Directory.Exists(parentPath))
            {
                continue;
            }

            try
            {
                var exists = previous.IsDirectory
                    ? Directory.Exists(fullPath)
                    : File.Exists(fullPath);
                if (!exists)
                {
                    deletes.Add(new WindowsCloudLocalDelete(path));
                }
            }
            catch
            {
            }
        }

        return deletes
            .GroupBy(delete => delete.Path, StringComparer.Ordinal)
            .Select(group => group.Last())
            .OrderBy(delete => delete.Path.Count(ch => ch == '/'))
            .ThenBy(delete => delete.Path, StringComparer.Ordinal)
            .ToArray();
    }

    public static void RemoveStaleSyncedLocalItems(
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState,
        IReadOnlyCollection<string>? protectedLocalItemPaths = null)
    {
        if (previousState.Count == 0)
        {
            return;
        }

        var expectedPaths = new HashSet<string>(
            PlaceholderEntries(entries).Select(entry => entry.Path),
            StringComparer.Ordinal);
        var protectedPaths = NormalizeVirtualPaths(protectedLocalItemPaths ?? Array.Empty<string>());
        var removedAny = false;

        foreach (var previous in previousState
            .OrderByDescending(entry => entry.Path.Count(ch => ch == '/')))
        {
            var path = NormalizeVirtualPath(previous.Path);
            if (string.IsNullOrEmpty(path) ||
                PathHasIgnoredComponent(path) ||
                PathCoveredByProtectedLocalItem(path, protectedPaths) ||
                expectedPaths.Contains(path))
            {
                continue;
            }

            var fullPath = Path.Combine(SyncRootPath, FromVirtualPath(path));
            try
            {
                if (previous.IsDirectory)
                {
                    if (Directory.Exists(fullPath) && !ExistingPlaceholder(fullPath))
                    {
                        removedAny |= TryProviderCleanupDelete(
                            path,
                            () => TryDeleteDirectory(fullPath, recursive: false));
                    }

                    continue;
                }

                if (!File.Exists(fullPath) ||
                    ExistingPlaceholder(fullPath) ||
                    string.IsNullOrWhiteSpace(previous.Sha256))
                {
                    continue;
                }

                var snapshot = SnapshotLocalFile(fullPath);
                if (snapshot is null ||
                    snapshot.Value.Size != previous.Size ||
                    !string.Equals(snapshot.Value.Sha256, previous.Sha256, StringComparison.Ordinal))
                {
                    continue;
                }

                ClearReadOnlyAttribute(fullPath);
                removedAny |= TryProviderCleanupDelete(path, () => TryDeleteFile(fullPath));
            }
            catch
            {
                // Local edits, Explorer handles, or non-empty directories are left for a later pass.
            }
        }

        if (removedAny)
        {
            NotifyShellDirectoryChanged(SyncRootPath);
        }
    }

    public static void NotifyShellEntriesChanged(
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState)
    {
        var currentByPath = PlaceholderEntries(entries)
            .ToDictionary(entry => entry.Path, StringComparer.Ordinal);
        var previousByPath = previousState
            .Where(entry => !PathHasIgnoredComponent(entry.Path))
            .GroupBy(entry => NormalizeVirtualPath(entry.Path), StringComparer.Ordinal)
            .ToDictionary(group => group.Key, group => group.Last(), StringComparer.Ordinal);
        var parentDirectories = new HashSet<string>(StringComparer.Ordinal);

        foreach (var path in currentByPath.Keys.Union(previousByPath.Keys, StringComparer.Ordinal))
        {
            var current = currentByPath.GetValueOrDefault(path);
            var previous = previousByPath.GetValueOrDefault(path);
            var fullPath = Path.Combine(SyncRootPath, FromVirtualPath(path));

            if (previous is null && current is not null)
            {
                NotifyShellPathChanged(current.IsDirectory ? ShcneMkdir : ShcneCreate, fullPath);
                parentDirectories.Add(ParentPath(path));
                continue;
            }

            if (previous is not null && current is null)
            {
                NotifyShellPathChanged(previous.IsDirectory ? ShcneRmdir : ShcneDelete, fullPath);
                parentDirectories.Add(ParentPath(path));
                continue;
            }

            if (previous is not null &&
                current is not null &&
                (!string.Equals(previous.Kind, current.Kind, StringComparison.OrdinalIgnoreCase) ||
                 previous.Size != current.Size))
            {
                NotifyShellPathChanged(ShcneUpdateItem, fullPath);
                parentDirectories.Add(ParentPath(path));
            }
        }

        parentDirectories.Add("");
        foreach (var directory in parentDirectories)
        {
            var fullPath = string.IsNullOrEmpty(directory)
                ? SyncRootPath
                : Path.Combine(SyncRootPath, FromVirtualPath(directory));
            NotifyShellDirectoryChanged(fullPath);
        }
    }

    public static DriveFolderPreparation EnsureSyncRoot(
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        Func<string, byte[]> readFile,
        Action<string>? deletePath = null,
        Action<string, string>? renamePath = null,
        IReadOnlyCollection<WindowsCloudLocalStateEntry>? previousState = null)
    {
        var path = SyncRootPath;
        Directory.CreateDirectory(path);

        try
        {
            RegisterSyncRoot(path);
            RemoveChangedSyncedLocalItems(
                path,
                entries,
                previousState ?? Array.Empty<WindowsCloudLocalStateEntry>());
            var population = PopulatePlaceholders(
                path,
                entries,
                readFile,
                previousState ?? Array.Empty<WindowsCloudLocalStateEntry>());
            NotifyShellDirectoryChanged(path);

            lock (ConnectionLock)
            {
                if (activeConnection is null)
                {
                    activeConnection = CloudFilesConnection.Connect(path, readFile, deletePath, renamePath);
                    DebugLog("cloud files provider connected");
                }
            }

            var warning = population.SkippedLocalItemCount == 0
                ? null
                : $"{population.SkippedLocalItemCount} existing local item(s) were left in place.";
            return new DriveFolderPreparation(
                path,
                nativeSyncRootReady: true,
                warning,
                population.PlaceholderCount,
                population.SkippedLocalItemCount,
                population.RefreshedPaths,
                population.ProtectedLocalItemPaths);
        }
        catch (DllNotFoundException error)
        {
            return NativeProviderUnavailable(path, $"Cloud Files API unavailable: {error.Message}");
        }
        catch (EntryPointNotFoundException error)
        {
            return NativeProviderUnavailable(path, $"Cloud Files API unavailable: {error.Message}");
        }
        catch (Win32Exception error)
        {
            return NativeProviderUnavailable(path, $"Cloud Files operation failed: {error.Message}");
        }
        catch (COMException error)
        {
            return NativeProviderUnavailable(path, $"Cloud Files operation failed: {error.Message}");
        }
    }

    private static DriveFolderPreparation NativeProviderUnavailable(string path, string warning) =>
        new(path, nativeSyncRootReady: false, warning);

    private static IReadOnlyList<WindowsCloudLocalStateEntry> SnapshotLocalState(
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        Func<string, byte[]> readFile,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState,
        IReadOnlyCollection<string> refreshedPlaceholderPaths,
        IReadOnlyCollection<string> protectedLocalItemPaths)
    {
        var state = new List<WindowsCloudLocalStateEntry>();
        var currentPaths = new HashSet<string>(StringComparer.Ordinal);
        var placeholderEntries = PlaceholderEntries(entries).ToArray();
        var previousByPath = previousState
            .GroupBy(entry => NormalizeVirtualPath(entry.Path), StringComparer.Ordinal)
            .ToDictionary(group => group.Key, group => group.Last(), StringComparer.Ordinal);
        var refreshedPaths = new HashSet<string>(
            refreshedPlaceholderPaths.Select(NormalizeVirtualPath),
            StringComparer.Ordinal);
        var protectedPaths = NormalizeVirtualPaths(protectedLocalItemPaths);
        foreach (var entry in placeholderEntries)
        {
            if (PathCoveredByProtectedLocalItem(entry.Path, protectedPaths))
            {
                continue;
            }

            currentPaths.Add(entry.Path);
            var fullPath = Path.Combine(SyncRootPath, FromVirtualPath(entry.Path));
            previousByPath.TryGetValue(entry.Path, out var previous);
            try
            {
                if (Directory.Exists(fullPath))
                {
                    state.Add(new WindowsCloudLocalStateEntry(
                        entry.Path,
                        "directory",
                        0,
                        null,
                        entry.Version));
                    continue;
                }

                if (!File.Exists(fullPath))
                {
                    continue;
                }

                if (ExistingPlaceholder(fullPath))
                {
                    state.Add(new WindowsCloudLocalStateEntry(
                        entry.Path,
                        "file",
                        entry.Size,
                        null,
                        SnapshotProviderVersion(entry, previous, refreshedPaths, true, null)));
                    continue;
                }

                var snapshot = SnapshotLocalFile(fullPath);
                if (snapshot is not null)
                {
                    state.Add(new WindowsCloudLocalStateEntry(
                        entry.Path,
                        "file",
                        snapshot.Value.Size,
                        snapshot.Value.Sha256,
                        SnapshotProviderVersion(entry, previous, refreshedPaths, false, snapshot)));
                }
            }
            catch
            {
                // A transiently locked file should not block the whole provider refresh.
            }
        }

        foreach (var previous in RetainedStaleLocalState(previousState, currentPaths, protectedPaths))
        {
            if (!currentPaths.Add(previous.Path))
            {
                continue;
            }
            state.Add(previous);
        }

        return state
            .OrderBy(entry => entry.Path, StringComparer.Ordinal)
            .ToArray();
    }

    private static IEnumerable<WindowsCloudLocalStateEntry> RetainedStaleLocalState(
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState,
        IReadOnlySet<string> currentPaths,
        IReadOnlySet<string> protectedLocalItemPaths)
    {
        foreach (var previous in previousState)
        {
            var path = NormalizeVirtualPath(previous.Path);
            if (string.IsNullOrEmpty(path) ||
                PathHasIgnoredComponent(path) ||
                PathCoveredByProtectedLocalItem(path, protectedLocalItemPaths) ||
                currentPaths.Contains(path))
            {
                continue;
            }

            var fullPath = Path.Combine(SyncRootPath, FromVirtualPath(path));
            var retain = false;
            try
            {
                if (previous.IsDirectory)
                {
                    if (Directory.Exists(fullPath) && !ExistingPlaceholder(fullPath))
                    {
                        retain = true;
                    }
                }
                else if (File.Exists(fullPath) &&
                    !ExistingPlaceholder(fullPath) &&
                    !string.IsNullOrWhiteSpace(previous.Sha256))
                {
                    var snapshot = SnapshotLocalFile(fullPath);
                    retain = snapshot is not null &&
                        snapshot.Value.Size == previous.Size &&
                        string.Equals(
                            snapshot.Value.Sha256,
                            previous.Sha256,
                            StringComparison.Ordinal);
                }
            }
            catch
            {
                // Keep cache writes best-effort; the next refresh will rebuild from disk again.
            }

            if (retain)
            {
                yield return previous with { Path = path };
            }
        }
    }

    private static string? SnapshotProviderVersion(
        WindowsCloudFileEntry entry,
        WindowsCloudLocalStateEntry? previous,
        IReadOnlySet<string> refreshedPlaceholderPaths,
        bool isPlaceholder,
        LocalFileSnapshot? localSnapshot)
    {
        if (entry.IsDirectory ||
            previous is null ||
            previous.IsDirectory ||
            string.IsNullOrWhiteSpace(previous.ProviderVersion) ||
            string.Equals(previous.ProviderVersion, entry.Version, StringComparison.Ordinal) ||
            refreshedPlaceholderPaths.Contains(entry.Path))
        {
            return entry.Version;
        }

        if (isPlaceholder)
        {
            DebugLogPath(entry.Path, "preserve previous provider version until placeholder refresh succeeds");
            return previous.ProviderVersion;
        }

        if (localSnapshot is { } snapshot &&
            !string.IsNullOrWhiteSpace(previous.Sha256) &&
            previous.Size == snapshot.Size &&
            string.Equals(previous.Sha256, snapshot.Sha256, StringComparison.Ordinal))
        {
            DebugLogPath(entry.Path, "preserve previous provider version until local refresh succeeds");
            return previous.ProviderVersion;
        }

        return entry.Version;
    }

    private static bool ProviderFileChangedSincePreviousState(
        WindowsCloudFileEntry entry,
        IReadOnlyDictionary<string, WindowsCloudLocalStateEntry> previousByPath)
    {
        if (!previousByPath.TryGetValue(entry.Path, out var previous) ||
            previous.IsDirectory ||
            previous.Size != entry.Size)
        {
            return true;
        }

        if (!string.IsNullOrWhiteSpace(entry.Version))
        {
            return !string.Equals(
                entry.Version,
                previous.ProviderVersion,
                StringComparison.Ordinal);
        }

        if (string.IsNullOrWhiteSpace(previous.Sha256))
        {
            return false;
        }

        return false;
    }

    private static string? TryProviderFileSha256(string path, Func<string, byte[]> readFile)
    {
        try
        {
            return Convert.ToHexString(SHA256.HashData(readFile(path))).ToLowerInvariant();
        }
        catch
        {
            return null;
        }
    }

    private static bool LocalPlaceholderContentDiffersFromProvider(
        string fullPath,
        WindowsCloudFileEntry entry,
        Func<string, byte[]> readFile)
    {
        var providerSha256 = TryProviderContentSha256FromVersion(entry);
        if (providerSha256 is not null)
        {
            try
            {
                var attributes = File.GetAttributes(fullPath);
                if ((attributes & FileAttributes.Offline) != 0)
                {
                    return false;
                }
            }
            catch
            {
                return false;
            }
        }
        else if (!string.IsNullOrWhiteSpace(entry.Version))
        {
            return false;
        }
        else
        {
            providerSha256 = TryProviderFileSha256(entry.Path, readFile);
            if (providerSha256 is null)
            {
                return false;
            }
        }

        try
        {
            var snapshot = SnapshotLocalFile(fullPath);
            return snapshot is not null &&
                (snapshot.Value.Size != entry.Size ||
                    !string.Equals(
                        snapshot.Value.Sha256,
                        providerSha256,
                        StringComparison.Ordinal));
        }
        catch
        {
            return false;
        }
    }

    private static string? TryProviderContentSha256FromVersion(WindowsCloudFileEntry entry)
    {
        if (entry.IsDirectory || string.IsNullOrWhiteSpace(entry.Version))
        {
            return null;
        }

        var separator = entry.Version.LastIndexOf(':');
        var candidate = separator >= 0
            ? entry.Version[(separator + 1)..]
            : entry.Version;
        candidate = candidate.Trim();
        if (candidate.Length != 64 || candidate.Any(ch => !Uri.IsHexDigit(ch)))
        {
            return null;
        }

        return candidate.ToLowerInvariant();
    }

    private static LocalFileSnapshot? SnapshotLocalFile(string fullPath)
    {
        using var stream = File.Open(fullPath, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
        var hash = SHA256.HashData(stream);
        return new LocalFileSnapshot(
            stream.Length,
            Convert.ToHexString(hash).ToLowerInvariant());
    }

    private static PlaceholderPopulationReport PopulatePlaceholders(
        string syncRootPath,
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        Func<string, byte[]> readFile,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState)
    {
        var placeholderCount = 0;
        var skippedLocalItems = 0;
        var failedPlaceholderCount = 0;
        var protectedLocalItemPaths = new HashSet<string>(StringComparer.Ordinal);
        var placeholderEntries = PlaceholderEntries(entries).ToArray();
        var pendingDeletes = PendingProviderDeletePaths();
        var pendingPreserves = PendingProviderPreservePaths();
        var refreshedPaths = new HashSet<string>(StringComparer.Ordinal);
        var locallyMissingProjected = MissingProjectedLocalPaths(
            syncRootPath,
            placeholderEntries,
            previousState);
        var previousByPath = previousState
            .GroupBy(entry => NormalizeVirtualPath(entry.Path), StringComparer.Ordinal)
            .ToDictionary(group => group.Key, group => group.Last(), StringComparer.Ordinal);
        var expectedPaths = new HashSet<string>(
            placeholderEntries.Select(entry => entry.Path)
                .Concat(pendingDeletes)
                .Concat(pendingPreserves)
                .Where(path => !locallyMissingProjected.Contains(path)),
            StringComparer.Ordinal);

        RemoveIgnoredLocalItems(syncRootPath);
        RemoveStalePlaceholders(syncRootPath, expectedPaths);

        foreach (var entry in placeholderEntries)
        {
            if (PathCoveredByPendingProviderDelete(entry.Path, pendingDeletes))
            {
                DebugLogPath(entry.Path, "skip pending provider mutation");
                continue;
            }

            if (locallyMissingProjected.Contains(entry.Path))
            {
                skippedLocalItems++;
                DebugLogPath(entry.Path, "skip previously projected local file missing from disk");
                continue;
            }

            var parentPath = ParentPath(entry.Path);
            var parentFullPath = string.IsNullOrEmpty(parentPath)
                ? syncRootPath
                : Path.Combine(syncRootPath, FromVirtualPath(parentPath));
            DebugLogPath(entry.Path, $"consider parent={parentFullPath} parent_exists={Directory.Exists(parentFullPath)}");
            if (!Directory.Exists(parentFullPath))
            {
                skippedLocalItems++;
                DebugLogPath(entry.Path, "skip parent missing");
                continue;
            }

            var itemFullPath = Path.Combine(parentFullPath, FileName(entry.Path));
            if (ExistingPlaceholder(itemFullPath))
            {
                if (entry.IsDirectory)
                {
                    DebugLogPath(entry.Path, $"skip existing directory placeholder {itemFullPath}");
                    continue;
                }

                if (!ProviderFileChangedSincePreviousState(entry, previousByPath) &&
                    !LocalPlaceholderContentDiffersFromProvider(itemFullPath, entry, readFile))
                {
                    DebugLogPath(entry.Path, $"skip unchanged file placeholder {itemFullPath}");
                    continue;
                }

                try
                {
                    ClearReadOnlyAttribute(itemFullPath);
                    if (!TryProviderCleanupDelete(entry.Path, () => TryDeleteFile(itemFullPath)))
                    {
                        failedPlaceholderCount++;
                        DebugLogPath(entry.Path, $"failed to delete existing file placeholder {itemFullPath}");
                        continue;
                    }

                    CreatePlaceholder(parentFullPath, FileName(entry.Path), entry);
                    refreshedPaths.Add(entry.Path);
                    DebugLogPath(entry.Path, $"recreated existing file placeholder {itemFullPath}");
                    placeholderCount++;
                }
                catch (COMException error)
                {
                    failedPlaceholderCount++;
                    DebugLog($"skip unsupported placeholder refresh path={entry.Path} hresult={FormatHResult(error.HResult)} message={error.Message}");
                }
                catch (ArgumentException error)
                {
                    failedPlaceholderCount++;
                    DebugLog($"skip invalid placeholder refresh path={entry.Path} message={error.Message}");
                }
                catch (NotSupportedException error)
                {
                    failedPlaceholderCount++;
                    DebugLog($"skip unsupported placeholder refresh path={entry.Path} message={error.Message}");
                }
                catch (PathTooLongException error)
                {
                    failedPlaceholderCount++;
                    DebugLog($"skip too-long placeholder refresh path={entry.Path} message={error.Message}");
                }

                continue;
            }

            if (ExistingLocalItem(itemFullPath))
            {
                skippedLocalItems++;
                if (!entry.IsDirectory)
                {
                    protectedLocalItemPaths.Add(entry.Path);
                }
                DebugLogPath(entry.Path, $"skip existing local item {itemFullPath}");
                continue;
            }

            try
            {
                CreatePlaceholder(parentFullPath, FileName(entry.Path), entry);
                refreshedPaths.Add(entry.Path);
                DebugLogPath(entry.Path, $"created exists={File.Exists(itemFullPath) || Directory.Exists(itemFullPath)} attrs={SafeAttributes(itemFullPath)}");
                placeholderCount++;
            }
            catch (COMException error)
            {
                failedPlaceholderCount++;
                DebugLog($"skip unsupported placeholder path={entry.Path} hresult={FormatHResult(error.HResult)} message={error.Message}");
            }
            catch (ArgumentException error)
            {
                failedPlaceholderCount++;
                DebugLog($"skip invalid placeholder path={entry.Path} message={error.Message}");
            }
            catch (NotSupportedException error)
            {
                failedPlaceholderCount++;
                DebugLog($"skip unsupported placeholder path={entry.Path} message={error.Message}");
            }
            catch (PathTooLongException error)
            {
                failedPlaceholderCount++;
                DebugLog($"skip too-long placeholder path={entry.Path} message={error.Message}");
            }
        }

        return new PlaceholderPopulationReport(
            placeholderCount,
            skippedLocalItems,
            failedPlaceholderCount,
            refreshedPaths.ToArray(),
            protectedLocalItemPaths.ToArray());
    }

    private static HashSet<string> MissingProjectedLocalPaths(
        string syncRootPath,
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState)
    {
        var providerEntries = entries
            .ToDictionary(entry => entry.Path, StringComparer.Ordinal);
        var missing = new HashSet<string>(StringComparer.Ordinal);

        foreach (var previous in previousState)
        {
            var path = NormalizeVirtualPath(previous.Path);
            if (string.IsNullOrEmpty(path) ||
                !providerEntries.TryGetValue(path, out var providerEntry) ||
                previous.IsDirectory != providerEntry.IsDirectory)
            {
                continue;
            }

            var fullPath = Path.Combine(syncRootPath, FromVirtualPath(path));
            try
            {
                if (previous.IsDirectory)
                {
                    if (!Directory.Exists(fullPath))
                    {
                        missing.Add(path);
                    }
                    continue;
                }

                if (string.IsNullOrWhiteSpace(previous.Sha256))
                {
                    continue;
                }

                // A dehydrated Cloud Files placeholder is still the correct visible
                // projection for a provider file. Only treat the path as locally
                // deleted when the item is actually absent from the sync root.
                if (!File.Exists(fullPath) && !Directory.Exists(fullPath))
                {
                    missing.Add(path);
                }
            }
            catch
            {
            }
        }

        return missing;
    }

    private static void NotifyShellDirectoryChanged(string path)
    {
        NotifyShellPathChanged(ShcneUpdateDir, path);
    }

    private static void RemoveChangedSyncedLocalItems(
        string syncRootPath,
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState)
    {
        if (previousState.Count == 0)
        {
            return;
        }

        var providerFiles = PlaceholderEntries(entries)
            .Where(entry => !entry.IsDirectory)
            .ToDictionary(entry => entry.Path, StringComparer.Ordinal);
        var removedAny = false;

        foreach (var previous in previousState)
        {
            var path = NormalizeVirtualPath(previous.Path);
            if (string.IsNullOrEmpty(path) ||
                previous.IsDirectory ||
                string.IsNullOrWhiteSpace(previous.Sha256) ||
                PathHasIgnoredComponent(path) ||
                !providerFiles.TryGetValue(path, out var providerEntry))
            {
                continue;
            }

            var fullPath = Path.Combine(syncRootPath, FromVirtualPath(path));
            try
            {
                if (!File.Exists(fullPath) || ExistingPlaceholder(fullPath))
                {
                    continue;
                }

                var snapshot = SnapshotLocalFile(fullPath);
                if (snapshot is null ||
                    snapshot.Value.Size != previous.Size ||
                    !string.Equals(snapshot.Value.Sha256, previous.Sha256, StringComparison.Ordinal))
                {
                    continue;
                }

                var providerChanged = providerEntry.Size != snapshot.Value.Size;
                if (!providerChanged && !string.IsNullOrWhiteSpace(providerEntry.Version))
                {
                    providerChanged = !string.Equals(
                        providerEntry.Version,
                        previous.ProviderVersion,
                        StringComparison.Ordinal);
                }

                if (!providerChanged)
                {
                    continue;
                }

                ClearReadOnlyAttribute(fullPath);
                removedAny |= TryProviderCleanupDelete(path, () => TryDeleteFile(fullPath));
            }
            catch
            {
                // Preserve files we cannot prove are unchanged synced Cloud Files placeholders.
            }
        }

        if (removedAny)
        {
            NotifyShellDirectoryChanged(syncRootPath);
        }
    }

    private static void NotifyShellPathChanged(uint eventId, string path)
    {
        try
        {
            NativeMethods.SHChangeNotify(
                eventId,
                ShcnfPathW | ShcnfFlushNowait,
                path,
                null);
        }
        catch
        {
            // Explorer can keep an open sync-root view; missing this nudge is non-fatal.
        }
    }

    private static HashSet<string> PendingProviderDeletePaths()
    {
        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            return new HashSet<string>(PendingProviderDeletes.Keys, StringComparer.Ordinal);
        }
    }

    private static HashSet<string> PendingProviderPreservePaths()
    {
        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            return new HashSet<string>(PendingProviderPreserves.Keys, StringComparer.Ordinal);
        }
    }

    private static bool PathCoveredByPendingProviderDelete(
        string path,
        IEnumerable<string> pendingDeletes)
    {
        return pendingDeletes.Any(pending => PathContainsOrEquals(pending, path));
    }

    private static HashSet<string> NormalizeVirtualPaths(IEnumerable<string> paths) =>
        paths
            .Select(NormalizeVirtualPath)
            .Where(path => !string.IsNullOrEmpty(path) && !PathHasIgnoredComponent(path))
            .ToHashSet(StringComparer.Ordinal);

    private static bool PathCoveredByProtectedLocalItem(
        string path,
        IEnumerable<string> protectedLocalItemPaths)
    {
        return protectedLocalItemPaths.Any(protectedPath =>
            PathContainsOrEquals(protectedPath, path) ||
            PathContainsOrEquals(path, protectedPath));
    }

    private static void CollectRecentLocalFileUpserts(
        string startPath,
        IReadOnlyDictionary<string, WindowsCloudFileEntry> providerEntries,
        IReadOnlyDictionary<string, WindowsCloudLocalStateEntry> previousByPath,
        IReadOnlySet<string> pendingDeletes,
        IReadOnlySet<string> pendingPreserves,
        ICollection<WindowsCloudLocalUpsert> upserts)
    {
        var stack = new Stack<string>();
        stack.Push(startPath);

        while (stack.Count > 0)
        {
            var fullPath = stack.Pop();
            var relative = NormalizeVirtualPath(Path.GetRelativePath(SyncRootPath, fullPath));
            if (string.IsNullOrEmpty(relative) || PathHasIgnoredComponent(relative))
            {
                continue;
            }

            if (PathCoveredByPendingProviderDelete(relative, pendingDeletes) ||
                pendingPreserves.Contains(relative))
            {
                continue;
            }

            FileAttributes attributes;
            try
            {
                attributes = File.GetAttributes(fullPath);
            }
            catch
            {
                continue;
            }

            var isDirectory = (attributes & FileAttributes.Directory) != 0;
            var isReparsePoint = (attributes & FileAttributes.ReparsePoint) != 0;
            if (isReparsePoint && !isDirectory)
            {
                continue;
            }

            if (isDirectory)
            {
                try
                {
                    foreach (var child in Directory.EnumerateFileSystemEntries(fullPath))
                    {
                        stack.Push(child);
                    }
                }
                catch
                {
                }

                continue;
            }

            if (LocalFileAlreadyRepresented(relative, fullPath, providerEntries, previousByPath))
            {
                continue;
            }

            upserts.Add(new WindowsCloudLocalUpsert(relative, fullPath));
        }
    }

    private static bool LocalFileAlreadyRepresented(
        string path,
        string fullPath,
        IReadOnlyDictionary<string, WindowsCloudFileEntry> providerEntries,
        IReadOnlyDictionary<string, WindowsCloudLocalStateEntry> previousByPath)
    {
        if (!providerEntries.TryGetValue(path, out var providerEntry) || providerEntry.IsDirectory)
        {
            if (previousByPath.TryGetValue(path, out var previousMissingProvider) &&
                !previousMissingProvider.IsDirectory &&
                !string.IsNullOrWhiteSpace(previousMissingProvider.Sha256))
            {
                LocalFileSnapshot? previousSnapshot;
                try
                {
                    previousSnapshot = SnapshotLocalFile(fullPath);
                }
                catch
                {
                    return true;
                }

                return previousSnapshot is not null &&
                    previousSnapshot.Value.Size == previousMissingProvider.Size &&
                    string.Equals(
                        previousSnapshot.Value.Sha256,
                        previousMissingProvider.Sha256,
                        StringComparison.Ordinal);
            }

            return false;
        }

        LocalFileSnapshot? snapshot;
        try
        {
            snapshot = SnapshotLocalFile(fullPath);
        }
        catch
        {
            return true;
        }

        if (snapshot is null)
        {
            return true;
        }

        if (providerEntry.Size != snapshot.Value.Size)
        {
            return false;
        }

        return previousByPath.TryGetValue(path, out var previous) &&
            previous.Size == snapshot.Value.Size &&
            string.Equals(previous.Sha256, snapshot.Value.Sha256, StringComparison.Ordinal);
    }

    private static void PrunePendingProviderMutations(DateTimeOffset now)
    {
        var minCreatedAt = now.AddSeconds(-PendingProviderMutationTtlSeconds);
        var minCleanupCreatedAt = now.AddSeconds(-PendingProviderCleanupDeleteTtlSeconds);
        foreach (var path in PendingProviderDeletes
            .Where(entry => entry.Value < minCreatedAt)
            .Select(entry => entry.Key)
            .ToArray())
        {
            PendingProviderDeletes.Remove(path);
        }

        foreach (var path in PendingProviderPreserves
            .Where(entry => entry.Value < minCreatedAt)
            .Select(entry => entry.Key)
            .ToArray())
        {
            PendingProviderPreserves.Remove(path);
        }

        foreach (var path in PendingProviderCleanupDeletes
            .Where(entry => entry.Value < minCleanupCreatedAt)
            .Select(entry => entry.Key)
            .ToArray())
        {
            PendingProviderCleanupDeletes.Remove(path);
        }
    }

    private static void MarkProviderCleanupDelete(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        if (string.IsNullOrEmpty(normalized) || PathHasIgnoredComponent(normalized))
        {
            return;
        }

        lock (PendingProviderMutationLock)
        {
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            PendingProviderCleanupDeletes[normalized] = DateTimeOffset.UtcNow;
            PersistProviderCleanupDeletesLocked();
        }

        DebugLogPath(normalized, "provider cleanup delete pending");
    }

    private static bool TryProviderCleanupDelete(string path, Func<bool> delete)
    {
        MarkProviderCleanupDelete(path);
        var removed = delete();
        if (!removed)
        {
            ClearProviderCleanupDelete(path);
        }

        return removed;
    }

    private static void ClearProviderCleanupDelete(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        if (string.IsNullOrEmpty(normalized))
        {
            return;
        }

        lock (PendingProviderMutationLock)
        {
            PendingProviderCleanupDeletes.Remove(normalized);
            PersistProviderCleanupDeletesLocked();
        }
    }

    private static bool TryConsumeProviderCleanupDelete(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        if (string.IsNullOrEmpty(normalized))
        {
            return false;
        }

        string[] matches;
        HashSet<string> loadedFromDisk;
        lock (PendingProviderMutationLock)
        {
            loadedFromDisk = LoadProviderCleanupDeletesLocked();
            PrunePendingProviderMutations(DateTimeOffset.UtcNow);
            // Cloud Files can coalesce a child cleanup delete into a parent
            // notification, but a parent marker must not hide later user deletes.
            matches = PendingProviderCleanupDeletes.Keys
                .Where(existing => PathContainsOrEquals(normalized, existing))
                .ToArray();
            foreach (var match in matches)
            {
                if (!loadedFromDisk.Contains(match))
                {
                    PendingProviderCleanupDeletes.Remove(match);
                }
            }

            if (matches.Any(match => !loadedFromDisk.Contains(match)))
            {
                PersistProviderCleanupDeletesLocked();
            }
        }

        if (matches.Length == 0)
        {
            return false;
        }

        DebugLogPath(normalized, "ignored provider cleanup delete notify");
        return true;
    }

    private static HashSet<string> LoadProviderCleanupDeletesLocked()
    {
        var loaded = new HashSet<string>(StringComparer.Ordinal);
        try
        {
            var path = Path.Combine(ConfigDirectoryPath, CleanupDeleteFileName);
            if (!File.Exists(path))
            {
                return loaded;
            }

            using var document = JsonDocument.Parse(File.ReadAllText(path));
            if (!document.RootElement.TryGetProperty("entries", out var entries) ||
                entries.ValueKind != JsonValueKind.Array)
            {
                return loaded;
            }

            foreach (var entry in entries.EnumerateArray())
            {
                var markerPath = TryGetJsonString(entry, "path", "Path") is { } rawPath
                    ? NormalizeVirtualPath(rawPath)
                    : "";
                if (string.IsNullOrEmpty(markerPath) || PathHasIgnoredComponent(markerPath))
                {
                    continue;
                }

                var createdAtMs = TryGetJsonInt64(
                    entry,
                    "created_at_unix_ms",
                    "CreatedAtUnixMs");
                if (createdAtMs is null)
                {
                    continue;
                }

                PendingProviderCleanupDeletes[markerPath] =
                    DateTimeOffset.FromUnixTimeMilliseconds(createdAtMs.Value);
                loaded.Add(markerPath);
            }
        }
        catch
        {
            // The Rust daemon may update this best-effort marker concurrently.
        }

        return loaded;
    }

    private static string? TryGetJsonString(
        JsonElement element,
        string lowerName,
        string upperName)
    {
        if (element.TryGetProperty(lowerName, out var lowerValue) &&
            lowerValue.ValueKind == JsonValueKind.String)
        {
            return lowerValue.GetString();
        }

        return element.TryGetProperty(upperName, out var upperValue) &&
            upperValue.ValueKind == JsonValueKind.String
                ? upperValue.GetString()
                : null;
    }

    private static long? TryGetJsonInt64(
        JsonElement element,
        string lowerName,
        string upperName)
    {
        if (element.TryGetProperty(lowerName, out var lowerValue) &&
            lowerValue.ValueKind == JsonValueKind.Number &&
            lowerValue.TryGetInt64(out var lowerParsed))
        {
            return lowerParsed;
        }

        return element.TryGetProperty(upperName, out var upperValue) &&
            upperValue.ValueKind == JsonValueKind.Number &&
            upperValue.TryGetInt64(out var upperParsed)
                ? upperParsed
                : null;
    }

    private static void PersistProviderCleanupDeletesLocked()
    {
        try
        {
            var path = Path.Combine(ConfigDirectoryPath, CleanupDeleteFileName);
            if (PendingProviderCleanupDeletes.Count == 0)
            {
                File.Delete(path);
                return;
            }

            Directory.CreateDirectory(ConfigDirectoryPath);
            var entries = PendingProviderCleanupDeletes
                .OrderBy(entry => entry.Key, StringComparer.Ordinal)
                .Select(entry => new
                {
                    path = entry.Key,
                    created_at_unix_ms = entry.Value.ToUnixTimeMilliseconds(),
                })
                .ToArray();
            File.WriteAllText(path, JsonSerializer.Serialize(new { entries }));
        }
        catch
        {
            // The daemon also has in-process guards; the shared marker is best effort.
        }
    }

    private static void RemoveStalePlaceholders(string syncRootPath, HashSet<string> expectedPaths)
    {
        if (!Directory.Exists(syncRootPath))
        {
            return;
        }

        var removedAny = false;
        var parentDirectories = new HashSet<string>(StringComparer.Ordinal);
        foreach (var fullPath in Directory
            .EnumerateFileSystemEntries(syncRootPath, "*", SearchOption.AllDirectories)
            .OrderByDescending(path => path.Count(ch => ch == Path.DirectorySeparatorChar)))
        {
            var relative = NormalizeVirtualPath(Path.GetRelativePath(syncRootPath, fullPath));
            if (string.IsNullOrEmpty(relative) || expectedPaths.Contains(relative))
            {
                continue;
            }

            if (!ExistingPlaceholder(fullPath))
            {
                continue;
            }

            if (PlaceholderIsTooRecentToPrune(fullPath))
            {
                DebugLogPath(relative, "skip recent stale placeholder");
                continue;
            }

            if (Directory.Exists(fullPath) && DirectoryHasChildren(fullPath))
            {
                DebugLogPath(relative, "skip non-empty stale placeholder directory");
                continue;
            }

            try
            {
                ClearReadOnlyAttribute(fullPath);
                var removed = TryProviderCleanupDelete(
                    relative,
                    () => Directory.Exists(fullPath)
                        ? TryDeleteDirectory(fullPath, recursive: false)
                        : TryDeleteFile(fullPath));

                if (removed)
                {
                    removedAny = true;
                    parentDirectories.Add(ParentPath(relative));
                    DebugLogPath(relative, "removed stale placeholder");
                }
            }
            catch
            {
                // Explorer or Cloud Files may have a transient handle; the next refresh retries.
            }
        }

        if (!removedAny)
        {
            return;
        }

        parentDirectories.Add("");
        foreach (var parent in parentDirectories)
        {
            var fullPath = string.IsNullOrEmpty(parent)
                ? syncRootPath
                : Path.Combine(syncRootPath, FromVirtualPath(parent));
            NotifyShellDirectoryChanged(fullPath);
        }
    }

    private static void RemoveIgnoredLocalItems(string syncRootPath)
    {
        if (!Directory.Exists(syncRootPath))
        {
            return;
        }

        List<string> entries;
        try
        {
            entries = Directory
                .EnumerateFileSystemEntries(syncRootPath, "*", SearchOption.AllDirectories)
                .ToList();
        }
        catch
        {
            return;
        }

        foreach (var fullPath in entries
            .OrderByDescending(path => path.Count(ch => ch == Path.DirectorySeparatorChar)))
        {
            var relative = NormalizeVirtualPath(Path.GetRelativePath(syncRootPath, fullPath));
            if (string.IsNullOrEmpty(relative) || !PathHasIgnoredComponent(relative))
            {
                continue;
            }

            try
            {
                ClearReadOnlyAttribute(fullPath);
                if (Directory.Exists(fullPath))
                {
                    _ = TryDeleteDirectory(fullPath, recursive: true);
                }
                else if (File.Exists(fullPath))
                {
                    _ = TryDeleteFile(fullPath);
                }
            }
            catch
            {
                // Explorer or Cloud Files may have a transient handle; the next refresh retries.
            }
        }
    }

    private static bool PlaceholderIsTooRecentToPrune(string fullPath)
    {
        try
        {
            var isDirectory = Directory.Exists(fullPath);
            var createdAt = isDirectory
                ? Directory.GetCreationTimeUtc(fullPath)
                : File.GetCreationTimeUtc(fullPath);
            var modifiedAt = isDirectory
                ? Directory.GetLastWriteTimeUtc(fullPath)
                : File.GetLastWriteTimeUtc(fullPath);
            var newestTimestamp = createdAt > modifiedAt ? createdAt : modifiedAt;
            return newestTimestamp > DateTime.UtcNow.AddSeconds(-StalePlaceholderGraceSeconds);
        }
        catch
        {
            return false;
        }
    }

    private static bool DirectoryHasChildren(string fullPath)
    {
        try
        {
            return Directory.EnumerateFileSystemEntries(fullPath).Any();
        }
        catch
        {
            return true;
        }
    }

    private static bool TryDeleteFile(string fullPath)
    {
        return TryDeleteWithRetry(() =>
        {
            if (File.Exists(fullPath))
            {
                File.Delete(fullPath);
            }
        });
    }

    private static bool TryDeleteDirectory(string fullPath, bool recursive)
    {
        return TryDeleteWithRetry(() =>
        {
            if (Directory.Exists(fullPath))
            {
                Directory.Delete(fullPath, recursive);
            }
        });
    }

    private static bool TryDeleteWithRetry(Action delete)
    {
        for (var attempt = 0; attempt < DeleteRetryCount; attempt++)
        {
            try
            {
                delete();
                return true;
            }
            catch when (attempt + 1 < DeleteRetryCount)
            {
                Thread.Sleep(DeleteRetryDelayMs);
            }
            catch
            {
                return false;
            }
        }

        return false;
    }

    private static void ClearReadOnlyAttribute(string fullPath)
    {
        try
        {
            if (!File.Exists(fullPath) && !Directory.Exists(fullPath))
            {
                return;
            }

            var attributes = File.GetAttributes(fullPath);
            if ((attributes & FileAttributes.ReadOnly) != 0)
            {
                File.SetAttributes(fullPath, attributes & ~FileAttributes.ReadOnly);
            }
        }
        catch
        {
            // Best-effort cleanup only.
        }
    }

    private static IEnumerable<WindowsCloudFileEntry> PlaceholderEntries(
        IEnumerable<WindowsCloudFileEntry> entries)
    {
        var byPath = new Dictionary<string, WindowsCloudFileEntry>(StringComparer.Ordinal);
        foreach (var entry in entries)
        {
            var path = NormalizeVirtualPath(entry.Path);
            if (string.IsNullOrEmpty(path) || PathHasIgnoredComponent(path))
            {
                continue;
            }

            byPath[path] = entry with { Path = path };
            var parent = ParentPath(path);
            while (!string.IsNullOrEmpty(parent))
            {
                byPath.TryAdd(parent, new WindowsCloudFileEntry(parent, "directory", 0, null));
                parent = ParentPath(parent);
            }
        }

        return byPath.Values
            .OrderBy(entry => entry.Path.Count(ch => ch == '/'))
            .ThenBy(entry => entry.IsDirectory ? 0 : 1)
            .ThenBy(entry => entry.Path, StringComparer.Ordinal);
    }

    private static void CreatePlaceholder(
        string parentFullPath,
        string fileName,
        WindowsCloudFileEntry entry)
    {
        var name = Marshal.StringToHGlobalUni(fileName);
        var identityBytes = Encoding.UTF8.GetBytes(entry.Path);
        var identity = Marshal.AllocHGlobal(identityBytes.Length);
        try
        {
            Marshal.Copy(identityBytes, 0, identity, identityBytes.Length);
            var flags = CfPlaceholderCreateFlagMarkInSync | CfPlaceholderCreateFlagSupersede;
            if (entry.IsDirectory)
            {
                flags |= CfPlaceholderCreateFlagDisableOnDemandPopulation;
            }

            var placeholders = new[]
            {
                new CfPlaceholderCreateInfo
                {
                    RelativeFileName = name,
                    FsMetadata = new CfFsMetadata
                    {
                        BasicInfo = new FileBasicInfo
                        {
                            FileAttributes = entry.IsDirectory
                                ? FileAttributeDirectory
                                : FileAttributeNormal,
                        },
                        FileSize = entry.IsDirectory ? 0 : Math.Max(0, entry.Size),
                    },
                    FileIdentity = identity,
                    FileIdentityLength = (uint)identityBytes.Length,
                    Flags = flags,
                },
            };

            var hresult = NativeMethods.CfCreatePlaceholders(
                parentFullPath,
                placeholders,
                1,
                CfCreateFlagStopOnError,
                out _);
            if (hresult < 0)
            {
                throw new COMException(
                    $"CfCreatePlaceholders failed for {entry.Path}: {FormatHResult(hresult)}",
                    hresult);
            }

            if (placeholders[0].Result < 0)
            {
                throw new COMException(
                    $"CfCreatePlaceholders failed for {entry.Path}: " +
                    FormatHResult(placeholders[0].Result),
                    placeholders[0].Result);
            }
        }
        finally
        {
            Marshal.FreeHGlobal(identity);
            Marshal.FreeHGlobal(name);
        }
    }

    private static bool ExistingLocalItem(string fullPath)
    {
        if (!File.Exists(fullPath) && !Directory.Exists(fullPath))
        {
            return false;
        }

        var attributes = File.GetAttributes(fullPath);
        return (attributes & FileAttributes.ReparsePoint) == 0;
    }

    private static bool ExistingPlaceholder(string fullPath)
    {
        if (!File.Exists(fullPath) && !Directory.Exists(fullPath))
        {
            return false;
        }

        var attributes = File.GetAttributes(fullPath);
        return (attributes & FileAttributes.ReparsePoint) != 0;
    }

    private static void DebugLogPath(string path, string message)
    {
        if (path.Contains("codex-lab-smoke", StringComparison.Ordinal))
        {
            DebugLog($"{path}: {message}");
        }
    }

    private static string SafeAttributes(string fullPath)
    {
        try
        {
            if (!File.Exists(fullPath) && !Directory.Exists(fullPath))
            {
                return "missing";
            }

            return File.GetAttributes(fullPath).ToString();
        }
        catch (Exception error)
        {
            return $"error:{error.Message}";
        }
    }

    private static void RegisterSyncRoot(string path)
    {
        var identityBytes = Encoding.UTF8.GetBytes("iris-drive:main");
        var identity = Marshal.AllocHGlobal(identityBytes.Length);
        try
        {
            Marshal.Copy(identityBytes, 0, identity, identityBytes.Length);
            var registration = new CfSyncRegistration
            {
                StructSize = (uint)Marshal.SizeOf<CfSyncRegistration>(),
                ProviderName = ProviderName,
                ProviderVersion = ProviderVersion,
                SyncRootIdentity = identity,
                SyncRootIdentityLength = (uint)identityBytes.Length,
                FileIdentity = IntPtr.Zero,
                FileIdentityLength = 0,
                ProviderId = ProviderId,
            };
            var policies = new CfSyncPolicies
            {
                StructSize = (uint)Marshal.SizeOf<CfSyncPolicies>(),
                Hydration = new CfHydrationPolicy
                {
                    Primary = CfHydrationPolicyFull,
                    Modifier = 0,
                },
                Population = new CfPopulationPolicy
                {
                    Primary = CfPopulationPolicyAlwaysFull,
                    Modifier = 0,
                },
                InSync = 0,
                HardLink = 0,
                PlaceholderManagement = 0,
            };
            var flags =
                CfRegisterFlagUpdate |
                CfRegisterFlagDisableOnDemandPopulationOnRoot |
                CfRegisterFlagMarkInSyncOnRoot;

            var hresult = NativeMethods.CfRegisterSyncRoot(path, ref registration, ref policies, flags);
            if (hresult >= 0)
            {
                return;
            }

            var createFlags = flags & ~CfRegisterFlagUpdate;
            var createHresult =
                NativeMethods.CfRegisterSyncRoot(path, ref registration, ref policies, createFlags);
            if (createHresult >= 0)
            {
                return;
            }

            throw new COMException(
                $"CfRegisterSyncRoot failed (update={FormatHResult(hresult)}, " +
                $"create={FormatHResult(createHresult)})",
                createHresult);
        }
        finally
        {
            Marshal.FreeHGlobal(identity);
        }
    }

    private static string NormalizeVirtualPath(string path) =>
        path.Replace('\\', '/').Trim('/');

    private static bool PathContainsOrEquals(string ancestor, string path)
    {
        var normalizedAncestor = NormalizeVirtualPath(ancestor);
        var normalizedPath = NormalizeVirtualPath(path);
        return string.Equals(normalizedAncestor, normalizedPath, StringComparison.Ordinal) ||
            normalizedPath.StartsWith(normalizedAncestor + "/", StringComparison.Ordinal);
    }

    private static string ShallowestMissingAncestorPath(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        var components = normalized.Split('/', StringSplitOptions.RemoveEmptyEntries);
        if (components.Length <= 1)
        {
            return normalized;
        }

        for (var length = 1; length < components.Length; length++)
        {
            var ancestor = string.Join('/', components.Take(length));
            if (!SyncRootEntryExists(ancestor))
            {
                return ancestor;
            }
        }

        return normalized;
    }

    private static bool PathHasIgnoredComponent(string path)
    {
        foreach (var component in NormalizeVirtualPath(path).Split(
            '/',
            StringSplitOptions.RemoveEmptyEntries))
        {
            if (IsIgnoredName(component))
            {
                return true;
            }
        }

        return false;
    }

    private static bool IsIgnoredName(string name) =>
        string.Equals(name, ".DS_Store", StringComparison.OrdinalIgnoreCase) ||
        string.Equals(name, ".hashtree", StringComparison.OrdinalIgnoreCase) ||
        string.Equals(name, ".Trash", StringComparison.OrdinalIgnoreCase) ||
        string.Equals(name, "$RECYCLE.BIN", StringComparison.OrdinalIgnoreCase) ||
        string.Equals(name, "Thumbs.db", StringComparison.OrdinalIgnoreCase) ||
        string.Equals(name, "desktop.ini", StringComparison.OrdinalIgnoreCase) ||
        name.StartsWith("._", StringComparison.Ordinal) ||
        name.StartsWith(".Trash-", StringComparison.OrdinalIgnoreCase) ||
        name.EndsWith('~') ||
        (name.StartsWith('#') && name.EndsWith('#')) ||
        string.Equals(Path.GetExtension(name), ".sbak", StringComparison.OrdinalIgnoreCase);

    private static string ParentPath(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        var lastSlash = normalized.LastIndexOf('/');
        return lastSlash < 0 ? "" : normalized[..lastSlash];
    }

    private static string FileName(string path)
    {
        var normalized = NormalizeVirtualPath(path);
        var lastSlash = normalized.LastIndexOf('/');
        return lastSlash < 0 ? normalized : normalized[(lastSlash + 1)..];
    }

    private static string FromVirtualPath(string path) =>
        Path.Combine(NormalizeVirtualPath(path).Split('/', StringSplitOptions.RemoveEmptyEntries));

    private static string FormatHResult(int hresult) => $"0x{hresult:X8}";

    private readonly record struct PlaceholderPopulationReport(
        int PlaceholderCount,
        int SkippedLocalItemCount,
        int FailedPlaceholderCount,
        IReadOnlyCollection<string> RefreshedPaths,
        IReadOnlyCollection<string> ProtectedLocalItemPaths);

    private readonly record struct LocalFileSnapshot(long Size, string Sha256);

    private sealed class CloudFilesConnection : IDisposable
    {
        private readonly string syncRootPath;
        private readonly Func<string, byte[]> readFile;
        private readonly Action<string>? deletePath;
        private readonly Action<string, string>? renamePath;
        private readonly CfCallback fetchDataCallback;
        private readonly CfCallback deleteCallback;
        private readonly CfCallback renameCallback;
        private readonly IntPtr callbackTable;
        private long connectionKey;
        private bool disposed;

        private CloudFilesConnection(
            string syncRootPath,
            Func<string, byte[]> readFile,
            Action<string>? deletePath,
            Action<string, string>? renamePath)
        {
            this.syncRootPath = syncRootPath;
            this.readFile = readFile;
            this.deletePath = deletePath;
            this.renamePath = renamePath;
            fetchDataCallback = OnFetchData;
            deleteCallback = OnNotifyDelete;
            renameCallback = OnNotifyRename;
            callbackTable = AllocateCallbackTable(fetchDataCallback, deleteCallback, renameCallback);
        }

        public static CloudFilesConnection Connect(
            string syncRootPath,
            Func<string, byte[]> readFile,
            Action<string>? deletePath,
            Action<string, string>? renamePath)
        {
            var connection = new CloudFilesConnection(syncRootPath, readFile, deletePath, renamePath);
            var hresult = NativeMethods.CfConnectSyncRoot(
                syncRootPath,
                connection.callbackTable,
                IntPtr.Zero,
                CfConnectFlagRequireFullFilePath,
                out connection.connectionKey);
            if (hresult < 0)
            {
                connection.Dispose();
                throw new COMException(
                    $"CfConnectSyncRoot failed: {FormatHResult(hresult)}",
                    hresult);
            }

            return connection;
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }

            disposed = true;
            if (connectionKey != 0)
            {
                NativeMethods.CfDisconnectSyncRoot(connectionKey);
                connectionKey = 0;
            }

            Marshal.FreeHGlobal(callbackTable);
        }

        private static IntPtr AllocateCallbackTable(
            CfCallback fetchData,
            CfCallback delete,
            CfCallback rename)
        {
            var registrations = new[]
            {
                new CfCallbackRegistration
                {
                    Type = CfCallbackTypeFetchData,
                    Callback = Marshal.GetFunctionPointerForDelegate(fetchData),
                },
                new CfCallbackRegistration
                {
                    Type = CfCallbackTypeNotifyDelete,
                    Callback = Marshal.GetFunctionPointerForDelegate(delete),
                },
                new CfCallbackRegistration
                {
                    Type = CfCallbackTypeNotifyRename,
                    Callback = Marshal.GetFunctionPointerForDelegate(rename),
                },
                new CfCallbackRegistration
                {
                    Type = CfCallbackTypeNone,
                    Callback = IntPtr.Zero,
                },
            };
            var size = Marshal.SizeOf<CfCallbackRegistration>();
            var table = Marshal.AllocHGlobal(size * registrations.Length);
            for (var index = 0; index < registrations.Length; index++)
            {
                Marshal.StructureToPtr(
                    registrations[index],
                    IntPtr.Add(table, index * size),
                    false);
            }

            return table;
        }

        private void OnFetchData(IntPtr callbackInfo, IntPtr callbackParameters)
        {
            var info = Marshal.PtrToStructure<CfCallbackInfo>(callbackInfo);
            try
            {
                var path = FileIdentityToPath(info);
                var bytes = readFile(path);
                TransferData(info, bytes, StatusSuccess);
            }
            catch (Exception error)
            {
                Debug.WriteLine($"Iris Drive Cloud Files hydration failed: {error}");
                TransferData(info, Array.Empty<byte>(), StatusUnsuccessful);
            }
        }

        private void OnNotifyDelete(IntPtr callbackInfo, IntPtr callbackParameters)
        {
            var info = Marshal.PtrToStructure<CfCallbackInfo>(callbackInfo);
            try
            {
                var path = FileIdentityToPath(info);
                DebugLog($"cloud delete notify path={path}");
                if (TryConsumeProviderCleanupDelete(path))
                {
                    AckDelete(info, StatusSuccess);
                    return;
                }

                deletePath?.Invoke(path);
                AckDelete(info, StatusSuccess);
            }
            catch (Exception error)
            {
                DebugLog($"cloud delete notify failed: {error.Message}");
                AckDelete(info, StatusUnsuccessful);
            }
        }

        private void OnNotifyRename(IntPtr callbackInfo, IntPtr callbackParameters)
        {
            var info = Marshal.PtrToStructure<CfCallbackInfo>(callbackInfo);
            try
            {
                var oldPath = FileIdentityToPath(info);
                var parameters = Marshal.PtrToStructure<CfCallbackParametersRename>(callbackParameters);
                var targetPath = Marshal.PtrToStringUni(parameters.Rename.TargetPath);
                if (string.IsNullOrWhiteSpace(targetPath))
                {
                    throw new InvalidOperationException("Cloud Files rename callback did not include a target path.");
                }

                var newPath = NormalizedPathToRelative(targetPath);
                DebugLog($"cloud rename notify old={oldPath} new={newPath}");
                renamePath?.Invoke(oldPath, newPath);
                AckRename(info, StatusSuccess);
            }
            catch (Exception error)
            {
                DebugLog($"cloud rename notify failed: {error.Message}");
                AckRename(info, StatusUnsuccessful);
            }
        }

        private string FileIdentityToPath(CfCallbackInfo info)
        {
            if (info.FileIdentity != IntPtr.Zero && info.FileIdentityLength > 0)
            {
                var bytes = new byte[info.FileIdentityLength];
                Marshal.Copy(info.FileIdentity, bytes, 0, bytes.Length);
                return Encoding.UTF8.GetString(bytes);
            }

            var normalizedPath = Marshal.PtrToStringUni(info.NormalizedPath);
            if (string.IsNullOrWhiteSpace(normalizedPath))
            {
                throw new InvalidOperationException("Cloud Files callback did not include a path.");
            }

            return NormalizedPathToRelative(normalizedPath);
        }

        private string NormalizedPathToRelative(string normalizedPath)
        {
            return Path
                .GetRelativePath(syncRootPath, normalizedPath)
                .Replace('\\', '/')
                .Trim('/');
        }

        private static void TransferData(CfCallbackInfo info, byte[] bytes, int status)
        {
            var operationInfo = new CfOperationInfo
            {
                StructSize = (uint)Marshal.SizeOf<CfOperationInfo>(),
                Type = CfOperationTypeTransferData,
                ConnectionKey = info.ConnectionKey,
                TransferKey = info.TransferKey,
                CorrelationVector = info.CorrelationVector,
                SyncStatus = IntPtr.Zero,
                RequestKey = info.RequestKey,
            };

            var handle = bytes.Length == 0
                ? default
                : GCHandle.Alloc(bytes, GCHandleType.Pinned);
            try
            {
                var parameters = new CfOperationParametersTransferData
                {
                    ParamSize = (uint)Marshal.SizeOf<CfOperationParametersTransferData>(),
                    TransferData = new CfOperationTransferData
                    {
                        Flags = CfOperationTransferDataFlagNone,
                        CompletionStatus = status,
                        Buffer = handle.IsAllocated
                            ? handle.AddrOfPinnedObject()
                            : IntPtr.Zero,
                        Offset = 0,
                        Length = status == StatusSuccess ? bytes.LongLength : 0,
                    },
                };

                var hresult = NativeMethods.CfExecute(ref operationInfo, ref parameters);
                if (hresult < 0)
                {
                    Debug.WriteLine(
                        $"Iris Drive Cloud Files CfExecute failed: {FormatHResult(hresult)}");
                }
            }
            finally
            {
                if (handle.IsAllocated)
                {
                    handle.Free();
                }
            }
        }

        private static void AckDelete(CfCallbackInfo info, int status)
        {
            var operationInfo = new CfOperationInfo
            {
                StructSize = (uint)Marshal.SizeOf<CfOperationInfo>(),
                Type = CfOperationTypeAckDelete,
                ConnectionKey = info.ConnectionKey,
                TransferKey = info.TransferKey,
                CorrelationVector = info.CorrelationVector,
                SyncStatus = IntPtr.Zero,
                RequestKey = info.RequestKey,
            };

            var parameters = new CfOperationParametersAckDelete
            {
                ParamSize = (uint)Marshal.SizeOf<CfOperationParametersAckDelete>(),
                AckDelete = new CfOperationAckDelete
                {
                    Flags = CfOperationAckDeleteFlagNone,
                    CompletionStatus = status,
                },
            };

            var hresult = NativeMethods.CfExecute(ref operationInfo, ref parameters);
            if (hresult < 0)
            {
                DebugLog($"Iris Drive Cloud Files delete ack failed: {FormatHResult(hresult)}");
            }
        }

        private static void AckRename(CfCallbackInfo info, int status)
        {
            var operationInfo = new CfOperationInfo
            {
                StructSize = (uint)Marshal.SizeOf<CfOperationInfo>(),
                Type = CfOperationTypeAckRename,
                ConnectionKey = info.ConnectionKey,
                TransferKey = info.TransferKey,
                CorrelationVector = info.CorrelationVector,
                SyncStatus = IntPtr.Zero,
                RequestKey = info.RequestKey,
            };

            var parameters = new CfOperationParametersAckRename
            {
                ParamSize = (uint)Marshal.SizeOf<CfOperationParametersAckRename>(),
                AckRename = new CfOperationAckRename
                {
                    Flags = CfOperationAckRenameFlagNone,
                    CompletionStatus = status,
                },
            };

            var hresult = NativeMethods.CfExecute(ref operationInfo, ref parameters);
            if (hresult < 0)
            {
                DebugLog($"Iris Drive Cloud Files rename ack failed: {FormatHResult(hresult)}");
            }
        }
    }

}
