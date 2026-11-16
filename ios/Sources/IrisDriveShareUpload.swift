import BackgroundTasks
import Foundation
import Network

@_silgen_name("iris_drive_share_publish_json")
private func irisDriveSharePublish(
    _ directory: UnsafePointer<CChar>, _ allowNetwork: Bool, _ context: UnsafeRawPointer,
    _ cancelled: @convention(c) (UnsafeRawPointer) -> Bool
) -> UnsafeMutablePointer<CChar>?

/// Both the extension and a background launch of its containing app publish from
/// the same durable queue. Each attempt has its own cancellation flag.
final class IrisDriveShareUpload: @unchecked Sendable {
    static var taskIdentifier: String {
        (Bundle.main.object(forInfoDictionaryKey: "IrisDriveBackgroundSyncTaskIdentifier") as? String
            ?? "fi.siriusbusiness.drive.background-sync") + ".share-upload"
    }

    static func schedule() throws {
        let request = BGProcessingTaskRequest(identifier: taskIdentifier)
        request.requiresNetworkConnectivity = true
        try BGTaskScheduler.shared.submit(request)
    }

    private let lock = NSLock()
    private var cancelled = false

    func cancel() {
        lock.withLock { cancelled = true }
    }

    enum Status { case complete, pending, paused, waitingForWiFi }

    static var allowsCellular: Bool {
        get { groupDefaults?.bool(forKey: "syncOverCellular") ?? false }
        set { groupDefaults?.set(newValue, forKey: "syncOverCellular") }
    }

    private static var groupDefaults: UserDefaults? {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "IrisDriveAppGroupIdentifier") as? String else { return nil }
        return UserDefaults(suiteName: group)
    }

    /// Called from worker queues. Explicit cellular policy applies to the share
    /// sheet and later background launches, including a connection change mid-upload.
    func publish(in directory: String, allowNetwork: Bool = true) -> Status {
        let status = run(in: directory, network: false)
        guard status == .pending, allowNetwork else { return status }
        let monitor = NWPathMonitor()
        let ready = DispatchSemaphore(value: 0)
        monitor.pathUpdateHandler = { path in
            if path.status != .satisfied || (path.usesInterfaceType(.cellular) && !Self.allowsCellular) {
                self.cancel()
            }
            ready.signal()
        }
        monitor.start(queue: DispatchQueue.global(qos: .utility))
        defer { monitor.cancel() }
        guard ready.wait(timeout: .now() + 2) == .success, monitor.currentPath.status == .satisfied else {
            return .pending
        }
        if monitor.currentPath.usesInterfaceType(.cellular) && !Self.allowsCellular { return .waitingForWiFi }
        return run(in: directory, network: true)
    }

    private func run(in directory: String, network: Bool) -> Status {
        let context = Unmanaged.passUnretained(self).toOpaque()
        let json = withExtendedLifetime(self) {
            IrisDriveNativeProvider.takeString(irisDriveSharePublish(directory, network, context) { pointer in
                let attempt = Unmanaged<IrisDriveShareUpload>.fromOpaque(pointer).takeUnretainedValue()
                return attempt.lock.withLock { attempt.cancelled }
            })
        }
        guard let result = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else { return .pending }
        if result["paused"] as? Bool == true { return .paused }
        return result["ok"] as? Bool == true ? .complete : .pending
    }
}
