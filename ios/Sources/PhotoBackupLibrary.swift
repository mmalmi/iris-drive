import CryptoKit
import Foundation
@preconcurrency import Photos

struct PhotoBackupScope: Equatable {
    let dataDir: String
    let profile: String
    let installation: String
}

enum PhotoBackupLibrary {
    static func isOriginal(_ type: PHAssetResourceType) -> Bool {
        [.photo, .video, .pairedVideo, .alternatePhoto, .audio].contains(type)
    }

    static func resourceID(installation: String, asset: String, type: Int, filename: String, occurrence: Int) -> String {
        let parts = ["ios", installation, asset, String(type), filename, String(occurrence)]
        return SHA256.hash(data: Data(parts.joined(separator: "\u{0}").utf8))
            .map { String(format: "%02x", $0) }.joined()
    }

    /// One reader, one staged original, and one commit at a time, including after a restart.
    static func run(scope: PhotoBackupScope, limit: Int?, networkAllowed: Bool,
                    networkForPublish: @escaping @MainActor @Sendable () -> Bool?, progress: @escaping @Sendable (Int) async -> Void) async throws -> String {
        let options = PHFetchOptions()
        options.predicate = NSPredicate(format: "mediaType == %d OR mediaType == %d", PHAssetMediaType.image.rawValue, PHAssetMediaType.video.rawValue)
        options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        return try await backUp(PHAsset.fetchAssets(with: options), scope: scope, support: support,
                                staging: FileManager.default.temporaryDirectory.appendingPathComponent("iris-drive-photo-backup.original"),
                                limit: limit, networkAllowed: networkAllowed, networkForPublish: networkForPublish, progress: progress)
    }

    static func backUp(_ assets: PHFetchResult<PHAsset>, scope: PhotoBackupScope, support: URL,
                       staging: URL, limit: Int?, networkAllowed: Bool,
                       networkForPublish: @escaping @MainActor @Sendable () -> Bool?,
                       progress: @escaping @Sendable (Int) async -> Void) async throws -> String {
        let pending = try await saveOriginals(assets, scope: scope, support: support, staging: staging,
                                              limit: limit, networkAllowed: networkAllowed, progress: progress)
        try Task.checkCancellation()
        guard let network = await networkForPublish() else { throw CancellationError() }
        try Task.checkCancellation()
        if try IrisDriveNativeProvider.publishPhotos(scope: scope, allowNetwork: network) {
            return pending ?? "Up to date"
        }
        let sync = "Saved in My Drive · waiting to sync"
        return pending.map { "\($0) \(sync)" } ?? sync
    }

    /// A nil result means all available originals have a local Drive receipt.
    private static func saveOriginals(_ assets: PHFetchResult<PHAsset>, scope: PhotoBackupScope, support: URL,
                                      staging: URL, limit: Int?, networkAllowed: Bool,
                                      progress: @escaping @Sendable (Int) async -> Void) async throws -> String? {
        let fm = FileManager.default
        let receipts = try PhotoBackupReceipts(url: support.appendingPathComponent("photo-backup.sqlite"), profile: scope.profile)
        if fm.fileExists(atPath: staging.path) { try fm.removeItem(at: staging) }
        var completed = 0
        var failures = 0
        var firstError: String?
        for index in 0..<assets.count {
            try Task.checkCancellation()
            let asset = assets.object(at: index)
            var occurrences: [String: Int] = [:]
            let originals = PHAssetResource.assetResources(for: asset).filter { isOriginal($0.type) }
            if originals.isEmpty {
                failures += 1
                firstError = firstError ?? "Photos is still preparing an original."
            }
            for resource in originals {
                let filename = resource.originalFilename
                let signature = "\(resource.type.rawValue)\u{0}\(filename)"
                let occurrence = occurrences[signature, default: 0]
                occurrences[signature] = occurrence + 1
                let id = resourceID(installation: scope.installation, asset: asset.localIdentifier,
                                    type: resource.type.rawValue, filename: filename, occurrence: occurrence)
                guard try !receipts.contains(id) else { continue }
                if let limit, completed + failures >= limit { return "More photos will be checked on the next backup." }
                try Task.checkCancellation()
                defer { try? fm.removeItem(at: staging) }
                do {
                    try await PhotoBackupExport.write(resource, to: staging, networkAllowed: networkAllowed)
                    if let taken = asset.creationDate {
                        try fm.setAttributes([.modificationDate: taken], ofItemAtPath: staging.path)
                    }
                    try Task.checkCancellation()
                    let access = PHPhotoLibrary.authorizationStatus(for: .readWrite)
                    guard access == .authorized || access == .limited else { return "Photo library access is required." }
                    if access == .limited && PHAsset.fetchAssets(withLocalIdentifiers: [asset.localIdentifier], options: nil).count == 0 { continue }
                    try IrisDriveNativeProvider.importPhoto(scope: scope, resource: id, filename: filename, source: staging)
                    try receipts.remember(id)
                    completed += 1
                    await progress(completed)
                } catch {
                    try Task.checkCancellation()
                    failures += 1
                    firstError = firstError ?? error.localizedDescription
                }
            }
        }
        if let firstError { return "\(failures) pending. \(firstError)" }
        return nil
    }
}

@_silgen_name("iris_drive_photo_backup_import_json")
private func irisDrivePhotoBackupImport(_ dataDir: UnsafePointer<CChar>, _ profile: UnsafePointer<CChar>,
    _ resource: UnsafePointer<CChar>, _ filename: UnsafePointer<CChar>, _ sourcePath: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_photo_backup_publish_json")
private func irisDrivePhotoBackupPublish(_ dataDir: UnsafePointer<CChar>, _ profile: UnsafePointer<CChar>,
                                         _ allowNetwork: Bool) -> UnsafeMutablePointer<CChar>?

extension IrisDriveNativeProvider {
    private struct ImportResult: Decodable { let ok: Bool; let error: String? }

    static func publishPhotos(scope: PhotoBackupScope, allowNetwork: Bool) throws -> Bool {
        let json = takeString(irisDrivePhotoBackupPublish(scope.dataDir, scope.profile, allowNetwork))
        return try JSONDecoder().decode(ImportResult.self, from: Data(json.utf8)).ok
    }

    static func importPhoto(scope: PhotoBackupScope, resource: String, filename: String, source: URL) throws {
        let raw = irisDrivePhotoBackupImport(scope.dataDir, scope.profile, resource, filename, source.path)
        let json = takeString(raw)
        let result = try JSONDecoder().decode(ImportResult.self, from: Data(json.utf8))
        guard result.ok else {
            throw NSError(domain: "IrisDrivePhotoBackup", code: 2, userInfo: [NSLocalizedDescriptionKey: result.error ?? "Couldn’t save the original to Drive."])
        }
    }
}
