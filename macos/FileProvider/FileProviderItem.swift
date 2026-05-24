import Foundation
import FileProvider
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
            filename: "Iris Drive",
            contentType: .folder,
            versionIdentifier: "root:\(anchor ?? "initial")"
        )
    }
}

enum FileProviderStorage {
    private static let runtimeFileName = "fileprovider-runtime.json"
    private static let pathPrefix = "path:"
    private static let tempDirectoryName = "FileProviderTmp"
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
        if let configuredRuntime {
            return configuredRuntime
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

    private static var runtimeDirectory: URL? {
        runtimeDirectories.first { directory in
            FileManager.default.fileExists(
                atPath: directory.appendingPathComponent(runtimeFileName).path
            )
        }
    }

    private static var runtimeDirectories: [URL] {
        var directories = [URL]()
        directories.append(fallbackApplicationSupportDirectory())

        var seen = Set<String>()
        return directories.filter { directory in
            seen.insert(directory.standardizedFileURL.path).inserted
        }
    }

    static var idriveExecutable: String? {
        if let configured = runtime?.idriveExecutable, !configured.isEmpty {
            return configured
        }
        let contents = Bundle.main.bundleURL
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let bundled = contents
            .appendingPathComponent("MacOS", isDirectory: true)
            .appendingPathComponent("idrive")
        guard FileManager.default.isExecutableFile(atPath: bundled.path) else { return nil }
        return bundled.path
    }

    static func item(for identifier: NSFileProviderItemIdentifier) -> FileProviderItem? {
        if identifier == .rootContainer || identifier == .workingSet {
            return .root(anchor: providerList().anchor)
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
        guard let parent = path(for: containerIdentifier) else { return [] }
        let list = providerList()
        return list.entries
            .filter { parentPath(for: $0.path) == parent }
            .sorted { $0.path.localizedStandardCompare($1.path) == .orderedAscending }
            .map { item(for: $0, anchor: list.anchor) }
    }

    static func allItems() -> [FileProviderItem] {
        let list = providerList()
        return list.entries.map { item(for: $0, anchor: list.anchor) }
    }

    static func createItem(
        template: NSFileProviderItem,
        contents: URL?
    ) throws -> FileProviderItem {
        let parent = path(for: template.parentItemIdentifier) ?? ""
        let destination = joinedPath(parent: parent, name: template.filename)
        NSLog("Iris Drive FileProvider create path=\(destination)")
        if (template.contentType ?? .data).conforms(to: .folder) {
            _ = try runIDrive(arguments: ["provider", "mkdir", destination])
        } else if let contents {
            _ = try runIDrive(arguments: ["provider", "write", destination, contents.path])
        } else {
            let empty = try emptyTemporaryFile()
            _ = try runIDrive(arguments: ["provider", "write", destination, empty.path])
        }
        guard let item = item(for: identifier(for: destination)) else {
            throw NSError.fileProviderErrorForNonExistentItem(
                withIdentifier: template.itemIdentifier
            )
        }
        NSLog("Iris Drive FileProvider created path=\(destination)")
        return item
    }

    static func modifyItem(
        _ item: NSFileProviderItem,
        changedFields: NSFileProviderItemFields,
        contents: URL?
    ) throws -> FileProviderItem {
        guard let original = path(for: item.itemIdentifier), !original.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier)
        }
        NSLog("Iris Drive FileProvider modify path=\(original)")
        var destination = original
        if changedFields.contains(.filename), item.filename != fileName(for: original) {
            destination = joinedPath(parent: parentPath(for: original), name: item.filename)
            _ = try runIDrive(arguments: ["provider", "rename", original, destination])
        }
        if let contents, !(item.contentType ?? .data).conforms(to: .folder) {
            _ = try runIDrive(arguments: ["provider", "write", destination, contents.path])
        }
        guard let updated = self.item(for: identifier(for: destination)) else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier)
        }
        NSLog("Iris Drive FileProvider modified path=\(destination)")
        return updated
    }

    static func deleteItem(identifier: NSFileProviderItemIdentifier) throws {
        guard let path = path(for: identifier), !path.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        NSLog("Iris Drive FileProvider delete path=\(path)")
        _ = try runIDrive(arguments: ["provider", "delete", path])
    }

    static func contentsURL(for identifier: NSFileProviderItemIdentifier) throws -> URL {
        guard let path = path(for: identifier), !path.isEmpty else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        let directory = try temporaryDirectory()
        let output = directory
            .appendingPathComponent(UUID().uuidString, isDirectory: false)
            .appendingPathExtension((path as NSString).pathExtension)
        NSLog("Iris Drive FileProvider fetch contents path=\(path)")
        _ = try runIDrive(arguments: ["provider", "read", path, output.path])
        return output
    }

    static func currentAnchor() -> NSFileProviderSyncAnchor {
        let anchor = providerList().anchor ?? "unavailable"
        return NSFileProviderSyncAnchor(rawValue: Data(anchor.utf8))
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

    private static func providerList() -> ProviderList {
        do {
            let data = try runIDrive(arguments: ["provider", "list"])
            return try JSONDecoder().decode(ProviderList.self, from: data)
        } catch {
            NSLog("Iris Drive FileProvider provider list failed: \(error)")
            return ProviderList(anchor: nil, entries: [])
        }
    }

    private static func runIDrive(arguments: [String]) throws -> Data {
        guard let executable = idriveExecutable, !executable.isEmpty else {
            throw providerError("bundled idrive helper unavailable")
        }

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
        try process.run()
        process.waitUntilExit()

        let output = stdout.fileHandleForReading.readDataToEndOfFile()
        let errorOutput = stderr.fileHandleForReading.readDataToEndOfFile()
        if process.terminationStatus != 0 {
            let message = String(data: errorOutput, encoding: .utf8) ?? "idrive provider failed"
            throw providerError(message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
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

    private static func fallbackApplicationSupportDirectory() -> URL {
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.homeDirectoryForCurrentUser
        return base.appendingPathComponent("Iris Drive", isDirectory: true)
    }

    private static func providerError(_ message: String) -> NSError {
        NSError(
            domain: NSFileProviderErrorDomain,
            code: NSFileProviderError.serverUnreachable.rawValue,
            userInfo: [NSLocalizedDescriptionKey: message]
        )
    }

}
