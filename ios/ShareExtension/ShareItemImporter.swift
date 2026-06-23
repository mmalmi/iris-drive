import Foundation
import UIKit
import UniformTypeIdentifiers

final class ShareItemImporter {
    typealias SaveSharedFile = (_ displayName: String, _ contentType: UTType, _ contents: Data) throws -> Void
    typealias Log = (_ message: String) -> Void

    private let saveSharedFile: SaveSharedFile
    private let log: Log

    init(saveSharedFile: @escaping SaveSharedFile, log: @escaping Log = { _ in }) {
        self.saveSharedFile = saveSharedFile
        self.log = log
    }

    func preferredTypeIdentifier(for provider: NSItemProvider) -> String? {
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

    func loadProviderItem(
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
                    self.log("share file representation unavailable: \(error)")
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

    func importItem(
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
                try saveSharedFile(
                    defaultName(
                        provider: provider,
                        fallbackName: url.lastPathComponent,
                        contentType: fileType
                    ),
                    fileType,
                    data
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
            try saveSharedFile(
                defaultName(provider: provider, contentType: contentType),
                contentType,
                data
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
            try saveSharedFile(
                defaultName(provider: provider, contentType: .png),
                .png,
                data
            )
            return true
        }
        return false
    }

    private func shouldLoadFileRepresentation(_ typeIdentifier: String) -> Bool {
        guard let contentType = UTType(typeIdentifier) else { return false }
        return contentType.conforms(to: .image)
            || contentType.conforms(to: .movie)
            || contentType.conforms(to: .data)
    }

    private func importText(_ text: String, name: String, contentType: UTType) throws {
        guard let data = text.data(using: .utf8) else { return }
        try saveSharedFile(name, contentType, data)
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
