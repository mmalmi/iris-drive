import Foundation
import FileProvider
import ImageIO
import UniformTypeIdentifiers

enum FileProviderStorage {
    private static let appGroupIdentifier = "group.to.iris.drive"
    private static let domainIdentifier = NSFileProviderDomainIdentifier("main")
    private static let domainDisplayName = "Iris Drive"
    private static let storageDirectoryName = "IrisDrive"
    private static let providerSnapshotFileName = "ios-provider-snapshot.json"
    private static let debugLogFileName = "ios-fileprovider-extension.log"
    private static let pathPrefix = "path:"
    private static let tempDirectoryName = "FileProviderTmp"
    private static let minDisplayUnixSeconds: Int64 = 946_684_800

    private struct ProviderState: Decodable {
        var anchor: String
        var entries: [ProviderEntry]
    }

    private struct ProviderEntry: Decodable {
        var path: String
        var parentPath: String
        var displayName: String
        var kind: String
        var size: UInt64
        var version: String?
        var modifiedAt: Int64?

        enum CodingKeys: String, CodingKey {
            case path
            case parentPath = "parent_path"
            case displayName = "display_name"
            case kind
            case size
            case version
            case modifiedAt = "modified_at"
        }
    }

    private struct ProviderList: Decodable {
        var anchor: String?
        var entries: [ProviderEntry]
        var error: String?
    }

    private struct ProviderSnapshot: Codable {
        let anchor: String
        let identifiers: [String]
    }

