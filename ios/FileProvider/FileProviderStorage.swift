import Foundation
import FileProvider
import UniformTypeIdentifiers

enum FileProviderStorage {
    private static let appGroupIdentifier = "group.to.iris.drive"
    private static let storageDirectoryName = "IrisDrive"
    private static let providerSnapshotFileName = "ios-provider-snapshot.json"
    private static let debugLogFileName = "ios-fileprovider-extension.log"
    private static let pathPrefix = "path:"
    private static let tempDirectoryName = "FileProviderTmp"

    private struct ProviderState: Decodable {
        var anchor: String
        var entries: [ProviderEntry]
    }

    private struct ProviderEntry: Decodable {
        var path: String
        var kind: String
        var size: UInt64
        var version: String?
        var modifiedAt: Int64?

        enum CodingKeys: String, CodingKey {
            case path
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
        return NSFileProviderItemIdentifier("\(pathPrefix)\(Data(path.utf8).base64EncodedString())")
    }

    static func children(of containerIdentifier: NSFileProviderItemIdentifier) -> [FileProviderItem] {
        if containerIdentifier == .trashContainer {
            return []
        }
        guard let parent = path(for: containerIdentifier) else { return [] }
        let state = loadStateForEnumeration()
        return state.entries
            .filter { parentPath(for: $0.path) == parent }
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
        guard let data = try? Data(contentsOf: snapshotURL()),
              let snapshot = try? JSONDecoder().decode(ProviderSnapshot.self, from: data)
        else {
            return []
        }
        return Set(snapshot.identifiers)
    }

    static func currentProviderAnchor() -> NSFileProviderSyncAnchor {
        syncAnchor(for: loadStateForEnumeration().anchor)
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
        let destination = joinedPath(parent: parent, name: template.filename)
        if (template.contentType ?? .data).conforms(to: .folder) {
            try runProviderMutation(
                IrisDriveNativeProvider.mkdir(dataDir: baseDirectory.path, path: destination)
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
                    path: destination,
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
        try runProviderMutation(
            IrisDriveNativeProvider.importSharedFile(
                dataDir: baseDirectory.path,
                displayName: sanitizedFileName(displayName, contentType: contentType),
                sourcePath: source.path
            )
        )
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
        if changedFields.contains(.parentItemIdentifier),
           item.parentItemIdentifier == .trashContainer {
            try deleteItem(identifier: item.itemIdentifier)
            return nil
        }
        let parent = changedFields.contains(.parentItemIdentifier)
            ? (path(for: item.parentItemIdentifier) ?? "")
            : parentPath(for: original)
        let name = changedFields.contains(.filename) ? item.filename : fileName(for: original)
        let destination = joinedPath(parent: parent, name: name)
        if destination != original {
            try runProviderMutation(
                IrisDriveNativeProvider.rename(
                    dataDir: baseDirectory.path,
                    oldPath: original,
                    newPath: destination
                )
            )
        }
        if let contents, !(item.contentType ?? .data).conforms(to: .folder) {
            try runProviderMutation(
                IrisDriveNativeProvider.write(
                    dataDir: baseDirectory.path,
                    path: destination,
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
            : UTType(filenameExtension: (entry.path as NSString).pathExtension) ?? .data
        return FileProviderItem(
            itemIdentifier: identifier(for: entry.path),
            parentItemIdentifier: identifier(for: parentPath(for: entry.path)),
            filename: fileName(for: entry.path),
            contentType: type,
            itemSize: isDirectory ? nil : NSNumber(value: entry.size),
            created: displayDate(from: entry.modifiedAt) ?? providerReferenceDate(),
            modified: displayDate(from: entry.modifiedAt) ?? providerReferenceDate(),
            versionIdentifier: "\(anchor):\(entry.version ?? "unknown"):\(entry.path):\(entry.size):\(entry.modifiedAt ?? 0)"
        )
    }

    private static func optimisticItem(
        for path: String,
        template: NSFileProviderItem,
        contents: URL?
    ) -> FileProviderItem {
        let isDirectory = (template.contentType ?? .data).conforms(to: .folder)
        let contentType = isDirectory
            ? UTType.folder
            : UTType(filenameExtension: (path as NSString).pathExtension) ?? .data
        return FileProviderItem(
            itemIdentifier: identifier(for: path),
            parentItemIdentifier: identifier(for: parentPath(for: path)),
            filename: fileName(for: path),
            contentType: contentType,
            itemSize: isDirectory ? nil : NSNumber(value: fileSize(at: contents)),
            created: Date(),
            modified: Date(),
            versionIdentifier: "optimistic:\(path):\(UUID().uuidString)"
        )
    }

    private static func snapshotURL() -> URL {
        baseDirectory.appendingPathComponent(providerSnapshotFileName, isDirectory: false)
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

    private static func stateModifiedDate(_ state: ProviderState) -> Date {
        state.entries
            .compactMap { displayDate(from: $0.modifiedAt) }
            .max()
            ?? providerReferenceDate()
    }

    private static func displayDate(from unixSeconds: Int64?) -> Date? {
        guard let unixSeconds, unixSeconds > 0 else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(unixSeconds))
    }

    private static func providerReferenceDate() -> Date {
        let values = try? baseDirectory.resourceValues(
            forKeys: [.contentModificationDateKey, .creationDateKey]
        )
        return values?.contentModificationDate ?? values?.creationDate ?? Date()
    }

    private static func syncAnchor(for anchor: String) -> NSFileProviderSyncAnchor {
        NSFileProviderSyncAnchor(rawValue: Data(anchor.utf8))
    }

    private static func joinedPath(parent: String, name: String) -> String {
        let cleanName = name.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if parent.isEmpty {
            return cleanName
        }
        return "\(parent)/\(cleanName)"
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
        NSFileProviderManager.default.signalEnumerator(for: .rootContainer) { error in
            if let error {
                debugLog("signal root failed: \(error)")
            }
        }
        NSFileProviderManager.default.signalEnumerator(for: .workingSet) { error in
            if let error {
                debugLog("signal working set failed: \(error)")
            }
        }
    }

    private static func sanitizedFileName(_ displayName: String, contentType: UTType) -> String {
        let separators = CharacterSet(charactersIn: "/:")
        let components = displayName
            .components(separatedBy: separators)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && $0 != "." && $0 != ".." }
        var name = components.joined(separator: "_")
        if name.isEmpty {
            name = "Shared file"
        }
        if (name as NSString).pathExtension.isEmpty,
           let preferredExtension = contentType.preferredFilenameExtension {
            name += ".\(preferredExtension)"
        }
        return name
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

    private static func providerError(_ message: String) -> NSError {
        NSError(
            domain: NSFileProviderErrorDomain,
            code: NSFileProviderError.serverUnreachable.rawValue,
            userInfo: [NSLocalizedDescriptionKey: message]
        )
    }
}
