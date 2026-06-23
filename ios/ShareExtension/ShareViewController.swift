import Foundation
import UIKit
import UniformTypeIdentifiers

final class ShareViewController: UIViewController {
    private let statusLabel = UILabel()
    private var didStartImport = false
    private lazy var importer = ShareItemImporter(
        saveSharedFile: { displayName, contentType, contents in
            try FileProviderStorage.importSharedFile(
                named: displayName,
                contentType: contentType,
                contents: contents
            )
        },
        log: FileProviderStorage.debugLog
    )

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        statusLabel.text = "Saving to Iris Drive..."
        statusLabel.textAlignment = .center
        statusLabel.font = .preferredFont(forTextStyle: .headline)
        view.addSubview(statusLabel)
        NSLayoutConstraint.activate([
            statusLabel.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            statusLabel.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            statusLabel.leadingAnchor.constraint(greaterThanOrEqualTo: view.leadingAnchor, constant: 24),
            statusLabel.trailingAnchor.constraint(lessThanOrEqualTo: view.trailingAnchor, constant: -24)
        ])
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !didStartImport else { return }
        didStartImport = true
        importSharedItems()
    }

    private func importSharedItems() {
        let providers = extensionContext?.inputItems
            .compactMap { $0 as? NSExtensionItem }
            .flatMap { $0.attachments ?? [] } ?? []
        let group = DispatchGroup()
        let lock = NSLock()
        var imported = 0
        var failed = 0

        for provider in providers {
            guard let typeIdentifier = importer.preferredTypeIdentifier(for: provider) else { continue }
            FileProviderStorage.debugLog(
                "share provider types=\(provider.registeredTypeIdentifiers.joined(separator: ",")) chosen=\(typeIdentifier)"
            )
            group.enter()
            importer.loadProviderItem(provider, typeIdentifier: typeIdentifier) { result in
                defer { group.leave() }
                do {
                    let item = try result.get()
                    if try self.importer.importItem(item, typeIdentifier: typeIdentifier, provider: provider) {
                        lock.lock()
                        imported += 1
                        lock.unlock()
                    } else {
                        FileProviderStorage.debugLog("share import unsupported item for \(typeIdentifier)")
                    }
                } catch {
                    FileProviderStorage.debugLog("share import failed: \(error)")
                    lock.lock()
                    failed += 1
                    lock.unlock()
                }
            }
        }

        group.notify(queue: .main) {
            if imported == 0 {
                self.statusLabel.text = failed == 0 ? "No items saved" : "Could not save to Iris Drive"
            } else {
                self.statusLabel.text = imported == 1
                    ? "Saved to Iris Drive"
                    : "Saved \(imported) items to Iris Drive"
            }
            self.extensionContext?.completeRequest(returningItems: nil)
        }
    }
}
