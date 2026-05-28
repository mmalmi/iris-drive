import Foundation
import FileProvider
import UniformTypeIdentifiers

enum FileProviderStorage {
    private static let appGroupIdentifier = "group.to.iris.drive"
    private static let providerStateFileName = "ios-provider-state.json"
    private static let providerSnapshotFileName = "ios-provider-snapshot.json"
    private static let debugLogFileName = "ios-fileprovider-extension.log"
    private static let pathPrefix = "path:"
    private static let tempDirectoryName = "FileProviderTmp"
    private static let lock = NSLock()

    private struct ProviderState: Codable {
        var anchor: String
        var entries: [ProviderEntry]

        static var empty: ProviderState {
            ProviderState(anchor: "empty", entries: [])
        }
    }

    private struct ProviderEntry: Codable {
        var path: String
        var kind: String
        var size: UInt64
        var version: String
        var contentBase64: String?
        var createdAt: Date
        var modifiedAt: Date
    }

    private struct ProviderSnapshot: Codable {
        let anchor: String
        let identifiers: [String]
    }

    static var baseDirectory: URL {
        if let shared = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) {
            return shared.appendingPathComponent("Iris Drive", isDirectory: true)
        }
        let support = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.temporaryDirectory
        return support.appendingPathComponent("Iris Drive", isDirectory: true)
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
        if identifier == .rootContainer || identifier == .workingSet {
            return .root(anchor: loadState().anchor)
        }
        if identifier == .trashContainer {
            return .trash(anchor: loadState().anchor)
        }
        let state = loadState()
        guard let path = path(for: identifier),
              let entry = state.entries.first(where: { $0.path == path })
        else {
            return nil
        }
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
        let state = loadState()
        return state.entries
            .filter { parentPath(for: $0.path) == parent }
            .sorted { $0.path.localizedStandardCompare($1.path) == .orderedAscending }
            .map { item(for: $0, anchor: state.anchor) }
    }

    static func allItemsAndAnchor() -> (items: [FileProviderItem], anchor: NSFileProviderSyncAnchor) {
        let state = loadState()
        var items = [FileProviderItem.root(anchor: state.anchor), FileProviderItem.trash(anchor: state.anchor)]
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
        syncAnchor(for: loadState().anchor)
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
        try mutateState { state in
            let parent = path(for: template.parentItemIdentifier) ?? ""
            let destination = joinedPath(parent: parent, name: template.filename)
            state.entries.removeAll { $0.path == destination }
            let entry = try entry(for: destination, template: template, contents: contents)
            state.entries.append(entry)
            state.anchor = newAnchor()
            return item(for: entry, anchor: state.anchor)
        }
    }

    static func importSharedFile(
        named displayName: String,
        contentType: UTType,
        contents: Data
    ) throws {
        try mutateState { state in
            let destination = uniquePath(
                in: state,
                parent: "",
                name: sanitizedFileName(displayName, contentType: contentType)
            )
            let now = Date()
            state.entries.append(
                ProviderEntry(
                    path: destination,
                    kind: "file",
                    size: UInt64(contents.count),
                    version: newAnchor(),
                    contentBase64: contents.base64EncodedString(),
                    createdAt: now,
                    modifiedAt: now
                )
            )
            state.anchor = newAnchor()
        }
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
        return try mutateState { state in
            guard let index = state.entries.firstIndex(where: { $0.path == original }) else {
                throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier)
            }
            let parent = changedFields.contains(.parentItemIdentifier)
                ? (path(for: item.parentItemIdentifier) ?? "")
                : parentPath(for: original)
            let name = changedFields.contains(.filename) ? item.filename : fileName(for: original)
            let destination = joinedPath(parent: parent, name: name)
            var entry = state.entries[index]
            if entry.kind == "directory" && destination != original {
                renameChildren(in: &state, from: original, to: destination)
            }
            entry.path = destination
            entry.version = newAnchor()
            entry.modifiedAt = Date()
            if let contents, entry.kind != "directory" {
                let data = try Data(contentsOf: contents)
                entry.size = UInt64(data.count)
                entry.contentBase64 = data.base64EncodedString()
            }
            state.entries[index] = entry
            state.anchor = newAnchor()
            return self.item(for: entry, anchor: state.anchor)
        }
    }

    static func deleteItem(identifier: NSFileProviderItemIdentifier) throws {
        guard let path = path(for: identifier), !path.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        try mutateState { state in
            let prefix = "\(path)/"
            state.entries.removeAll { $0.path == path || $0.path.hasPrefix(prefix) }
            state.anchor = newAnchor()
            return ()
        }
    }

    static func contentsURL(for identifier: NSFileProviderItemIdentifier) throws -> URL {
        guard let path = path(for: identifier), !path.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        let state = loadState()
        guard let entry = state.entries.first(where: { $0.path == path && $0.kind != "directory" }),
              let contentBase64 = entry.contentBase64,
              let data = Data(base64Encoded: contentBase64)
        else {
            throw providerError("content unavailable")
        }
        let directory = try temporaryDirectory()
        let url = directory
            .appendingPathComponent(UUID().uuidString, isDirectory: false)
            .appendingPathExtension((path as NSString).pathExtension)
        try data.write(to: url)
        return url
    }

    private static func loadState() -> ProviderState {
        lock.lock()
        defer { lock.unlock() }
        return loadStateUnlocked()
    }

    private static func mutateState<T>(_ body: (inout ProviderState) throws -> T) throws -> T {
        lock.lock()
        defer { lock.unlock() }
        var state = loadStateUnlocked()
        let value = try body(&state)
        try saveStateUnlocked(state)
        return value
    }

    private static func loadStateUnlocked() -> ProviderState {
        guard let data = try? Data(contentsOf: stateURL()) else {
            return .empty
        }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        guard let state = try? decoder.decode(ProviderState.self, from: data) else {
            return .empty
        }
        return state
    }

    private static func saveStateUnlocked(_ state: ProviderState) throws {
        try FileManager.default.createDirectory(at: baseDirectory, withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        try encoder.encode(state).write(to: stateURL(), options: [.atomic])
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
            created: entry.createdAt,
            modified: entry.modifiedAt,
            versionIdentifier: "\(anchor):\(entry.version):\(entry.path):\(entry.size)"
        )
    }

    private static func entry(
        for path: String,
        template: NSFileProviderItem,
        contents: URL?
    ) throws -> ProviderEntry {
        let isDirectory = (template.contentType ?? .data).conforms(to: .folder)
        let data = try contents.map { try Data(contentsOf: $0) } ?? Data()
        let now = Date()
        return ProviderEntry(
            path: path,
            kind: isDirectory ? "directory" : "file",
            size: isDirectory ? 0 : UInt64(data.count),
            version: newAnchor(),
            contentBase64: isDirectory ? nil : data.base64EncodedString(),
            createdAt: now,
            modifiedAt: now
        )
    }

    private static func renameChildren(in state: inout ProviderState, from: String, to: String) {
        let prefix = "\(from)/"
        for index in state.entries.indices where state.entries[index].path.hasPrefix(prefix) {
            let suffix = state.entries[index].path.dropFirst(prefix.count)
            state.entries[index].path = "\(to)/\(suffix)"
            state.entries[index].version = newAnchor()
            state.entries[index].modifiedAt = Date()
        }
    }

    private static func stateURL() -> URL {
        baseDirectory.appendingPathComponent(providerStateFileName, isDirectory: false)
    }

    private static func snapshotURL() -> URL {
        baseDirectory.appendingPathComponent(providerSnapshotFileName, isDirectory: false)
    }

    private static func temporaryDirectory() throws -> URL {
        let directory = baseDirectory.appendingPathComponent(tempDirectoryName, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    private static func syncAnchor(for anchor: String) -> NSFileProviderSyncAnchor {
        NSFileProviderSyncAnchor(rawValue: Data(anchor.utf8))
    }

    private static func newAnchor() -> String {
        "\(Int(Date().timeIntervalSince1970 * 1000))-\(UUID().uuidString)"
    }

    private static func joinedPath(parent: String, name: String) -> String {
        let cleanName = name.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if parent.isEmpty {
            return cleanName
        }
        return "\(parent)/\(cleanName)"
    }

    private static func uniquePath(in state: ProviderState, parent: String, name: String) -> String {
        let existing = Set(state.entries.map(\.path))
        var candidate = joinedPath(parent: parent, name: name)
        if !existing.contains(candidate) {
            return candidate
        }

        let nsName = name as NSString
        let extensionWithDot = nsName.pathExtension.isEmpty ? "" : ".\(nsName.pathExtension)"
        let basename = nsName.deletingPathExtension
        var index = 2
        while existing.contains(candidate) {
            candidate = joinedPath(parent: parent, name: "\(basename) (\(index))\(extensionWithDot)")
            index += 1
        }
        return candidate
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
