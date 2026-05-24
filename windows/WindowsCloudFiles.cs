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

namespace IrisDrive.WindowsShell;

public sealed class DriveFolderPreparation
{
    public DriveFolderPreparation(
        string path,
        bool nativeSyncRootReady,
        string? warning,
        int placeholderCount = 0,
        int skippedLocalItemCount = 0)
    {
        Path = path;
        NativeSyncRootReady = nativeSyncRootReady;
        Warning = warning;
        PlaceholderCount = placeholderCount;
        SkippedLocalItemCount = skippedLocalItemCount;
    }

    public string Path { get; }
    public bool NativeSyncRootReady { get; }
    public string? Warning { get; }
    public int PlaceholderCount { get; }
    public int SkippedLocalItemCount { get; }
}

public sealed record WindowsCloudFileEntry(string Path, string Kind, long Size)
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

        return new WindowsCloudFileEntry(path, kind, size);
    }
}

public sealed record WindowsCloudLocalStateEntry(string Path, string Kind, long Size, string? Sha256)
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

        return new WindowsCloudLocalStateEntry(path, kind, size, sha256);
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
    private const int CfCallbackTypeNone = -1;
    private const int CfConnectFlagRequireFullFilePath = 0x00000004;
    private const int CfCreateFlagStopOnError = 0x00000001;
    private const int CfPlaceholderCreateFlagDisableOnDemandPopulation = 0x00000001;
    private const int CfPlaceholderCreateFlagMarkInSync = 0x00000002;
    private const int CfPlaceholderCreateFlagSupersede = 0x00000004;
    private const int CfOperationTypeTransferData = 0;
    private const int CfOperationTransferDataFlagNone = 0;
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
    private const string LocalStateFileName = "windows-cloud-local-state.json";
    private static readonly Guid ProviderId = new("2b58fb5d-b823-4d84-bd52-fcf9bd297fd4");
    private static readonly object ConnectionLock = new();
    private static CloudFilesConnection? activeConnection;

    public static string SyncRootPath =>
        System.IO.Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
            "Iris Drive");

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
        IReadOnlyCollection<WindowsCloudFileEntry> entries)
    {
        try
        {
            Directory.CreateDirectory(configDirectory);
            var state = SnapshotLocalState(entries);
            var json = JsonSerializer.Serialize(new { entries = state });
            File.WriteAllText(Path.Combine(configDirectory, LocalStateFileName), json);
        }
        catch
        {
            // This is a safety cache for remote-delete cleanup; sync still works without it.
        }
    }

    public static void RemoveStaleSyncedLocalItems(
        IReadOnlyCollection<WindowsCloudFileEntry> entries,
        IReadOnlyCollection<WindowsCloudLocalStateEntry> previousState)
    {
        if (previousState.Count == 0)
        {
            return;
        }

        var expectedPaths = new HashSet<string>(
            PlaceholderEntries(entries).Select(entry => entry.Path),
            StringComparer.Ordinal);
        var removedAny = false;

        foreach (var previous in previousState
            .OrderByDescending(entry => entry.Path.Count(ch => ch == '/')))
        {
            var path = NormalizeVirtualPath(previous.Path);
            if (string.IsNullOrEmpty(path) ||
                PathHasIgnoredComponent(path) ||
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
                        Directory.Delete(fullPath, recursive: false);
                        removedAny = true;
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
                File.Delete(fullPath);
                removedAny = true;
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
        Func<string, byte[]> readFile)
    {
        var path = SyncRootPath;
        Directory.CreateDirectory(path);

        try
        {
            RegisterSyncRoot(path);
            var population = PopulatePlaceholders(path, entries);
            NotifyShellDirectoryChanged(path);

            lock (ConnectionLock)
            {
                activeConnection?.Dispose();
                activeConnection = CloudFilesConnection.Connect(path, readFile);
            }

            var warning = population.SkippedLocalItemCount == 0
                ? null
                : $"{population.SkippedLocalItemCount} existing local item(s) were left in place.";
            return new DriveFolderPreparation(
                path,
                nativeSyncRootReady: true,
                warning,
                population.PlaceholderCount,
                population.SkippedLocalItemCount);
        }
        catch (DllNotFoundException error)
        {
            return Fallback(path, $"Cloud Files API unavailable: {error.Message}");
        }
        catch (EntryPointNotFoundException error)
        {
            return Fallback(path, $"Cloud Files API unavailable: {error.Message}");
        }
        catch (Win32Exception error)
        {
            return Fallback(path, $"Cloud Files operation failed: {error.Message}");
        }
        catch (COMException error)
        {
            return Fallback(path, $"Cloud Files operation failed: {error.Message}");
        }
    }

    private static DriveFolderPreparation Fallback(string path, string warning) =>
        new(path, nativeSyncRootReady: false, warning);

    private static IReadOnlyList<WindowsCloudLocalStateEntry> SnapshotLocalState(
        IReadOnlyCollection<WindowsCloudFileEntry> entries)
    {
        var state = new List<WindowsCloudLocalStateEntry>();
        foreach (var entry in PlaceholderEntries(entries))
        {
            var fullPath = Path.Combine(SyncRootPath, FromVirtualPath(entry.Path));
            try
            {
                if (Directory.Exists(fullPath))
                {
                    state.Add(new WindowsCloudLocalStateEntry(
                        entry.Path,
                        "directory",
                        0,
                        null));
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
                        null));
                    continue;
                }

                var snapshot = SnapshotLocalFile(fullPath);
                if (snapshot is not null)
                {
                    state.Add(new WindowsCloudLocalStateEntry(
                        entry.Path,
                        "file",
                        snapshot.Value.Size,
                        snapshot.Value.Sha256));
                }
            }
            catch
            {
                // A transiently locked file should not block the whole provider refresh.
            }
        }

        return state
            .OrderBy(entry => entry.Path, StringComparer.Ordinal)
            .ToArray();
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
        IReadOnlyCollection<WindowsCloudFileEntry> entries)
    {
        var placeholderCount = 0;
        var skippedLocalItems = 0;
        var expectedPaths = new HashSet<string>(
            PlaceholderEntries(entries).Select(entry => entry.Path),
            StringComparer.Ordinal);

        RemoveIgnoredLocalItems(syncRootPath);
        RemoveStalePlaceholders(syncRootPath, expectedPaths);

        foreach (var entry in PlaceholderEntries(entries))
        {
            var parentPath = ParentPath(entry.Path);
            var parentFullPath = string.IsNullOrEmpty(parentPath)
                ? syncRootPath
                : Path.Combine(syncRootPath, FromVirtualPath(parentPath));
            if (!Directory.Exists(parentFullPath))
            {
                skippedLocalItems++;
                continue;
            }

            var itemFullPath = Path.Combine(parentFullPath, FileName(entry.Path));
            if (ExistingPlaceholder(itemFullPath))
            {
                continue;
            }

            if (ExistingLocalItem(itemFullPath))
            {
                skippedLocalItems++;
                continue;
            }

            CreatePlaceholder(parentFullPath, FileName(entry.Path), entry);
            placeholderCount++;
        }

        return new PlaceholderPopulationReport(placeholderCount, skippedLocalItems);
    }

    private static void NotifyShellDirectoryChanged(string path)
    {
        NotifyShellPathChanged(ShcneUpdateDir, path);
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

    private static void RemoveStalePlaceholders(string syncRootPath, HashSet<string> expectedPaths)
    {
        if (!Directory.Exists(syncRootPath))
        {
            return;
        }

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

            try
            {
                if (Directory.Exists(fullPath))
                {
                    Directory.Delete(fullPath, recursive: true);
                }
                else
                {
                    File.Delete(fullPath);
                }
            }
            catch
            {
                // Explorer or Cloud Files may have a transient handle; the next refresh retries.
            }
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
                    Directory.Delete(fullPath, recursive: true);
                }
                else if (File.Exists(fullPath))
                {
                    File.Delete(fullPath);
                }
            }
            catch
            {
                // Explorer or Cloud Files may have a transient handle; the next refresh retries.
            }
        }
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
                byPath.TryAdd(parent, new WindowsCloudFileEntry(parent, "directory", 0));
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
        int SkippedLocalItemCount);

    private readonly record struct LocalFileSnapshot(long Size, string Sha256);

    private sealed class CloudFilesConnection : IDisposable
    {
        private readonly string syncRootPath;
        private readonly Func<string, byte[]> readFile;
        private readonly CfCallback fetchDataCallback;
        private readonly IntPtr callbackTable;
        private long connectionKey;
        private bool disposed;

        private CloudFilesConnection(string syncRootPath, Func<string, byte[]> readFile)
        {
            this.syncRootPath = syncRootPath;
            this.readFile = readFile;
            fetchDataCallback = OnFetchData;
            callbackTable = AllocateCallbackTable(fetchDataCallback);
        }

        public static CloudFilesConnection Connect(string syncRootPath, Func<string, byte[]> readFile)
        {
            var connection = new CloudFilesConnection(syncRootPath, readFile);
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

        private static IntPtr AllocateCallbackTable(CfCallback fetchData)
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
    }

}
