import CoreGraphics
import FileProvider
import Foundation

final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension, NSFileProviderThumbnailing {
    private let storage: FileProviderStorage

    required init(domain: NSFileProviderDomain) {
        storage = FileProviderStorage(domain: domain)
        super.init()
        storage.debugLog("extension init domain=\(domain.identifier.rawValue)")
    }

    func invalidate() {
        storage.debugLog("extension invalidate")
    }

    func enumerator(
        for containerItemIdentifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest
    ) throws -> NSFileProviderEnumerator {
        storage.debugLog("enumerator requested container=\(containerItemIdentifier.rawValue)")
        guard containerItemIdentifier == .rootContainer
            || containerItemIdentifier == .workingSet
            || containerItemIdentifier == .trashContainer
            || storage.item(for: containerItemIdentifier)?.contentType == .folder
        else {
            storage.debugLog("enumerator missing container=\(containerItemIdentifier.rawValue)")
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: containerItemIdentifier)
        }
        return FileProviderEnumerator(containerIdentifier: containerItemIdentifier, storage: storage)
    }

    func item(
        for identifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        if let item = storage.item(for: identifier) {
            storage.debugLog("item resolved identifier=\(identifier.rawValue)")
            completionHandler(item, nil)
        } else {
            storage.debugLog("item missing identifier=\(identifier.rawValue)")
            completionHandler(nil, NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier))
        }
        progress.completedUnitCount = 1
        return progress
    }

    func fetchContents(
        for itemIdentifier: NSFileProviderItemIdentifier,
        version requestedVersion: NSFileProviderItemVersion?,
        request: NSFileProviderRequest,
        completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            storage.debugLog("fetch contents identifier=\(itemIdentifier.rawValue)")
            let url = try storage.contentsURL(for: itemIdentifier)
            guard let item = storage.item(for: itemIdentifier) else {
                throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: itemIdentifier)
            }
            completionHandler(url, item, nil)
        } catch {
            storage.debugLog("fetch contents failed identifier=\(itemIdentifier.rawValue) error=\(error)")
            completionHandler(nil, nil, error)
        }
        progress.completedUnitCount = 1
        return progress
    }

    func fetchThumbnails(
        for itemIdentifiers: [NSFileProviderItemIdentifier],
        requestedSize size: CGSize,
        perThumbnailCompletionHandler: @escaping (
            NSFileProviderItemIdentifier,
            Data?,
            Error?
        ) -> Void,
        completionHandler: @escaping (Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: Int64(itemIdentifiers.count))
        progress.cancellationHandler = { [storage] in
            storage.debugLog("fetch thumbnails cancelled")
        }

        DispatchQueue.global(qos: .utility).async { [storage] in
            storage.debugLog(
                "fetch thumbnails count=\(itemIdentifiers.count) size=\(Int(size.width))x\(Int(size.height))"
            )
            for identifier in itemIdentifiers {
                guard !progress.isCancelled else { break }
                do {
                    let thumbnail = try storage.thumbnailData(
                        for: identifier,
                        requestedSize: size
                    )
                    perThumbnailCompletionHandler(identifier, thumbnail, nil)
                } catch {
                    storage.debugLog(
                        "fetch thumbnail failed identifier=\(identifier.rawValue) error=\(error)"
                    )
                    perThumbnailCompletionHandler(identifier, nil, error)
                }
                progress.completedUnitCount += 1
            }
            completionHandler(progress.isCancelled ? CocoaError(.userCancelled) : nil)
        }
        return progress
    }

    func createItem(
        basedOn itemTemplate: NSFileProviderItem,
        fields: NSFileProviderItemFields,
        contents url: URL?,
        options: NSFileProviderCreateItemOptions,
        request: NSFileProviderRequest,
        completionHandler: @escaping (
            NSFileProviderItem?,
            NSFileProviderItemFields,
            Bool,
            Error?
        ) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            storage.debugLog("create item name=\(itemTemplate.filename)")
            let item = try storage.createItem(
                template: itemTemplate,
                contents: url,
                mayAlreadyExist: options.contains(.mayAlreadyExist)
            )
            completionHandler(item, [], false, nil)
        } catch {
            storage.debugLog("create item failed name=\(itemTemplate.filename) error=\(error)")
            completionHandler(nil, [], false, error)
        }
        progress.completedUnitCount = 1
        return progress
    }

    func modifyItem(
        _ item: NSFileProviderItem,
        baseVersion version: NSFileProviderItemVersion,
        changedFields: NSFileProviderItemFields,
        contents newContents: URL?,
        options: NSFileProviderModifyItemOptions,
        request: NSFileProviderRequest,
        completionHandler: @escaping (
            NSFileProviderItem?,
            NSFileProviderItemFields,
            Bool,
            Error?
        ) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            storage.debugLog("modify item identifier=\(item.itemIdentifier.rawValue)")
            let updated = try storage.modifyItem(
                item,
                changedFields: changedFields,
                contents: newContents
            )
            completionHandler(updated, [], false, nil)
        } catch {
            storage.debugLog("modify item failed identifier=\(item.itemIdentifier.rawValue) error=\(error)")
            completionHandler(nil, [], false, error)
        }
        progress.completedUnitCount = 1
        return progress
    }

    func deleteItem(
        identifier: NSFileProviderItemIdentifier,
        baseVersion version: NSFileProviderItemVersion,
        options: NSFileProviderDeleteItemOptions,
        request: NSFileProviderRequest,
        completionHandler: @escaping (Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            storage.debugLog("delete item identifier=\(identifier.rawValue)")
            try storage.deleteItem(identifier: identifier)
            completionHandler(nil)
        } catch {
            storage.debugLog("delete item failed identifier=\(identifier.rawValue) error=\(error)")
            completionHandler(error)
        }
        progress.completedUnitCount = 1
        return progress
    }
}
