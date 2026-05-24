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
        observer.finishEnumerating(upTo: nil)
    }

    func enumerateChanges(
        for observer: NSFileProviderChangeObserver,
        from syncAnchor: NSFileProviderSyncAnchor
    ) {
        let currentAnchor = FileProviderStorage.currentAnchor()
        if syncAnchor.rawValue != currentAnchor.rawValue {
            let items = FileProviderStorage.allItems()
            NSLog("Iris Drive FileProvider enumerate changes count=\(items.count)")
            observer.didUpdate(items)
        }
        observer.finishEnumeratingChanges(upTo: currentAnchor, moreComing: false)
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        completionHandler(FileProviderStorage.currentAnchor())
    }
}
