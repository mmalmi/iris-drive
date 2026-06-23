import FileProvider

final class FileProviderEnumerator: NSObject, NSFileProviderEnumerator {
    private let containerIdentifier: NSFileProviderItemIdentifier

    init(containerIdentifier: NSFileProviderItemIdentifier) {
        self.containerIdentifier = containerIdentifier
        super.init()
    }

    func invalidate() {}

    func enumerateItems(
        for observer: NSFileProviderEnumerationObserver,
        startingAt page: NSFileProviderPage
    ) {
        let items = FileProviderStorage.children(of: containerIdentifier)
        FileProviderStorage.debugLog(
            "enumerate items container=\(containerIdentifier.rawValue) count=\(items.count)"
        )
        observer.didEnumerate(items)
        observer.finishEnumerating(upTo: nil)
    }

    func enumerateChanges(
        for observer: NSFileProviderChangeObserver,
        from syncAnchor: NSFileProviderSyncAnchor
    ) {
        let (items, currentAnchor) = FileProviderStorage.allItemsAndAnchor()
        let previousIdentifiers = FileProviderStorage.storedSnapshotIdentifiers()
        let currentIdentifiers = Set(items.map(\.itemIdentifier.rawValue))
        let deleted = previousIdentifiers
            .subtracting(currentIdentifiers)
            .map { NSFileProviderItemIdentifier($0) }
        FileProviderStorage.debugLog(
            "enumerate changes from=\(String(data: syncAnchor.rawValue, encoding: .utf8) ?? "unavailable") current=\(String(data: currentAnchor.rawValue, encoding: .utf8) ?? "unavailable") previous=\(previousIdentifiers.count) current_count=\(currentIdentifiers.count) deleted=\(deleted.count)"
        )

        if !deleted.isEmpty {
            observer.didDeleteItems(withIdentifiers: deleted)
        }
        if syncAnchor.rawValue != currentAnchor.rawValue || !deleted.isEmpty {
            observer.didUpdate(items)
        }
        FileProviderStorage.recordSnapshot(items: items, anchor: currentAnchor)
        observer.finishEnumeratingChanges(upTo: currentAnchor, moreComing: false)
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        let anchor = FileProviderStorage.currentProviderAnchor()
        FileProviderStorage.debugLog(
            "current sync anchor \(String(data: anchor.rawValue, encoding: .utf8) ?? "unavailable")"
        )
        completionHandler(anchor)
    }
}
