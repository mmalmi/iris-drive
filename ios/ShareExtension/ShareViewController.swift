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
            FileProviderStorage.debugLog(
                "share provider types=\(provider.registeredTypeIdentifiers.joined(separator: ",")) chosen=\(typeIdentifier)"
            )
            group.enter()
            loadProviderItem(provider, typeIdentifier: typeIdentifier) { result in
                defer { group.leave() }
                do {
                    let item = try result.get()
                    if try self.importItem(item, typeIdentifier: typeIdentifier, provider: provider) {
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

    private func preferredTypeIdentifier(for provider: NSItemProvider) -> String? {
        let exactPreferred = [
            UTType.fileURL.identifier,
            UTType.url.identifier,
            UTType.plainText.identifier,
        ]
        if let typeIdentifier = exactPreferred.first(where: { provider.hasItemConformingToTypeIdentifier($0) }) {
            return typeIdentifier
        }
        let concretePreferred = [UTType.image, UTType.movie, UTType.data]
        for contentType in concretePreferred {
            if let typeIdentifier = provider.registeredTypeIdentifiers.first(where: { identifier in
                UTType(identifier)?.conforms(to: contentType) == true
            }) {
                return typeIdentifier
            }
        }
        return provider.registeredTypeIdentifiers.first { identifier in
            UTType(identifier)?.conforms(to: .data) == true
        }
    }

    private func loadProviderItem(
        _ provider: NSItemProvider,
        typeIdentifier: String,
        completion: @escaping (Result<NSSecureCoding?, Error>) -> Void
    ) {
        if shouldLoadFileRepresentation(typeIdentifier) {
            provider.loadFileRepresentation(forTypeIdentifier: typeIdentifier) { url, error in
                if let url {
                    completion(.success(url as NSURL))
                    return
                }
                if let error {
                    FileProviderStorage.debugLog("share file representation unavailable: \(error)")
                }
                provider.loadItem(forTypeIdentifier: typeIdentifier, options: nil) { item, error in
                    if let error {
                        completion(.failure(error))
                    } else {
                        completion(.success(item))
                    }
                }
            }
            return
        }
        provider.loadItem(forTypeIdentifier: typeIdentifier, options: nil) { item, error in
            if let error {
                completion(.failure(error))
            } else {
                completion(.success(item))
            }
        }
    }

    private func shouldLoadFileRepresentation(_ typeIdentifier: String) -> Bool {
        guard let contentType = UTType(typeIdentifier) else { return false }
        return contentType.conforms(to: .image)
            || contentType.conforms(to: .movie)
            || contentType.conforms(to: .data)
    }

    private func importItem(
        _ item: NSSecureCoding?,
        typeIdentifier: String,
        provider: NSItemProvider
    ) throws -> Bool {
        let contentType = UTType(typeIdentifier) ?? .data
        let sharedURL: URL?
        if let url = item as? URL {
            sharedURL = url
        } else if let url = item as? NSURL {
            sharedURL = url as URL
        } else {
            sharedURL = nil
        }
        if let url = sharedURL {
            if url.isFileURL {
                let scoped = url.startAccessingSecurityScopedResource()
                defer {
                    if scoped {
                        url.stopAccessingSecurityScopedResource()
                    }
                }
                let data = try Data(contentsOf: url)
                let fileType = UTType(filenameExtension: url.pathExtension) ?? contentType
                try FileProviderStorage.importSharedFile(
                    named: defaultName(
                        provider: provider,
                        fallbackName: url.lastPathComponent,
                        contentType: fileType
                    ),
                    contentType: fileType,
                    contents: data
                )
            } else {
                try importText(
                    url.absoluteString,
                    name: defaultName(provider: provider, fallbackName: "Shared link.url", contentType: .url),
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
                name: defaultName(provider: provider, fallbackName: "Shared text.txt", contentType: .plainText),
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
        defaultName(provider: provider, fallbackName: nil, contentType: contentType)
    }

    private func defaultName(
        provider: NSItemProvider,
        fallbackName: String?,
        contentType: UTType
    ) -> String {
        if let suggestedName = provider.suggestedName?.trimmingCharacters(in: .whitespacesAndNewlines),
           !suggestedName.isEmpty {
            return nameWithExtensionIfNeeded(suggestedName, contentType: contentType)
        }
        if let fallbackName = fallbackName?.trimmingCharacters(in: .whitespacesAndNewlines),
           !fallbackName.isEmpty {
            return nameWithExtensionIfNeeded(fallbackName, contentType: contentType)
        }
        if let preferredExtension = contentType.preferredFilenameExtension {
            return "Shared file.\(preferredExtension)"
        }
        return "Shared file"
    }

    private func nameWithExtensionIfNeeded(_ name: String, contentType: UTType) -> String {
        guard (name as NSString).pathExtension.isEmpty,
              let preferredExtension = contentType.preferredFilenameExtension,
              !preferredExtension.isEmpty
        else {
            return name
        }
        return "\(name).\(preferredExtension)"
    }
}
