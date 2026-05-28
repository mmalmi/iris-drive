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
        completionHandler(FileProviderStorage.currentProviderAnchor())
    }
}
