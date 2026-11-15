import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// "Save to Iris Drive" from any app's share sheet. Like Google Drive's, it
/// shows what will be saved and where, and saves only when asked to.
final class ShareViewController: UIViewController {
    private let model = ShareSaveModel()
    private var didStartLoading = false
    private let collectedLock = NSLock()
    private var collected: [PendingSharedItem] = []
    private lazy var importer = ShareItemImporter(
        saveSharedFile: { [weak self] displayName, contentType, contents in
            guard let self else { return }
            self.collectedLock.lock()
            self.collected.append(PendingSharedItem(name: displayName, contentType: contentType, contents: contents))
            self.collectedLock.unlock()
        },
        log: FileProviderStorage.debugLog
    )

    override func viewDidLoad() {
        super.viewDidLoad()
        let host = UIHostingController(rootView: ShareSaveView(
            model: model,
            onCancel: { [weak self] in self?.cancel() },
            onSave: { [weak self] in self?.save() }
        ))
        addChild(host)
        host.view.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(host.view)
        NSLayoutConstraint.activate([
            host.view.topAnchor.constraint(equalTo: view.topAnchor),
            host.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            host.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            host.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
        ])
        host.didMove(toParent: self)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !didStartLoading else { return }
        didStartLoading = true
        loadSharedItems()
    }

    /// Read what was shared; nothing is written to the Drive yet.
    private func loadSharedItems() {
        let providers = extensionContext?.inputItems
            .compactMap { $0 as? NSExtensionItem }
            .flatMap { $0.attachments ?? [] } ?? []
        let group = DispatchGroup()
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
                    if try !self.importer.importItem(item, typeIdentifier: typeIdentifier, provider: provider) {
                        FileProviderStorage.debugLog("share import unsupported item for \(typeIdentifier)")
                    }
                } catch {
                    FileProviderStorage.debugLog("share item unreadable: \(error)")
                }
            }
        }
        group.notify(queue: .main) {
            self.collectedLock.lock()
            let items = self.collected
            self.collectedLock.unlock()
            let folders = FileProviderStorage.shareDestinations()
            Task { @MainActor in self.model.loaded(items, folders: folders) }
        }
    }

    private func save() {
        model.save(using: { item, folder in
            try FileProviderStorage.importSharedFile(
                named: item.name.trimmingCharacters(in: .whitespacesAndNewlines),
                contentType: item.contentType,
                contents: item.contents,
                into: folder
            )
        }, done: { [weak self] in
            self?.extensionContext?.completeRequest(returningItems: nil)
        })
    }

    private func cancel() {
        if case .saved = model.phase {
            extensionContext?.completeRequest(returningItems: nil)
        } else {
            extensionContext?.cancelRequest(withError: CocoaError(.userCancelled))
        }
    }
}
