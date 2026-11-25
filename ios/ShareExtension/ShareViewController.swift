import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// "Save to Iris Drive" from any app's share sheet. Like Google Drive's, it
/// shows what will be saved and where, and saves only when asked to.
final class ShareViewController: UIViewController {
    private let model = ShareSaveModel()
    private var folderTask: Task<Void, Never>?
    private var didStartLoading = false
    private let upload = IrisDriveShareUpload()
    private var finished = false
    private let collectedLock = NSLock()
    private var collected: [PendingSharedItem] = []
    private var loadError: String?
    /// Shared files wait here, on disk, until Save or Cancel.
    private let staging = FileManager.default.temporaryDirectory
        .appendingPathComponent("Shared items", isDirectory: true)
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    private lazy var importer = ShareItemImporter(
        stagingDirectory: staging,
        saveSharedFile: { [weak self] displayName, contentType, staged in
            guard let self else { return }
            self.collectedLock.lock()
            self.collected.append(PendingSharedItem(name: displayName, contentType: contentType, source: staged))
            self.collectedLock.unlock()
        },
        log: FileProviderStorage.debugLog
    )

    deinit {
        folderTask?.cancel()
        try? FileManager.default.removeItem(at: staging)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        FileProviderStorage.useSmallWriteBatches()
        let host = UIHostingController(rootView: ShareSaveView(
            model: model,
            onCancel: { [weak self] in self?.cancel() },
            onRetryFolders: { [weak self] in self?.retryFolders() },
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
            guard let typeIdentifier = importer.preferredTypeIdentifier(for: provider) else {
                recordLoadError("One of the shared items could not be read.")
                continue
            }
            FileProviderStorage.debugLog(
                "share provider types=\(provider.registeredTypeIdentifiers.joined(separator: ",")) chosen=\(typeIdentifier)"
            )
            group.enter()
            importer.loadProviderItem(provider, typeIdentifier: typeIdentifier) { result in
                defer { group.leave() }
                do {
                    let item = try result.get()
                    if try !self.importer.importItem(item, typeIdentifier: typeIdentifier, provider: provider) {
                        self.recordLoadError("One of the shared items could not be read.")
                    }
                } catch {
                    self.recordLoadError(error.localizedDescription)
                }
            }
        }
        group.notify(queue: .global(qos: .userInitiated)) {
            self.collectedLock.lock()
            let items = self.collected
            let error = self.loadError
            self.collectedLock.unlock()
            if let error {
                Task { @MainActor in self.model.phase = .failed(error) }
                return
            }
            Task { @MainActor in self.prepareFolders(for: items) }
        }
    }

    private func retryFolders() {
        guard model.phase == .waitingForFolders else { return }
        prepareFolders(for: model.items)
    }

    private func prepareFolders(for items: [PendingSharedItem]) {
        guard !finished else { return }
        folderTask?.cancel()
        folderTask = Task { [model] in
            await model.prepareDestinations(for: items) {
                do {
                    return try FileProviderStorage.shareDestinations()
                } catch {
                    FileProviderStorage.debugLog("share destinations unavailable: \(error.localizedDescription)")
                    throw error
                }
            }
        }
    }

    private func recordLoadError(_ message: String) {
        collectedLock.withLock { if loadError == nil { loadError = message } }
    }

    private func save() {
        var backgroundScheduled = false
        let upload = self.upload
        model.save(using: { item, folder in
            try FileProviderStorage.importSharedFile(
                named: item.name.trimmingCharacters(in: .whitespacesAndNewlines),
                from: item.source,
                into: folder
            )
            if upload.publish(in: FileProviderStorage.baseDirectory.path, allowNetwork: false) == .pending {
                do {
                    try IrisDriveShareUpload.schedule()
                    backgroundScheduled = true
                } catch {
                    FileProviderStorage.debugLog("Background upload scheduling failed: \(error.localizedDescription)")
                }
            }
        }, sync: {
            switch upload.publish(in: FileProviderStorage.baseDirectory.path) {
            case .complete: return nil
            case .paused: return "Saved on this iPhone. Sync is paused."
            case .waitingForWiFi where backgroundScheduled: return "Saved on this iPhone. Waiting for Wi-Fi."
            default: break
            }
            return backgroundScheduled
                ? "Saved on this iPhone. Waiting to sync."
                : "Saved on this iPhone. Open Iris Drive to finish syncing."
        }, done: { [weak self] in
            self?.complete()
        })
    }

    private func removeStaging() {
        try? FileManager.default.removeItem(at: staging)
    }

    private func complete() {
        guard !finished else { return }
        finished = true
        removeStaging()
        extensionContext?.completeRequest(returningItems: nil)
    }

    private func cancel() {
        switch model.phase {
        case .saved, .syncing:
            upload.cancel()
            complete()
        case .saving:
            break
        default:
            finished = true
            folderTask?.cancel()
            removeStaging()
            extensionContext?.cancelRequest(withError: CocoaError(.userCancelled))
        }
    }
}
