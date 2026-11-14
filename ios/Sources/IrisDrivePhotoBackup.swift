import Foundation
import Network
@preconcurrency import Photos
import SwiftUI

@MainActor
final class IrisDrivePhotoBackup: NSObject, ObservableObject, PHPhotoLibraryChangeObserver {
    @Published private(set) var enabled = false
    @Published private(set) var access = PHAuthorizationStatus.notDetermined
    @Published private(set) var status = ""
    @Published private(set) var busy = false

    private let defaults = UserDefaults.standard
    private let consentKey = "photoBackupEnabledForProfile"
    private var profile = ""
    private var dataDir = ""
    private var foreground = false
    private var allowsCellular = false
    private var monitor: NWPathMonitor?
    private var path: NWPath?
    private var observing = false
    private var task: Task<Void, Never>?
    private var needsRescan = false
    private var canRead: Bool { access == .authorized || access == .limited }
    private var waitingForWiFi: Bool { path?.isExpensive == true && !allowsCellular }

    func configure(profile: String, dataDir: String, allowsCellular: Bool) {
        guard self.profile != profile || self.dataDir != dataDir || self.allowsCellular != allowsCellular else { return }
        let changedProfile = self.profile != profile || self.dataDir != dataDir
        if changedProfile { task?.cancel(); status = "" }
        self.profile = profile
        self.dataDir = dataDir
        self.allowsCellular = allowsCellular
        enabled = !profile.isEmpty && defaults.string(forKey: consentKey) == profile
        refreshAccess()
        if waitingForWiFi { task?.cancel() }
        requestScan()
    }

    func setForeground(_ active: Bool) {
        foreground = active
        refreshAccess()
        if active { requestScan() } else { task?.cancel() }
    }

    func setEnabled(_ value: Bool) {
        guard !value || !profile.isEmpty else { return }
        enabled = value
        if value { defaults.set(profile, forKey: consentKey) }
        else {
            defaults.removeObject(forKey: consentKey)
            task?.cancel()
            needsRescan = false
            status = ""
        }
        refreshAccess()
        guard value else { return }
        if access == .notDetermined {
            Task {
                _ = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
                refreshAccess()
                requestScan()
            }
        } else { requestScan() }
    }

    nonisolated func photoLibraryDidChange(_ changeInstance: PHChange) {
        Task { @MainActor in refreshAccess(); requestScan() }
    }

    private func refreshAccess() {
        access = enabled ? PHPhotoLibrary.authorizationStatus(for: .readWrite) : .notDetermined
        if enabled && canRead && !observing {
            PHPhotoLibrary.shared().register(self)
            observing = true
        } else if (!enabled || !canRead) && observing {
            PHPhotoLibrary.shared().unregisterChangeObserver(self)
            observing = false
            task?.cancel()
        }
        if enabled && canRead && monitor == nil {
            let monitor = NWPathMonitor()
            self.monitor = monitor
            monitor.pathUpdateHandler = { [weak self] path in
                Task { @MainActor in self?.networkChanged(path) }
            }
            monitor.start(queue: DispatchQueue(label: "iris-drive.photo-backup.network"))
        } else if !enabled || !canRead {
            monitor?.cancel()
            monitor = nil
            path = nil
        }
    }

    private func networkChanged(_ updated: NWPath) {
        guard monitor != nil else { return }
        let changed = path?.isExpensive != updated.isExpensive || path?.status != updated.status
        path = updated
        guard changed else { return }
        if waitingForWiFi { task?.cancel(); status = "Waiting for Wi-Fi" }
        else { requestScan() }
    }

    func requestScan() {
        guard foreground, enabled, canRead, path != nil else { return }
        guard !waitingForWiFi else { status = "Waiting for Wi-Fi"; return }
        if task != nil { needsRescan = true; return }
        _ = startScan(limit: nil)
    }

    func backgroundRefresh() async {
        refreshAccess()
        guard enabled, canRead, path != nil, !waitingForWiFi, task == nil else { return }
        let scan = startScan(limit: 4)
        await withTaskCancellationHandler { await scan.value } onCancel: { scan.cancel() }
    }

    private func startScan(limit: Int?) -> Task<Void, Never> {
        let installationKey = "photoBackupSourceID"
        let installation = defaults.string(forKey: installationKey) ?? UUID().uuidString
        defaults.set(installation, forKey: installationKey)
        let scope = PhotoBackupScope(dataDir: dataDir, profile: profile, installation: installation)
        let networkAllowed = path?.status == .satisfied && !waitingForWiFi
        status = "Checking your library…"
        busy = true
        let scan = Task {
            do {
                let result = try await PhotoBackupLibrary.run(scope: scope, limit: limit, networkAllowed: networkAllowed, networkForPublish: {
                    guard self.enabled, self.canRead, self.profile == scope.profile else { return nil }
                    return networkAllowed && self.path?.status == .satisfied && !self.waitingForWiFi
                }) { n in
                    await self.showProgress(n, profile: scope.profile)
                }
                if !Task.isCancelled && profile == scope.profile && enabled { status = result }
            } catch {
                if !Task.isCancelled && profile == scope.profile && enabled { status = error.localizedDescription }
            }
            task = nil
            busy = false
            if needsRescan { needsRescan = false; requestScan() }
        }
        task = scan
        return scan
    }

    private func showProgress(_ count: Int, profile: String) {
        guard self.profile == profile, enabled else { return }
        status = count == 1 ? "1 original saved" : "\(count) originals saved"
    }
}
