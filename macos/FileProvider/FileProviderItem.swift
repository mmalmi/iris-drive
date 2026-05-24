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

    init(
        itemIdentifier: NSFileProviderItemIdentifier,
        parentItemIdentifier: NSFileProviderItemIdentifier,
        filename: String,
        contentType: UTType,
        itemSize: NSNumber? = nil,
        created: Date? = nil,
        modified: Date? = nil
    ) {
        self.itemIdentifier = itemIdentifier
        self.parentItemIdentifier = parentItemIdentifier
        self.filename = filename
        self.contentType = contentType
        self.itemSize = itemSize
        self.created = created
        self.modified = modified
    }

    var itemVersion: NSFileProviderItemVersion {
        let version = Data("iris-drive-bootstrap-v1".utf8)
        return NSFileProviderItemVersion(contentVersion: version, metadataVersion: version)
    }

    var capabilities: NSFileProviderItemCapabilities {
        if contentType == .folder {
            return [
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
    static let root = FileProviderItem(
        itemIdentifier: .rootContainer,
        parentItemIdentifier: .rootContainer,
        filename: "Iris Drive",
        contentType: .folder
    )
}

enum FileProviderStorage {
    private static let appGroupIdentifier = "group.to.iris.drive"
    private static let runtimeFileName = "fileprovider-runtime.json"
    private static let pathPrefix = "path:"

    struct Runtime: Decodable {
        let configDirectory: String?
        let driveDirectory: String?
        let idriveExecutable: String?

        enum CodingKeys: String, CodingKey {
            case configDirectory = "config_dir"
            case driveDirectory = "drive_dir"
            case idriveExecutable = "idrive_executable"
        }
    }

    static var baseDirectory: URL {
        FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) ?? fallbackApplicationSupportDirectory()
    }

    static var driveRoot: URL {
        if let configured = runtime?.driveDirectory, !configured.isEmpty {
            return URL(fileURLWithPath: configured, isDirectory: true)
        }
        return baseDirectory.appendingPathComponent("Drive", isDirectory: true)
    }

    static var configDirectory: URL {
        if let configured = runtime?.configDirectory, !configured.isEmpty {
            return URL(fileURLWithPath: configured, isDirectory: true)
        }
        return baseDirectory.appendingPathComponent("Config", isDirectory: true)
    }

    static var runtime: Runtime? {
        let url = baseDirectory.appendingPathComponent(runtimeFileName)
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(Runtime.self, from: data)
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
        guard let url = url(for: identifier) else { return nil }
        return item(for: url, identifier: identifier)
    }

    static func url(for identifier: NSFileProviderItemIdentifier) -> URL? {
        if identifier == .rootContainer || identifier == .workingSet {
            return driveRoot
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
        return driveRoot.appendingPathComponent(relative)
    }

    static func identifier(for url: URL) -> NSFileProviderItemIdentifier? {
        guard let relative = relativePath(for: url) else { return nil }
        if relative.isEmpty {
            return .rootContainer
        }
        let encoded = Data(relative.utf8).base64EncodedString()
        return NSFileProviderItemIdentifier("\(pathPrefix)\(encoded)")
    }

    static func children(of containerIdentifier: NSFileProviderItemIdentifier) -> [FileProviderItem] {
        guard let directory = url(for: containerIdentifier) else { return [] }
        let urls = (try? FileManager.default.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: [
                .contentTypeKey,
                .creationDateKey,
                .contentModificationDateKey,
                .fileSizeKey,
                .isDirectoryKey,
            ],
            options: [.skipsHiddenFiles]
        )) ?? []
        return urls
            .sorted { $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending }
            .compactMap { url in
                guard let itemIdentifier = Self.identifier(for: url) else { return nil }
                return item(for: url, identifier: itemIdentifier)
            }
    }

    static func createItem(
        template: NSFileProviderItem,
        contents: URL?
    ) throws -> FileProviderItem {
        let parent = url(for: template.parentItemIdentifier) ?? driveRoot
        try FileManager.default.createDirectory(at: parent, withIntermediateDirectories: true)
        let destination = uniqueDestination(
            parent.appendingPathComponent(template.filename),
            mayReuseExactName: true
        )
        if (template.contentType ?? .data).conforms(to: .folder) {
            try FileManager.default.createDirectory(
                at: destination,
                withIntermediateDirectories: true
            )
        } else if let contents {
            try replaceItem(at: destination, with: contents)
        } else {
            FileManager.default.createFile(atPath: destination.path, contents: Data())
        }
        importDrive()
        guard let identifier = identifier(for: destination),
              let item = item(for: destination, identifier: identifier)
        else {
            throw NSError.fileProviderErrorForNonExistentItem(
                withIdentifier: template.itemIdentifier
            )
        }
        return item
    }

    static func modifyItem(
        _ item: NSFileProviderItem,
        changedFields: NSFileProviderItemFields,
        contents: URL?
    ) throws -> FileProviderItem {
        guard let original = url(for: item.itemIdentifier) else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier)
        }
        var destination = original
        if changedFields.contains(.filename), item.filename != original.lastPathComponent {
            destination = original.deletingLastPathComponent().appendingPathComponent(item.filename)
            try FileManager.default.moveItem(at: original, to: destination)
        }
        if let contents, !(item.contentType ?? .data).conforms(to: .folder) {
            try replaceItem(at: destination, with: contents)
        }
        importDrive()
        guard let identifier = identifier(for: destination),
              let updated = self.item(for: destination, identifier: identifier)
        else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier)
        }
        return updated
    }

    static func deleteItem(identifier: NSFileProviderItemIdentifier) throws {
        guard let url = url(for: identifier), url != driveRoot else {
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier)
        }
        try FileManager.default.removeItem(at: url)
        importDrive()
    }

    static func currentAnchor() -> NSFileProviderSyncAnchor {
        let modified = (try? driveRoot.resourceValues(
            forKeys: [.contentModificationDateKey]
        ).contentModificationDate)?.timeIntervalSince1970 ?? 0
        return NSFileProviderSyncAnchor(rawValue: Data("iris-drive-\(modified)".utf8))
    }

    private static func item(
        for url: URL,
        identifier: NSFileProviderItemIdentifier
    ) -> FileProviderItem? {
        if identifier == .rootContainer {
            return .root
        }
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let values = try? url.resourceValues(forKeys: [
            .contentTypeKey,
            .creationDateKey,
            .contentModificationDateKey,
            .fileSizeKey,
            .isDirectoryKey,
        ])
        let isDirectory = values?.isDirectory == true
        let contentType = isDirectory
            ? UTType.folder
            : values?.contentType ?? UTType(filenameExtension: url.pathExtension) ?? .data
        let parentIdentifier = Self.identifier(for: url.deletingLastPathComponent()) ?? .rootContainer
        let size = isDirectory ? nil : (values?.fileSize).map { NSNumber(value: $0) }
        return FileProviderItem(
            itemIdentifier: identifier,
            parentItemIdentifier: parentIdentifier,
            filename: url.lastPathComponent,
            contentType: contentType,
            itemSize: size,
            created: values?.creationDate,
            modified: values?.contentModificationDate
        )
    }

    private static func importDrive() {
        guard let executable = idriveExecutable, !executable.isEmpty else { return }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = ["--config-dir", configDirectory.path, "import", driveRoot.path]
        var environment = ProcessInfo.processInfo.environment
        environment["IRIS_DRIVE_CONFIG_DIR"] = configDirectory.path
        process.environment = environment
        do {
            try process.run()
            process.waitUntilExit()
        } catch {
            NSLog("Iris Drive FileProvider import failed: \(error)")
        }
    }

    private static func replaceItem(at destination: URL, with source: URL) throws {
        try FileManager.default.createDirectory(
            at: destination.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        if FileManager.default.fileExists(atPath: destination.path) {
            try FileManager.default.removeItem(at: destination)
        }
        try FileManager.default.copyItem(at: source, to: destination)
    }

    private static func uniqueDestination(
        _ url: URL,
        mayReuseExactName: Bool
    ) -> URL {
        if mayReuseExactName || !FileManager.default.fileExists(atPath: url.path) {
            return url
        }
        let directory = url.deletingLastPathComponent()
        let base = url.deletingPathExtension().lastPathComponent
        let ext = url.pathExtension
        for index in 2...999 {
            let name = ext.isEmpty ? "\(base) \(index)" : "\(base) \(index).\(ext)"
            let candidate = directory.appendingPathComponent(name)
            if !FileManager.default.fileExists(atPath: candidate.path) {
                return candidate
            }
        }
        return url
    }

    private static func relativePath(for url: URL) -> String? {
        let root = driveRoot.standardizedFileURL.path
        let path = url.standardizedFileURL.path
        if path == root {
            return ""
        }
        let prefix = root.hasSuffix("/") ? root : root + "/"
        guard path.hasPrefix(prefix) else { return nil }
        return String(path.dropFirst(prefix.count))
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
}
