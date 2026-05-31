import CoreGraphics
import FileProvider
import Foundation

final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension, NSFileProviderThumbnailing {
    private let domain: NSFileProviderDomain

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        super.init()
        FileProviderStorage.debugLog("extension init domain=\(domain.identifier.rawValue)")
    }

    func invalidate() {
        FileProviderStorage.debugLog("extension invalidate")
    }

    func enumerator(
        for containerItemIdentifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest
    ) throws -> NSFileProviderEnumerator {
        guard containerItemIdentifier == .rootContainer
            || containerItemIdentifier == .workingSet
            || containerItemIdentifier == .trashContainer
            || FileProviderStorage.item(for: containerItemIdentifier)?.contentType == .folder
        else {
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
            completionHandler(item, nil)
        } else {
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
            let url = try FileProviderStorage.contentsURL(for: itemIdentifier)
            let item = FileProviderStorage.item(for: itemIdentifier)
            completionHandler(url, item, nil)
        } catch {
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
            let item = try FileProviderStorage.createItem(template: itemTemplate, contents: url)
            completionHandler(item, [], false, nil)
        } catch {
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
            let updated = try FileProviderStorage.modifyItem(
                item,
                changedFields: changedFields,
                contents: newContents
            )
            completionHandler(updated, [], false, nil)
        } catch {
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
            try FileProviderStorage.deleteItem(identifier: identifier)
            completionHandler(nil)
        } catch {
            completionHandler(error)
        }
        progress.completedUnitCount = 1
        return progress
    }
}
