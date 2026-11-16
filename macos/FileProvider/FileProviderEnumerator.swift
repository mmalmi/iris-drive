import FileProvider

final class FileProviderEnumerator: NSObject, NSFileProviderEnumerator {
    private let storage: FileProviderStorage
    private let containerIdentifier: NSFileProviderItemIdentifier

    init(containerIdentifier: NSFileProviderItemIdentifier, storage: FileProviderStorage) {
        self.storage = storage
        self.containerIdentifier = containerIdentifier
        super.init()
    }

    func invalidate() {}

    func enumerateItems(
        for observer: NSFileProviderEnumerationObserver,
        startingAt page: NSFileProviderPage
    ) {
        do {
            let items = try storage.children(of: containerIdentifier)
            storage.debugLog(
                "enumerate items container=\(containerIdentifier.rawValue) count=\(items.count)"
            )
            observer.didEnumerate(items)
            observer.finishEnumerating(upTo: nil)
        } catch {
            observer.finishEnumeratingWithError(error)
        }
    }

    func enumerateChanges(
        for observer: NSFileProviderChangeObserver,
        from syncAnchor: NSFileProviderSyncAnchor
    ) {
        do {
            let (items, currentAnchor) = try storage.allItemsAndAnchor()
            let baseline = try storage.snapshotIdentifiers(from: syncAnchor, current: currentAnchor)
            let previousIdentifiers = baseline ?? []
            let currentIdentifiers = Set(items.map(\.itemIdentifier.rawValue))
            let deletedIdentifiers = previousIdentifiers.subtracting(currentIdentifiers)
            let shouldPublishChanges = baseline == nil
                || syncAnchor.rawValue != currentAnchor.rawValue
                || !deletedIdentifiers.isEmpty
            if shouldPublishChanges {
                let deleted = deletedIdentifiers.map { NSFileProviderItemIdentifier($0) }
                if !deleted.isEmpty {
                    observer.didDeleteItems(withIdentifiers: deleted)
                }
                storage.debugLog(
                    "enumerate changes update=\(items.count) delete=\(deleted.count) bootstrap=\(baseline == nil) anchor=\(String(data: currentAnchor.rawValue, encoding: .utf8) ?? "unreadable")"
                )
                if !items.isEmpty {
                    observer.didUpdate(items)
                }
            } else {
                storage.debugLog("enumerate changes noop")
            }
            storage.recordSnapshot(items: items, anchor: currentAnchor)
            observer.finishEnumeratingChanges(upTo: currentAnchor, moreComing: false)
        } catch {
            observer.finishEnumeratingWithError(error)
        }
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        let anchor = storage.currentProviderAnchor()
        storage.debugLog(
            "current sync anchor \(String(data: anchor.rawValue, encoding: .utf8) ?? "unreadable")"
        )
        completionHandler(anchor)
    }
}
