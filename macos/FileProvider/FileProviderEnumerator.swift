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
        NSLog(
            "Iris Drive FileProvider enumerate items container=\(containerIdentifier.rawValue) count=\(items.count)"
        )
        observer.didEnumerate(items)
        FileProviderStorage.recordCurrentSnapshot()
        observer.finishEnumerating(upTo: nil)
    }

    func enumerateChanges(
        for observer: NSFileProviderChangeObserver,
        from syncAnchor: NSFileProviderSyncAnchor
    ) {
        let previousIdentifiers = FileProviderStorage.storedSnapshotIdentifiers()
        let (items, currentAnchor) = FileProviderStorage.allItemsAndAnchor()
        let currentIdentifiers = Set(items.map(\.itemIdentifier.rawValue))
        let deletedIdentifiers = previousIdentifiers.subtracting(currentIdentifiers)
        if syncAnchor.rawValue != currentAnchor.rawValue || !deletedIdentifiers.isEmpty {
            let deleted = deletedIdentifiers.map { NSFileProviderItemIdentifier($0) }
            if !deleted.isEmpty {
                observer.didDeleteItems(withIdentifiers: deleted)
            }
            NSLog(
                "Iris Drive FileProvider enumerate changes update=\(items.count) delete=\(deleted.count)"
            )
            observer.didUpdate(items)
        }
        FileProviderStorage.recordSnapshot(items: items, anchor: currentAnchor)
        observer.finishEnumeratingChanges(upTo: currentAnchor, moreComing: false)
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        completionHandler(FileProviderStorage.currentAnchor())
    }
}
