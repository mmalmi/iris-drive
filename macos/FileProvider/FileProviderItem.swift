import Foundation
import FileProvider
import Security
import UniformTypeIdentifiers

final class FileProviderItem: NSObject, NSFileProviderItem {
    let itemIdentifier: NSFileProviderItemIdentifier
    let parentItemIdentifier: NSFileProviderItemIdentifier
    let filename: String
    let contentType: UTType
    let itemSize: NSNumber?
    let created: Date?
    let modified: Date?
    let itemVersion: NSFileProviderItemVersion

    init(
        itemIdentifier: NSFileProviderItemIdentifier,
        parentItemIdentifier: NSFileProviderItemIdentifier,
        filename: String,
        contentType: UTType,
        itemSize: NSNumber? = nil,
        created: Date? = nil,
        modified: Date? = nil,
        versionIdentifier: String = "iris-drive-provider-v1"
    ) {
        self.itemIdentifier = itemIdentifier
        self.parentItemIdentifier = parentItemIdentifier
        self.filename = filename
        self.contentType = contentType
        self.itemSize = itemSize
        self.created = created
        self.modified = modified
        let version = Data(versionIdentifier.utf8)
        self.itemVersion = NSFileProviderItemVersion(
            contentVersion: version,
            metadataVersion: version
        )
    }

    var capabilities: NSFileProviderItemCapabilities {
        if contentType == .folder {
            return [
                .allowsReading,
                .allowsWriting,
                .allowsContentEnumerating,
                .allowsAddingSubItems,
                .allowsRenaming,
                .allowsReparenting,
                .allowsDeleting,
            ]
        }
        return [
            .allowsReading,
            .allowsWriting,
            .allowsRenaming,
            .allowsReparenting,
            .allowsDeleting,
        ]
    }

    var documentSize: NSNumber? {
        itemSize
    }

    var creationDate: Date? {
        created
    }

    var contentModificationDate: Date? {
        modified
    }
}

extension FileProviderItem {
    static func root(anchor: String? = nil) -> FileProviderItem {
        FileProviderItem(
            itemIdentifier: .rootContainer,
            parentItemIdentifier: .rootContainer,
            filename: "My Drive",
            contentType: .folder,
            versionIdentifier: "root:\(anchor ?? "initial")"
        )
    }

    static func trash(anchor: String? = nil) -> FileProviderItem {
        FileProviderItem(
            itemIdentifier: .trashContainer,
            parentItemIdentifier: .rootContainer,
            filename: ".Trash",
            contentType: .folder,
            versionIdentifier: "trash:\(anchor ?? "initial")"
        )
    }
}

enum FileProviderStorage {
    private static let runtimeFileName = "fileprovider-runtime.json"
    private static let snapshotFileName = "fileprovider-snapshot.json"
    private static let debugLogFileName = "fileprovider-extension.log"
    private static let pathPrefix = "path:"
    private static let tempDirectoryName = "FileProviderTmp"
    private static let contentCacheDirectoryName = "FileProviderContentCache"
    private static let providerListRetryDelays: [TimeInterval] = [0.15, 0.35, 0.75, 1.5]
    private static let providerListCacheTTL: TimeInterval = 1.0
    private static let providerListCacheLock = NSLock()
    private static let contentCacheLock = NSLock()
    private static var providerListCache: (loadedAt: Date, list: ProviderList)?
    private static var configuredRuntime: Runtime?

    struct Runtime: Decodable {
        let configDirectory: String?
        let idriveExecutable: String?

        init(configDirectory: String?, idriveExecutable: String?) {
            self.configDirectory = configDirectory
            self.idriveExecutable = idriveExecutable
        }

        init?(userInfo: [AnyHashable: Any]?) {
            let configDirectory = userInfo?["config_dir"] as? String
            let idriveExecutable = userInfo?["idrive_executable"] as? String
            guard configDirectory != nil || idriveExecutable != nil else {
                return nil
            }
            self.init(
                configDirectory: configDirectory,
                idriveExecutable: idriveExecutable
            )
        }

