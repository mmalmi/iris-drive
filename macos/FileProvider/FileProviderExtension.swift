import CoreGraphics
import FileProvider
import Foundation

final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension, NSFileProviderThumbnailing {
    private let domain: NSFileProviderDomain

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        super.init()
        FileProviderStorage.configure(domain: domain)
        FileProviderStorage.debugLog("extension init domain=\(domain.identifier.rawValue)")
    }

    func invalidate() {
        FileProviderStorage.debugLog("extension invalidate")
    }

    func enumerator(
        for containerItemIdentifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest
    ) throws -> NSFileProviderEnumerator {
        FileProviderStorage.debugLog("enumerator requested container=\(containerItemIdentifier.rawValue)")
        guard containerItemIdentifier == .rootContainer
            || containerItemIdentifier == .workingSet
            || containerItemIdentifier == .trashContainer
            || FileProviderStorage.item(for: containerItemIdentifier)?.contentType == .folder
        else {
            FileProviderStorage.debugLog("enumerator missing container=\(containerItemIdentifier.rawValue)")
            throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: containerItemIdentifier)
        }
        return FileProviderEnumerator(containerIdentifier: containerItemIdentifier)
    }

    func item(
        for identifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        if let item = FileProviderStorage.item(for: identifier) {
            FileProviderStorage.debugLog("item resolved identifier=\(identifier.rawValue)")
            completionHandler(item, nil)
        } else {
            FileProviderStorage.debugLog("item missing identifier=\(identifier.rawValue)")
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
            FileProviderStorage.debugLog("fetch contents identifier=\(itemIdentifier.rawValue)")
            let url = try FileProviderStorage.contentsURL(for: itemIdentifier)
            guard let item = FileProviderStorage.item(for: itemIdentifier) else {
                throw NSError.fileProviderErrorForNonExistentItem(withIdentifier: itemIdentifier)
            }
            completionHandler(url, item, nil)
        } catch {
            FileProviderStorage.debugLog("fetch contents failed identifier=\(itemIdentifier.rawValue) error=\(error)")
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
        progress.cancellationHandler = {
            FileProviderStorage.debugLog("fetch thumbnails cancelled")
        }

        DispatchQueue.global(qos: .utility).async {
            FileProviderStorage.debugLog(
                "fetch thumbnails count=\(itemIdentifiers.count) size=\(Int(size.width))x\(Int(size.height))"
            )
            for identifier in itemIdentifiers {
                guard !progress.isCancelled else { break }
                do {
                    let thumbnail = try FileProviderStorage.thumbnailData(
                        for: identifier,
                        requestedSize: size
                    )
                    perThumbnailCompletionHandler(identifier, thumbnail, nil)
                } catch {
                    FileProviderStorage.debugLog(
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
            FileProviderStorage.debugLog("create item name=\(itemTemplate.filename)")
            let item = try FileProviderStorage.createItem(
                template: itemTemplate,
                contents: url,
                mayAlreadyExist: options.contains(.mayAlreadyExist)
            )
            completionHandler(item, [], false, nil)
        } catch {
            FileProviderStorage.debugLog("create item failed name=\(itemTemplate.filename) error=\(error)")
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
            FileProviderStorage.debugLog("modify item identifier=\(item.itemIdentifier.rawValue)")
            let updated = try FileProviderStorage.modifyItem(
                item,
                changedFields: changedFields,
                contents: newContents
            )
            completionHandler(updated, [], false, nil)
        } catch {
            FileProviderStorage.debugLog("modify item failed identifier=\(item.itemIdentifier.rawValue) error=\(error)")
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
            FileProviderStorage.debugLog("delete item identifier=\(identifier.rawValue)")
            try FileProviderStorage.deleteItem(identifier: identifier)
            completionHandler(nil)
        } catch {
            FileProviderStorage.debugLog("delete item failed identifier=\(identifier.rawValue) error=\(error)")
            completionHandler(error)
        }
        progress.completedUnitCount = 1
        return progress
    }
}
