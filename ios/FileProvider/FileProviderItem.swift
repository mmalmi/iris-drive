import Foundation
import FileProvider
import UniformTypeIdentifiers

private func fileProviderVersionData(_ value: String) -> Data { let data = Data(value.utf8); return data.count <= 128 ? data : Data(data.prefix(128)) }

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
        versionIdentifier: String = "iris-drive-ios-provider-v1"
    ) {
        self.itemIdentifier = itemIdentifier
        self.parentItemIdentifier = parentItemIdentifier
        self.filename = filename
        self.contentType = contentType
        self.itemSize = itemSize
        self.created = created
        self.modified = modified
        let version = fileProviderVersionData(versionIdentifier)
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

    var typeIdentifier: String {
        contentType.identifier
    }

    var creationDate: Date? {
        created
    }

    var contentModificationDate: Date? {
        modified
    }
}

extension FileProviderItem {
    static func root(anchor: String? = nil, modified: Date? = nil) -> FileProviderItem {
        FileProviderItem(
            itemIdentifier: .rootContainer,
            parentItemIdentifier: .rootContainer,
            filename: "Iris Drive",
            contentType: .folder,
            created: modified,
            modified: modified,
            versionIdentifier: "root:\(anchor ?? "initial")"
        )
    }

    static func trash(anchor: String? = nil, modified: Date? = nil) -> FileProviderItem {
        FileProviderItem(
            itemIdentifier: .trashContainer,
            parentItemIdentifier: .rootContainer,
            filename: ".Trash",
            contentType: .folder,
            created: modified,
            modified: modified,
            versionIdentifier: "trash:\(anchor ?? "initial")"
        )
    }
}