        enum CodingKeys: String, CodingKey {
            case configDirectory = "config_dir"
            case idriveExecutable = "idrive_executable"
        }
    }

    static func configure(domain: NSFileProviderDomain) {
        if #available(macOS 15.0, *) {
            configuredRuntime = Runtime(userInfo: domain.userInfo)
            debugLog(
                "configure domain=\(domain.identifier.rawValue) runtime=\(configuredRuntime != nil)"
            )
        } else {
            debugLog("configure domain=\(domain.identifier.rawValue) runtime=userInfo-unavailable")
        }
    }

    static func debugLog(_ message: String) {
        let clean = message.replacingOccurrences(of: "\n", with: "\\n")
        NSLog("Iris Drive FileProvider \(clean)")

        let formatter = ISO8601DateFormatter()
        let line = "\(formatter.string(from: Date())) \(clean)\n"
        guard let data = line.data(using: .utf8) else { return }

        let urls = [
            appGroupApplicationSupportDirectory()
                .appendingPathComponent(debugLogFileName, isDirectory: false),
        ]
        for url in urls {
            appendDebugLog(data, to: url)
        }
    }

    private static func appendDebugLog(_ data: Data, to url: URL) {
        do {
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            if FileManager.default.fileExists(atPath: url.path) {
                let handle = try FileHandle(forWritingTo: url)
                defer { try? handle.close() }
                try handle.seekToEnd()
                try handle.write(contentsOf: data)
            } else {
                try data.write(to: url)
            }
        } catch {
            NSLog("Iris Drive FileProvider debug log write failed at \(url.path): \(error)")
        }
    }

    private struct ProviderList: Decodable {
        let anchor: String?
        let entries: [ProviderEntry]
    }

    private struct ProviderEntry: Decodable {
        let path: String
        let kind: String
        let size: UInt64
        let version: String?
    }

    private struct ProviderSnapshot: Codable {
        let anchor: String
        let identifiers: [String]
    }

    static var baseDirectory: URL {
        runtimeDirectory ?? runtimeDirectories[0]
    }

    static var configDirectory: URL {
        if let configured = runtime?.configDirectory, !configured.isEmpty {
            return URL(fileURLWithPath: configured, isDirectory: true)
        }
        return baseDirectory.appendingPathComponent("Config", isDirectory: true)
    }

    static var runtime: Runtime? {
        if let configuredRuntime, runtimeIsUsable(configuredRuntime) {
            return configuredRuntime
        } else if configuredRuntime != nil {
            debugLog("configured runtime inaccessible; using runtime file")
        }
        for directory in runtimeDirectories {
            let url = directory.appendingPathComponent(runtimeFileName)
            guard let data = try? Data(contentsOf: url) else { continue }
            do {
                return try JSONDecoder().decode(Runtime.self, from: data)
            } catch {
                NSLog("Iris Drive FileProvider runtime decode failed at \(url.path): \(error)")
            }
        }
        return nil
    }

    private static func runtimeIsUsable(_ runtime: Runtime) -> Bool {
        guard let configDirectory = runtime.configDirectory,
              !configDirectory.isEmpty
        else {
            return true
        }
        return FileManager.default.isReadableFile(atPath: configDirectory)
    }

    private static var runtimeDirectory: URL? {
        runtimeDirectories.first { directory in
            FileManager.default.fileExists(
                atPath: directory.appendingPathComponent(runtimeFileName).path
            )
        }
    }

    private static var runtimeDirectories: [URL] {
        var directories = [URL]()
        directories.append(appGroupApplicationSupportDirectory())

        var seen = Set<String>()
        return directories.filter { directory in
            seen.insert(directory.standardizedFileURL.path).inserted
        }
    }

    static var idriveExecutable: String? {
        let extensionBundled = Bundle.main.bundleURL
            .appendingPathComponent("Contents", isDirectory: true)
            .appendingPathComponent("MacOS", isDirectory: true)
            .appendingPathComponent("idrive")
        if FileManager.default.isExecutableFile(atPath: extensionBundled.path) {
            return extensionBundled.path
        }

        if let configured = runtime?.idriveExecutable, !configured.isEmpty {
            if FileManager.default.isExecutableFile(atPath: configured) {
                return configured
            }
            debugLog("configured idrive helper unavailable at \(configured)")
        }

        let containingAppBundled = Bundle.main.bundleURL
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("MacOS", isDirectory: true)
            .appendingPathComponent("idrive")
        guard FileManager.default.isExecutableFile(atPath: containingAppBundled.path) else {
            debugLog("bundled idrive helper unavailable")
            return nil
        }
        return containingAppBundled.path
    }

    static func item(for identifier: NSFileProviderItemIdentifier) -> FileProviderItem? {
        if identifier == .rootContainer || identifier == .workingSet {
            return .root(anchor: providerList().anchor)
        }
        if identifier == .trashContainer {
            return .trash(anchor: providerList().anchor)
        }
        let list = providerList()
        guard let path = path(for: identifier),
              let entry = list.entries.first(where: { $0.path == path })
        else {
            return nil
        }
        return item(for: entry, anchor: list.anchor)
    }

    static func path(for identifier: NSFileProviderItemIdentifier) -> String? {
        if identifier == .rootContainer || identifier == .workingSet {
            return ""
        }
        let raw = identifier.rawValue
        guard raw.hasPrefix(pathPrefix) else { return nil }
        let encoded = String(raw.dropFirst(pathPrefix.count))
        guard let data = Data(base64Encoded: encoded),
              let relative = String(data: data, encoding: .utf8),
              isSafeRelativePath(relative)
        else {
            return nil
        }
        return relative
    }

    static func identifier(for path: String) -> NSFileProviderItemIdentifier {
        if path.isEmpty {
            return .rootContainer
        }
        let encoded = Data(path.utf8).base64EncodedString()
        return NSFileProviderItemIdentifier("\(pathPrefix)\(encoded)")
    }

    static func children(of containerIdentifier: NSFileProviderItemIdentifier) -> [FileProviderItem] {
        if containerIdentifier == .trashContainer {
            debugLog("children parent=trash count=0")
            return []
        }
        guard let parent = path(for: containerIdentifier) else { return [] }
        let list = providerList()
        let items = list.entries
            .filter { parentPath(for: $0.path) == parent }
            .sorted { $0.path.localizedStandardCompare($1.path) == .orderedAscending }
            .map { item(for: $0, anchor: list.anchor) }
        debugLog("children parent=\(parent.isEmpty ? "/" : parent) count=\(items.count)")
        return items
    }

    static func allItems() -> [FileProviderItem] {
        allItemsAndAnchor().items
    }

    static func allItemsAndAnchor() -> (items: [FileProviderItem], anchor: NSFileProviderSyncAnchor) {
        let list = providerList()
        var items = [FileProviderItem]()
        if list.anchor != nil {
            items.append(.root(anchor: list.anchor))
            items.append(.trash(anchor: list.anchor))
        }
        items.append(contentsOf: list.entries.map { item(for: $0, anchor: list.anchor) })
        return (
            items,
            syncAnchor(for: list.anchor)
        )
    }

    static func storedSnapshotIdentifiers() -> Set<String> {
        guard let snapshot = storedSnapshot() else {
            return []
        }
        return Set(snapshot.identifiers)
    }

    static func storedSnapshotAnchor() -> NSFileProviderSyncAnchor? {
        guard let snapshot = storedSnapshot() else {
            return nil
        }
        return syncAnchor(for: snapshot.anchor)
    }

    static func currentProviderAnchor() -> NSFileProviderSyncAnchor {
        let list = providerList()
        guard list.anchor != nil else {
            return storedSnapshotAnchor() ?? bootstrapAnchor()
        }
        return syncAnchor(for: list.anchor)
    }

    static func bootstrapAnchor() -> NSFileProviderSyncAnchor {
        syncAnchor(for: "bootstrap")
    }

    private static func storedSnapshot() -> ProviderSnapshot? {
        let url = snapshotURL()
        guard let data = try? Data(contentsOf: url),
              let snapshot = try? JSONDecoder().decode(ProviderSnapshot.self, from: data)
        else {
            return nil
        }
        guard snapshot.anchor != "unavailable" else {
            return nil
        }
        return snapshot
    }

    static func hasStoredSnapshot() -> Bool {
        let url = snapshotURL()
        guard let data = try? Data(contentsOf: url) else { return false }
        return (try? JSONDecoder().decode(ProviderSnapshot.self, from: data)) != nil
    }

    static func recordSnapshot(
        items: [FileProviderItem],
        anchor: NSFileProviderSyncAnchor
    ) {
        do {
            try FileManager.default.createDirectory(
                at: baseDirectory,
                withIntermediateDirectories: true
            )
            let anchorString = String(data: anchor.rawValue, encoding: .utf8) ?? "unavailable"
            guard anchorString != "unavailable" || !items.isEmpty else {
                try? FileManager.default.removeItem(at: snapshotURL())
                return
            }
            let snapshot = ProviderSnapshot(
                anchor: anchorString,
                identifiers: items
                    .map(\.itemIdentifier.rawValue)
                    .sorted()
            )
            let data = try JSONEncoder().encode(snapshot)
            try data.write(to: snapshotURL())
            debugLog("record snapshot anchor=\(anchorString) identifiers=\(snapshot.identifiers.count)")
        } catch {
            debugLog("snapshot write failed: \(error)")
        }
    }

    static func createItem(
        template: NSFileProviderItem,
        contents: URL?
    ) throws -> FileProviderItem {
        let parent = path(for: template.parentItemIdentifier) ?? ""
        let destination = joinedPath(parent: parent, name: template.filename)
        NSLog("Iris Drive FileProvider create path=\(destination)")
        invalidateProviderListCache()
        if (template.contentType ?? .data).conforms(to: .folder) {
            _ = try runIDrive(arguments: ["provider", "mkdir", destination])
        } else if let contents {
            _ = try runIDrive(arguments: ["provider", "write", destination, contents.path])
        } else {
            let empty = try emptyTemporaryFile()
            _ = try runIDrive(arguments: ["provider", "write", destination, empty.path])
        }
        let item = optimisticItem(for: destination, template: template, contents: contents)
        invalidateProviderListCache()
        NSLog("Iris Drive FileProvider created path=\(destination) optimistic=true")
        return item
    }

    static func modifyItem(
        _ item: NSFileProviderItem,
        changedFields: NSFileProviderItemFields,
        contents: URL?
    ) throws -> FileProviderItem? {
        guard let original = path(for: item.itemIdentifier), !original.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier)
        }
        NSLog("Iris Drive FileProvider modify path=\(original)")
        if changedFields.contains(.parentItemIdentifier),
           item.parentItemIdentifier == .trashContainer {
            try deleteItem(identifier: item.itemIdentifier)
            NSLog("Iris Drive FileProvider moved to trash path=\(original)")
            return nil
        }

        let parent: String
        if changedFields.contains(.parentItemIdentifier) {
            guard let resolvedParent = path(for: item.parentItemIdentifier) else {
                throw NSError.fileProviderErrorForNonExistentItem(
                    withIdentifier: item.parentItemIdentifier
                )
            }
            parent = resolvedParent
        } else {
            parent = parentPath(for: original)
        }
        let name = changedFields.contains(.filename) ? item.filename : fileName(for: original)
        let destination = joinedPath(parent: parent, name: name)
        invalidateProviderListCache()
        if destination != original {
            _ = try runIDrive(arguments: ["provider", "rename", original, destination])
        }
        if let contents, !(item.contentType ?? .data).conforms(to: .folder) {
            _ = try runIDrive(arguments: ["provider", "write", destination, contents.path])
        }
        let updated = optimisticItem(for: destination, template: item, contents: contents)
        invalidateProviderListCache()
        NSLog("Iris Drive FileProvider modified path=\(destination) optimistic=true")
        return updated
    }

    static func deleteItem(identifier: NSFileProviderItemIdentifier) throws {
        guard let path = path(for: identifier), !path.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        NSLog("Iris Drive FileProvider delete path=\(path)")
        invalidateProviderListCache()
        _ = try runIDrive(arguments: ["provider", "delete", path])
        invalidateProviderListCache()
    }

    static func contentsURL(for identifier: NSFileProviderItemIdentifier) throws -> URL {
        guard let path = path(for: identifier), !path.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        let list = providerList()
        if let entry = list.entries.first(where: { $0.path == path && $0.kind != "directory" }) {
            if let cached = try existingCachedContentsURL(for: entry, anchor: list.anchor) {
                debugLog("fetch contents cache hit path=\(path)")
                return cached
            }
            do {
                let cached = try cachedContentsURL(for: entry, anchor: list.anchor)
                debugLog("fetch contents private cache path=\(path)")
                return cached
            } catch {
                debugLog("fetch contents private cache failed path=\(path) error=\(error)")
            }
        }

        let directory = try temporaryDirectory()
        let output = directory
            .appendingPathComponent(UUID().uuidString, isDirectory: false)
            .appendingPathExtension((path as NSString).pathExtension)
        NSLog("Iris Drive FileProvider fetch contents path=\(path)")
        _ = try runIDrive(arguments: ["provider", "read", path, output.path])
        return output
    }

    private static func item(for entry: ProviderEntry, anchor: String?) -> FileProviderItem {
        let isDirectory = entry.kind == "directory"
        let contentType: UTType = isDirectory
            ? UTType.folder
            : UTType(filenameExtension: (entry.path as NSString).pathExtension) ?? .data
        return FileProviderItem(
            itemIdentifier: identifier(for: entry.path),
            parentItemIdentifier: identifier(for: parentPath(for: entry.path)),
            filename: fileName(for: entry.path),
            contentType: contentType,
            itemSize: isDirectory ? nil : NSNumber(value: entry.size),
            created: nil,
            modified: nil,
            versionIdentifier: "\(anchor ?? "unavailable"):\(entry.kind):\(entry.path):\(entry.size)"
        )
    }

    private static func optimisticItem(
        for path: String,
        template: NSFileProviderItem,
        contents: URL?
    ) -> FileProviderItem {
        let isDirectory = (template.contentType ?? .data).conforms(to: .folder)
        let size = isDirectory ? nil : NSNumber(value: fileSize(at: contents))
        let contentType = isDirectory
            ? UTType.folder
            : UTType(filenameExtension: (path as NSString).pathExtension) ?? .data
        return FileProviderItem(
            itemIdentifier: identifier(for: path),
            parentItemIdentifier: identifier(for: parentPath(for: path)),
            filename: fileName(for: path),
            contentType: contentType,
            itemSize: size,
            created: Date(),
            modified: Date(),
            versionIdentifier: "optimistic:\(path):\(size?.stringValue ?? "dir")"
        )
    }

    private static func providerList() -> ProviderList {
        if let cached = cachedProviderList() {
            debugLog("provider list cached anchor=\(cached.anchor ?? "nil") entries=\(cached.entries.count)")
            return cached
        }
        var lastError: Error?
        for (attempt, delay) in ([0.0] + providerListRetryDelays).enumerated() {
            if delay > 0 {
                Thread.sleep(forTimeInterval: delay)
            }
            do {
                let data = try runIDrive(arguments: ["provider", "list"])
                let list = try JSONDecoder().decode(ProviderList.self, from: data)
                storeProviderListCache(list)
                debugLog("provider list ok anchor=\(list.anchor ?? "nil") entries=\(list.entries.count)")
                return list
            } catch {
                lastError = error
                debugLog("provider list attempt \(attempt + 1) failed: \(error)")
            }
        }
        if let lastError {
            debugLog("provider list failed after retries: \(lastError)")
        }
        return ProviderList(anchor: nil, entries: [])
    }

    private static func cachedProviderList() -> ProviderList? {
        providerListCacheLock.lock()
        defer { providerListCacheLock.unlock() }
        guard let cached = providerListCache,
              Date().timeIntervalSince(cached.loadedAt) <= providerListCacheTTL
        else {
            return nil
        }
        return cached.list
    }

    private static func storeProviderListCache(_ list: ProviderList) {
        providerListCacheLock.lock()
        providerListCache = (Date(), list)
        providerListCacheLock.unlock()
    }

    private static func invalidateProviderListCache() {
        providerListCacheLock.lock()
        providerListCache = nil
        providerListCacheLock.unlock()
    }

    private static func existingCachedContentsURL(for entry: ProviderEntry, anchor: String?) throws -> URL? {
        let url = contentCacheURL(for: entry, anchor: anchor)
        guard contentCacheFileMatches(url, entry: entry) else {
            return nil
        }
        return url
    }

    private static func cachedContentsURL(for entry: ProviderEntry, anchor: String?) throws -> URL {
        contentCacheLock.lock()
        defer { contentCacheLock.unlock() }

        if let cached = try existingCachedContentsURL(for: entry, anchor: anchor) {
            return cached
        }

        let root = contentCacheRootURL(for: anchor)
        try FileManager.default.createDirectory(
            at: root,
            withIntermediateDirectories: true
        )
        _ = try runIDrive(arguments: ["provider", "hydrate-cache", root.path], timeout: 60)
        pruneContentCacheDirectories(keeping: root)

        let url = contentCacheURL(for: entry, anchor: anchor)
        guard contentCacheFileMatches(url, entry: entry) else {
            throw providerError("private cache missing \(entry.path)")
        }
        return url
    }

    private static func contentCacheRootURL(for anchor: String?) -> URL {
        baseDirectory
            .appendingPathComponent(contentCacheDirectoryName, isDirectory: true)
            .appendingPathComponent(contentCacheKey(for: anchor), isDirectory: true)
    }

    private static func contentCacheURL(for entry: ProviderEntry, anchor: String?) -> URL {
        var url = contentCacheRootURL(for: anchor)
        for component in entry.path.split(separator: "/") {
            url.appendPathComponent(String(component), isDirectory: false)
        }
        return url
    }

    private static func contentCacheKey(for anchor: String?) -> String {
        let value = anchor ?? "unavailable"
        let encoded = Data(value.utf8)
            .base64EncodedString()
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "=", with: "")
        return encoded.isEmpty ? "empty" : encoded
    }

    private static func contentCacheFileMatches(_ url: URL, entry: ProviderEntry) -> Bool {
        guard FileManager.default.isReadableFile(atPath: url.path),
              let values = try? url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey]),
              values.isRegularFile == true,
              let size = values.fileSize
        else {
            return false
        }
        return UInt64(size) == entry.size
    }

    private static func pruneContentCacheDirectories(keeping keepURL: URL) {
        let root = baseDirectory.appendingPathComponent(contentCacheDirectoryName, isDirectory: true)
        guard let entries = try? FileManager.default.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: nil
        ) else {
            return
        }
        let keepPath = keepURL.standardizedFileURL.path
        for url in entries where url.standardizedFileURL.path != keepPath {
            try? FileManager.default.removeItem(at: url)
        }
    }

    private static func fileSize(at url: URL?) -> Int {
        guard let url,
              let values = try? url.resourceValues(forKeys: [.fileSizeKey]),
              let size = values.fileSize
        else {
            return 0
        }
        return size
    }

    private static func syncAnchor(for anchor: String?) -> NSFileProviderSyncAnchor {
        NSFileProviderSyncAnchor(rawValue: Data((anchor ?? "unavailable").utf8))
    }

    private static func snapshotURL() -> URL {
        baseDirectory.appendingPathComponent(snapshotFileName, isDirectory: false)
    }

    private static func runIDrive(arguments: [String], timeout: TimeInterval = 15) throws -> Data {
        guard let executable = idriveExecutable, !executable.isEmpty else {
            throw providerError("bundled idrive helper unavailable")
        }

        debugLog(
            "run idrive executable=\(executable) config=\(configDirectory.path) args=\(arguments.joined(separator: " "))"
        )
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = ["--config-dir", configDirectory.path] + arguments
        var environment = ProcessInfo.processInfo.environment
        environment["IRIS_DRIVE_CONFIG_DIR"] = configDirectory.path
        process.environment = environment

        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        var output = Data()
        var errorOutput = Data()
        let outputGroup = DispatchGroup()
        outputGroup.enter()
        DispatchQueue.global(qos: .utility).async {
            output = stdout.fileHandleForReading.readDataToEndOfFile()
            outputGroup.leave()
        }
        outputGroup.enter()
        DispatchQueue.global(qos: .utility).async {
            errorOutput = stderr.fileHandleForReading.readDataToEndOfFile()
            outputGroup.leave()
        }

        try process.run()
        let deadline = Date().addingTimeInterval(timeout)
        while process.isRunning && Date() < deadline {
            Thread.sleep(forTimeInterval: 0.05)
        }
        if process.isRunning {
            process.terminate()
            Thread.sleep(forTimeInterval: 0.2)
            if process.isRunning {
                kill(process.processIdentifier, SIGKILL)
            }
            outputGroup.wait()
            debugLog("idrive timed out args=\(arguments.joined(separator: " "))")
            throw providerError("idrive command timed out")
        }

        process.waitUntilExit()
        outputGroup.wait()

        if process.terminationStatus != 0 {
            let message = String(data: errorOutput, encoding: .utf8) ?? "idrive provider failed"
            debugLog(
                "idrive failed status=\(process.terminationStatus) stderr=\(message.trimmingCharacters(in: .whitespacesAndNewlines))"
            )
            throw providerError(message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        debugLog("idrive ok bytes=\(output.count)")
        return output
    }

    private static func emptyTemporaryFile() throws -> URL {
        let url = try temporaryDirectory()
            .appendingPathComponent(UUID().uuidString, isDirectory: false)
        FileManager.default.createFile(atPath: url.path, contents: Data())
        return url
    }

    private static func temporaryDirectory() throws -> URL {
        let directory = baseDirectory.appendingPathComponent(tempDirectoryName, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        return directory
    }

    private static func joinedPath(parent: String, name: String) -> String {
        let cleanName = name.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if parent.isEmpty {
            return cleanName
        }
        return "\(parent)/\(cleanName)"
    }

    private static func parentPath(for path: String) -> String {
        let value = path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard let slash = value.lastIndex(of: "/") else {
            return ""
        }
        return String(value[..<slash])
    }

    private static func fileName(for path: String) -> String {
        let value = path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard let slash = value.lastIndex(of: "/") else {
            return value
        }
        return String(value[value.index(after: slash)...])
    }

    private static func isSafeRelativePath(_ path: String) -> Bool {
        !path.isEmpty
            && !path.hasPrefix("/")
            && !path.split(separator: "/").contains("..")
    }

    private static func appGroupApplicationSupportDirectory() -> URL {
        IrisDriveAppGroup.applicationSupportDirectory(
            teamIdentifier: currentProcessTeamIdentifier()
        )
    }

    private static func currentProcessTeamIdentifier() -> String? {
        IrisDriveCodeSigning.currentTeamIdentifier()
    }

    private static func providerError(_ message: String) -> NSError {
        NSError(
            domain: NSFileProviderErrorDomain,
            code: NSFileProviderError.serverUnreachable.rawValue,
            userInfo: [NSLocalizedDescriptionKey: message]
        )
    }

}
