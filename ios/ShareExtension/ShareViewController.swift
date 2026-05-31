import Foundation
import UIKit
import UniformTypeIdentifiers

final class ShareViewController: UIViewController {
    private let statusLabel = UILabel()
    private var didStartImport = false

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
            guard let typeIdentifier = preferredTypeIdentifier(for: provider) else { continue }
            group.enter()
            provider.loadItem(forTypeIdentifier: typeIdentifier, options: nil) { item, _ in
                defer { group.leave() }
                do {
                    if try self.importItem(item, typeIdentifier: typeIdentifier, provider: provider) {
                        lock.lock()
                        imported += 1
                        lock.unlock()
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

    private func preferredTypeIdentifier(for provider: NSItemProvider) -> String? {
        let preferred = [
            UTType.fileURL.identifier,
            UTType.image.identifier,
            UTType.movie.identifier,
            UTType.url.identifier,
            UTType.plainText.identifier,
            UTType.data.identifier
        ]
        if let typeIdentifier = preferred.first(where: { provider.hasItemConformingToTypeIdentifier($0) }) {
            return typeIdentifier
        }
        return provider.registeredTypeIdentifiers.first { identifier in
            UTType(identifier)?.conforms(to: .data) == true
        }
    }

    private func importItem(
        _ item: NSSecureCoding?,
        typeIdentifier: String,
        provider: NSItemProvider
    ) throws -> Bool {
        let contentType = UTType(typeIdentifier) ?? .data
        if let url = item as? URL {
            if url.isFileURL {
                let data = try Data(contentsOf: url)
                try FileProviderStorage.importSharedFile(
                    named: provider.suggestedName ?? url.lastPathComponent,
                    contentType: UTType(filenameExtension: url.pathExtension) ?? contentType,
                    contents: data
                )
            } else {
                try importText(
                    url.absoluteString,
                    name: provider.suggestedName ?? "Shared link.url",
                    contentType: .url
                )
            }
            return true
        }
        if let data = item as? Data {
            try FileProviderStorage.importSharedFile(
                named: defaultName(provider: provider, contentType: contentType),
                contentType: contentType,
                contents: data
            )
            return true
        }
        if let text = item as? String {
            try importText(
                text,
                name: provider.suggestedName ?? "Shared text.txt",
                contentType: .plainText
            )
            return true
        }
        if let image = item as? UIImage,
           let data = image.pngData() {
            try FileProviderStorage.importSharedFile(
                named: defaultName(provider: provider, contentType: .png),
                contentType: .png,
                contents: data
            )
            return true
        }
        return false
    }

    private func importText(_ text: String, name: String, contentType: UTType) throws {
        guard let data = text.data(using: .utf8) else { return }
        try FileProviderStorage.importSharedFile(
            named: name,
            contentType: contentType,
            contents: data
        )
    }

    private func defaultName(provider: NSItemProvider, contentType: UTType) -> String {
        if let suggestedName = provider.suggestedName, !suggestedName.isEmpty {
            return suggestedName
        }
        if let preferredExtension = contentType.preferredFilenameExtension {
            return "Shared file.\(preferredExtension)"
        }
        return "Shared file"
    }
}