    static var baseDirectory: URL {
        guard let shared = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) else {
            fatalError("Iris Drive app group is unavailable")
        }
        return shared.appendingPathComponent(storageDirectoryName, isDirectory: true)
    }

    static func debugLog(_ message: String) {
        NSLog("Iris Drive iOS FileProvider \(message)")
        let line = "\(ISO8601DateFormatter().string(from: Date())) \(message)\n"
        guard let data = line.data(using: .utf8) else { return }
        do {
            let url = baseDirectory.appendingPathComponent(debugLogFileName)
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
            NSLog("Iris Drive iOS FileProvider debug log write failed: \(error)")
        }
    }

    static func item(for identifier: NSFileProviderItemIdentifier) -> FileProviderItem? {
        let state = loadStateForEnumeration()
        if identifier == .rootContainer || identifier == .workingSet {
            return .root(anchor: state.anchor, modified: stateModifiedDate(state))
        }
        if identifier == .trashContainer {
            return .trash(anchor: state.anchor, modified: stateModifiedDate(state))
        }
        guard let path = path(for: identifier),
              let entry = state.entries.first(where: { $0.path == path })
        else { return nil }
        return item(for: entry, anchor: state.anchor)
    }

    static func path(for identifier: NSFileProviderItemIdentifier) -> String? {
        if identifier == .rootContainer || identifier == .workingSet {
            return ""
        }
        let raw = identifier.rawValue
        guard raw.hasPrefix(pathPrefix) else { return nil }
        let encoded = String(raw.dropFirst(pathPrefix.count))
        guard let data = Data(base64Encoded: base64DecodeString(encoded)),
              let relative = String(data: data, encoding: .utf8)
        else {
            return nil
        }
        let normalized = IrisDriveNativeProvider.normalizePath(path: relative)
        return normalized.error.isEmpty && !normalized.path.isEmpty ? normalized.path : nil
    }

    static func identifier(for path: String) -> NSFileProviderItemIdentifier {
        if path.isEmpty {
            return .rootContainer
        }
        return NSFileProviderItemIdentifier("\(pathPrefix)\(base64EncodeString(Data(path.utf8)))")
    }

    static func children(of containerIdentifier: NSFileProviderItemIdentifier) -> [FileProviderItem] {
        if containerIdentifier == .trashContainer {
            return []
        }
        guard let parent = path(for: containerIdentifier) else { return [] }
        let state = loadStateForEnumeration()
        return state.entries
            .filter { $0.parentPath == parent }
            .sorted { $0.path.localizedStandardCompare($1.path) == .orderedAscending }
            .map { item(for: $0, anchor: state.anchor) }
    }

    static func allItemsAndAnchor() -> (items: [FileProviderItem], anchor: NSFileProviderSyncAnchor) {
        let state = loadStateForEnumeration()
        let modified = stateModifiedDate(state)
        var items = [
            FileProviderItem.root(anchor: state.anchor, modified: modified),
            FileProviderItem.trash(anchor: state.anchor, modified: modified),
        ]
        items.append(contentsOf: state.entries.map { item(for: $0, anchor: state.anchor) })
        return (items, syncAnchor(for: state.anchor))
    }

    static func storedSnapshotIdentifiers() -> Set<String> {
        Set(storedSnapshot()?.identifiers ?? [])
    }

    static func currentProviderAnchor() -> NSFileProviderSyncAnchor {
        syncAnchor(for: storedSnapshot()?.anchor ?? "bootstrap")
    }

    static func recordSnapshot(items: [FileProviderItem], anchor: NSFileProviderSyncAnchor) {
        do {
            try FileManager.default.createDirectory(at: baseDirectory, withIntermediateDirectories: true)
            let snapshot = ProviderSnapshot(
                anchor: String(data: anchor.rawValue, encoding: .utf8) ?? "unavailable",
                identifiers: items.map(\.itemIdentifier.rawValue).sorted()
            )
            try JSONEncoder().encode(snapshot).write(to: snapshotURL())
        } catch {
            debugLog("snapshot write failed: \(error)")
        }
    }

    static func createItem(template: NSFileProviderItem, contents: URL?) throws -> FileProviderItem {
        let parent = path(for: template.parentItemIdentifier) ?? ""
        let destination = try resolvedPath(parent: parent, name: template.filename)
        if (template.contentType ?? .data).conforms(to: .folder) {
            try runProviderMutation(
                IrisDriveNativeProvider.mkdir(dataDir: baseDirectory.path, path: destination.path)
            )
        } else {
            let source: URL
            if let contents {
                source = contents
            } else {
                source = try emptyTemporaryFile()
            }
            try runProviderMutation(
                IrisDriveNativeProvider.write(
                    dataDir: baseDirectory.path,
                    path: destination.path,
                    sourcePath: source.path
                )
            )
        }
        signalProviderChanged()
        return optimisticItem(for: destination, template: template, contents: contents)
    }

    static func importSharedFile(
        named displayName: String,
        contentType: UTType,
        contents: Data
    ) throws {
        let source = try temporaryDirectory()
            .appendingPathComponent(UUID().uuidString, isDirectory: false)
        try contents.write(to: source)
        let result = IrisDriveNativeProvider.importSharedFile(
            dataDir: baseDirectory.path,
            displayName: displayName,
            sourcePath: source.path
        )
        try runProviderMutation(result)
        debugLog("share import saved name=\(displayName) bytes=\(contents.count) result=\(result)")
        signalProviderChanged()
    }

    static func modifyItem(
        _ item: NSFileProviderItem,
        changedFields: NSFileProviderItemFields,
        contents: URL?
    ) throws -> FileProviderItem? {
        guard let original = path(for: item.itemIdentifier), !original.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier)
        }
        guard let originalEntry = loadStateForEnumeration().entries.first(where: { $0.path == original }) else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier)
        }
        if changedFields.contains(.parentItemIdentifier),
           item.parentItemIdentifier == .trashContainer {
            try deleteItem(identifier: item.itemIdentifier)
            return nil
        }
        let parent = changedFields.contains(.parentItemIdentifier)
            ? (path(for: item.parentItemIdentifier) ?? "")
            : originalEntry.parentPath
        let name = changedFields.contains(.filename) ? item.filename : originalEntry.displayName
        let destination = try resolvedPath(parent: parent, name: name, excluding: original)
        if destination.path != original {
            try runProviderMutation(
                IrisDriveNativeProvider.rename(
                    dataDir: baseDirectory.path,
                    oldPath: original,
                    newPath: destination.path
                )
            )
        }
        if let contents, !(item.contentType ?? .data).conforms(to: .folder) {
            try runProviderMutation(
                IrisDriveNativeProvider.write(
                    dataDir: baseDirectory.path,
                    path: destination.path,
                    sourcePath: contents.path
                )
            )
        }
        signalProviderChanged()
        return optimisticItem(for: destination, template: item, contents: contents)
    }

    static func deleteItem(identifier: NSFileProviderItemIdentifier) throws {
        guard let path = path(for: identifier), !path.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        try runProviderMutation(
            IrisDriveNativeProvider.delete(dataDir: baseDirectory.path, path: path)
        )
        signalProviderChanged()
    }

    static func contentsURL(for identifier: NSFileProviderItemIdentifier) throws -> URL {
        guard let path = path(for: identifier), !path.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        let backed = try loadProviderBackedState()
        guard let entry = backed.entries.first(where: { $0.path == path && $0.kind != "directory" })
        else {
            throw providerError("content unavailable")
        }
        let directory = try temporaryDirectory()
        let output = directory
            .appendingPathComponent(UUID().uuidString, isDirectory: false)
            .appendingPathExtension((path as NSString).pathExtension)
        try runProviderMutation(
            IrisDriveNativeProvider.read(
                dataDir: baseDirectory.path,
                path: entry.path,
                outputPath: output.path
            )
        )
        return output
    }

    static func thumbnailData(
        for identifier: NSFileProviderItemIdentifier,
        requestedSize size: CGSize
    ) throws -> Data? {
        guard let item = item(for: identifier) else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        guard item.contentType.conforms(to: .image) else {
            debugLog("thumbnail unsupported type identifier=\(identifier.rawValue)")
            return nil
        }

        let url = try contentsURL(for: identifier)
        let maxPixelSize = thumbnailMaxPixelSize(for: size)
        guard let source = CGImageSourceCreateWithURL(
            url as CFURL,
            [kCGImageSourceShouldCache: false] as CFDictionary
        ) else {
            debugLog("thumbnail source unavailable identifier=\(identifier.rawValue)")
            return nil
        }

        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageIfAbsent: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(
            source,
            0,
            options as CFDictionary
        ) else {
            debugLog("thumbnail image unavailable identifier=\(identifier.rawValue)")
            return nil
        }

        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            data,
            UTType.png.identifier as CFString,
            1,
            nil
        ) else {
            throw providerError("thumbnail destination unavailable")
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw providerError("thumbnail encode failed")
        }
        debugLog("thumbnail generated identifier=\(identifier.rawValue) bytes=\(data.length)")
        return data as Data
    }

    private static func loadStateForEnumeration() -> ProviderState {
        do {
            return try loadProviderBackedState()
        } catch {
            debugLog("provider list unavailable: \(error)")
            return ProviderState(anchor: "bootstrap", entries: [])
        }
    }

    private static func loadProviderBackedState() throws -> ProviderState {
        let json = IrisDriveNativeProvider.list(dataDir: baseDirectory.path)
        guard let data = json.data(using: .utf8) else {
            throw providerError("provider returned invalid text")
        }
        let list = try JSONDecoder().decode(ProviderList.self, from: data)
        if let error = list.error, !error.isEmpty {
            throw providerError(error)
        }
        guard let anchor = list.anchor, !anchor.isEmpty else {
            throw providerError("provider root unavailable")
        }
        return ProviderState(anchor: anchor, entries: list.entries)
    }

    private static func item(for entry: ProviderEntry, anchor: String) -> FileProviderItem {
        let isDirectory = entry.kind == "directory"
        let type = isDirectory
            ? UTType.folder
            : UTType(filenameExtension: (entry.displayName as NSString).pathExtension) ?? .data
        return FileProviderItem(
            itemIdentifier: identifier(for: entry.path),
            parentItemIdentifier: identifier(for: entry.parentPath),
            filename: entry.displayName,
            contentType: type,
            itemSize: isDirectory ? nil : NSNumber(value: entry.size),
            created: displayDate(from: entry.modifiedAt),
            modified: displayDate(from: entry.modifiedAt),
            versionIdentifier: "\(anchor):\(entry.version ?? "unknown"):\(entry.path):\(entry.size):\(entry.modifiedAt ?? 0)"
        )
    }

    private static func optimisticItem(
        for resolved: NativeProviderResolvedPath,
        template: NSFileProviderItem,
        contents: URL?
    ) -> FileProviderItem {
        let isDirectory = (template.contentType ?? .data).conforms(to: .folder)
        let contentType = isDirectory
            ? UTType.folder
            : UTType(filenameExtension: (resolved.displayName as NSString).pathExtension) ?? .data
        return FileProviderItem(
            itemIdentifier: identifier(for: resolved.path),
            parentItemIdentifier: identifier(for: resolved.parentPath),
            filename: resolved.displayName,
            contentType: contentType,
            itemSize: isDirectory ? nil : NSNumber(value: fileSize(at: contents)),
            created: Date(),
            modified: Date(),
            versionIdentifier: "optimistic:\(resolved.path):\(UUID().uuidString)"
        )
    }

    private static func snapshotURL() -> URL {
        baseDirectory.appendingPathComponent(providerSnapshotFileName, isDirectory: false)
    }

    private static func storedSnapshot() -> ProviderSnapshot? {
        guard let data = try? Data(contentsOf: snapshotURL()) else { return nil }
        return try? JSONDecoder().decode(ProviderSnapshot.self, from: data)
    }

    private static func base64EncodeString(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    private static func base64DecodeString(_ value: String) -> String {
        var normalized = value
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let padding = (4 - normalized.count % 4) % 4
        if padding > 0 {
            normalized += String(repeating: "=", count: padding)
        }
        return normalized
    }

    private static func temporaryDirectory() throws -> URL {
        let directory = baseDirectory.appendingPathComponent(tempDirectoryName, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    private static func emptyTemporaryFile() throws -> URL {
        let url = try temporaryDirectory()
            .appendingPathComponent(UUID().uuidString, isDirectory: false)
        FileManager.default.createFile(atPath: url.path, contents: Data())
        return url
    }

    private static func fileSize(at url: URL?) -> UInt64 {
        guard let url,
              let values = try? url.resourceValues(forKeys: [.fileSizeKey]),
              let size = values.fileSize
        else {
            return 0
        }
        return UInt64(size)
    }

    private static func thumbnailMaxPixelSize(for size: CGSize) -> Int {
        max(64, Int(ceil(max(size.width, size.height))))
    }

    private static func stateModifiedDate(_ state: ProviderState) -> Date? {
        state.entries
            .compactMap { displayDate(from: $0.modifiedAt) }
            .max()
    }

    private static func displayDate(from unixSeconds: Int64?) -> Date? {
        guard let unixSeconds, unixSeconds >= minDisplayUnixSeconds else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(unixSeconds))
    }

    private static func syncAnchor(for anchor: String) -> NSFileProviderSyncAnchor {
        NSFileProviderSyncAnchor(rawValue: Data(anchor.utf8))
    }

    private static func runProviderMutation(_ json: String) throws {
        guard let data = json.data(using: .utf8) else {
            throw providerError("provider returned invalid text")
        }
        if let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
           let error = object["error"] as? String,
           !error.isEmpty {
            throw providerError(error)
        }
    }

    private static func signalProviderChanged() {
        guard let manager = NSFileProviderManager(
            for: NSFileProviderDomain(identifier: domainIdentifier, displayName: domainDisplayName)
        ) else {
            debugLog("signal provider failed: manager unavailable")
            return
        }
        manager.signalEnumerator(for: .rootContainer) { error in
            if let error {
                debugLog("signal root failed: \(error)")
            }
        }
        manager.signalEnumerator(for: .workingSet) { error in
            if let error {
                debugLog("signal working set failed: \(error)")
            }
        }
    }

    private static func resolvedPath(
        parent: String,
        name: String,
        excluding: String = ""
    ) throws -> NativeProviderResolvedPath {
        let resolved = IrisDriveNativeProvider.resolvePath(
            dataDir: baseDirectory.path,
            parentPath: parent,
            displayName: name,
            excludingPath: excluding
        )
        if !resolved.error.isEmpty {
            throw providerError(resolved.error)
        }
        guard !resolved.path.isEmpty else {
            throw providerError("provider path resolver returned no path")
        }
        return resolved
    }

    private static func providerError(_ message: String) -> NSError {
        NSError(
            domain: NSFileProviderErrorDomain,
            code: NSFileProviderError.serverUnreachable.rawValue,
            userInfo: [NSLocalizedDescriptionKey: message]
        )
    }
}
