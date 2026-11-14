import Foundation
@preconcurrency import Photos

/// PhotoKit delivers resource chunks serially. Cancel an iCloud download as soon as the
/// system expires a refresh, the user pauses backup, or the staged file stops accepting data.
final class PhotoBackupExport: @unchecked Sendable {
    private let lock = NSLock()
    private var request: PHAssetResourceDataRequestID?
    private var cancelled = false

    static func write(_ resource: PHAssetResource, to url: URL, networkAllowed: Bool) async throws {
        let export = PhotoBackupExport()
        guard FileManager.default.createFile(atPath: url.path, contents: nil) else {
            throw CocoaError(.fileWriteUnknown)
        }
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        let options = PHAssetResourceRequestOptions()
        options.isNetworkAccessAllowed = networkAllowed
        try await withTaskCancellationHandler {
            try Task.checkCancellation()
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
                var writeError: Error?
                let manager = PHAssetResourceManager.default()
                let id = manager.requestData(for: resource, options: options) { data in
                    guard writeError == nil else { return }
                    do { try handle.write(contentsOf: data) }
                    catch {
                        writeError = error
                        export.cancel()
                    }
                } completionHandler: { error in
                    if let error = writeError ?? error { cont.resume(throwing: error) }
                    else { cont.resume() }
                }
                export.register(id)
            }
        } onCancel: { export.cancel() }
    }

    private func register(_ id: PHAssetResourceDataRequestID) {
        lock.lock()
        request = id
        let shouldCancel = cancelled
        lock.unlock()
        if shouldCancel { PHAssetResourceManager.default().cancelDataRequest(id) }
    }

    private func cancel() {
        lock.lock()
        cancelled = true
        let id = request
        lock.unlock()
        if let id { PHAssetResourceManager.default().cancelDataRequest(id) }
    }
}
