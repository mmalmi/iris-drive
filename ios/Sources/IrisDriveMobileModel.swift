import Foundation
import FileProvider
import SwiftUI
import UIKit

struct IrisDriveDevice: Identifiable, Equatable {
    var id: String { label }
    var label: String
    var state: String
    var detail: String
    var isOnline: Bool
    var canRevoke: Bool
}

struct IrisDriveBackup: Identifiable, Equatable {
    let id = UUID()
    var label: String
    var state: String
    var detail: String
}

enum IrisDriveSharedContainer {
    static let appGroupIdentifier = "group.to.iris.drive"

    static var baseDirectory: URL {
        if let shared = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) {
            return shared.appendingPathComponent("Iris Drive", isDirectory: true)
        }
        let support = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.temporaryDirectory
        return support.appendingPathComponent("Iris Drive", isDirectory: true)
    }
}

private let irisDriveDomainIdentifier = NSFileProviderDomainIdentifier("main")
private let irisDriveFileProviderDisplayName = "My Drive"

@MainActor
final class IrisDriveMobileModel: ObservableObject {
    @Published var driveName = "My Drive"
    @Published var statusTitle = "Ready"
    @Published var statusDetail = "Waiting for this device to be linked."
    @Published var deviceLabel = UIDevice.current.name
    @Published var ownerPublicKey = ""
    @Published var devicePublicKey = "local-device"
    @Published var restoreSecret = ""
    @Published var relay = "wss://relay.damus.io"
    @Published var syncOverCellular = false
    @Published var syncRunning = false
    @Published var fileProviderStatus = "Files provider not registered"
    @Published var approveDeviceKey = ""
    @Published var approveDeviceLabel = ""
    @Published var devices: [IrisDriveDevice] = []
    @Published var backups: [IrisDriveBackup] = []

    private let defaults = UserDefaults.standard
    private let approvedDevicesKey = "approvedDevices"

    private struct StoredDevice: Codable, Equatable {
        var label: String
        var key: String
    }

    init() {
        load()
    }

    var sharedContainerPath: String {
        IrisDriveSharedContainer.baseDirectory.path
    }

    var statusSymbol: String {
        ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "link.circle"
            : "checkmark.circle.fill"
    }

    var statusTint: Color {
        ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? .orange
            : .green
    }

    var syncStateTitle: String {
        syncRunning ? "Sync running" : "Sync paused"
    }

    var snapshotLink: String {
        "https://drive.iris.to/snapshot/\(ownerPublicKey.isEmpty ? "local" : ownerPublicKey)"
    }

    var hasLocalProfile: Bool {
        !ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func ensureFileProviderDomainIfProfileExists() {
        guard hasLocalProfile else {
            fileProviderStatus = "Files provider not registered"
            return
        }
        ensureFileProviderDomain()
    }

    func ensureFileProviderDomain() {
        guard hasLocalProfile else {
            fileProviderStatus = "Files provider not registered"
            return
        }
        fileProviderStatus = "Registering Files provider"
        let domain = NSFileProviderDomain(
            identifier: irisDriveDomainIdentifier,
            displayName: irisDriveFileProviderDisplayName
        )
        NSFileProviderManager.add(domain) { [weak self] error in
            if error == nil {
                Task { @MainActor in
                    self?.fileProviderStatus = "Files provider ready"
                }
                return
            }

            NSFileProviderManager.getDomainsWithCompletionHandler { [weak self] domains, _ in
                let exists = domains.contains { $0.identifier == irisDriveDomainIdentifier }
                Task { @MainActor [weak self] in
                    self?.fileProviderStatus = exists
                        ? "Files provider ready"
                        : "Files provider unavailable"
                }
            }
        }
    }

    func refresh() {
        ensureFileProviderDomainIfProfileExists()
        load()
    }

    func createProfile() {
        ownerPublicKey = "local-owner"
        statusTitle = "Linked"
        statusDetail = syncStateTitle
        devicePublicKey = "device-\(UUID().uuidString.prefix(8))"
        persist()
        load()
        ensureFileProviderDomainIfProfileExists()
    }

    func restoreProfile() {
        guard !restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        ownerPublicKey = "restored-owner"
        statusTitle = "Restored"
        statusDetail = syncStateTitle
        restoreSecret = ""
        persist()
        load()
        ensureFileProviderDomainIfProfileExists()
    }

    func linkDevice() {
        guard !ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        statusTitle = "Link requested"
        statusDetail = "Approval is pending from an owner device."
        persist()
        load()
        ensureFileProviderDomainIfProfileExists()
    }

    func approveDevice() {
        let key = approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return }
        let label = approveDeviceLabel.trimmingCharacters(in: .whitespacesAndNewlines)
        var stored = loadApprovedDevices()
        let device = StoredDevice(
            label: label.isEmpty ? "Linked device" : label,
            key: key
        )
        if let index = stored.firstIndex(where: { $0.key == key }) {
            stored[index] = device
        } else {
            stored.append(device)
        }
        saveApprovedDevices(stored)
        approveDeviceKey = ""
        approveDeviceLabel = ""
        statusTitle = "Device approved"
        statusDetail = syncStateTitle
        persist()
        load()
    }

    func revokeDevice(label: String) {
        let stored = loadApprovedDevices().filter { $0.label != label }
        saveApprovedDevices(stored)
        statusTitle = "Device revoked"
        statusDetail = syncStateTitle
        load()
    }

    func startSync() {
        guard hasLocalProfile else { return }
        syncRunning = true
        statusTitle = "Sync running"
        statusDetail = "Foreground sync is active."
        persist()
        load()
    }

    func stopSync() {
        syncRunning = false
        statusTitle = "Sync paused"
        statusDetail = "Foreground sync is paused."
        persist()
        load()
    }

    func restartSync() {
        guard hasLocalProfile else { return }
        stopSync()
        startSync()
    }

    func copyOwnerKey() {
        UIPasteboard.general.string = ownerPublicKey
    }

    func copyDeviceKey() {
        UIPasteboard.general.string = devicePublicKey
    }

    func copySnapshotLink() {
        UIPasteboard.general.string = snapshotLink
    }

    func openSnapshotLink() {
        guard let url = URL(string: snapshotLink) else { return }
        UIApplication.shared.open(url)
    }

    func resetRelay() {
        relay = "wss://relay.damus.io"
        persist()
    }

    func resetLocalState() {
        ownerPublicKey = ""
        devicePublicKey = "local-device"
        restoreSecret = ""
        syncRunning = false
        statusTitle = "Ready"
        statusDetail = "Waiting for this device to be linked."
        saveApprovedDevices([])
        persist()
        load()
    }

    func handle(url: URL) {
        ownerPublicKey = url.absoluteString
        statusTitle = "Link opened"
        statusDetail = "Approval is pending from an owner device."
        persist()
        load()
    }

    private func load() {
        deviceLabel = defaults.string(forKey: "deviceLabel") ?? UIDevice.current.name
        ownerPublicKey = defaults.string(forKey: "ownerPublicKey") ?? ownerPublicKey
        devicePublicKey = defaults.string(forKey: "devicePublicKey") ?? devicePublicKey
        statusTitle = defaults.string(forKey: "statusTitle") ?? statusTitle
        statusDetail = defaults.string(forKey: "statusDetail") ?? statusDetail
        relay = defaults.string(forKey: "relay") ?? relay
        syncOverCellular = defaults.bool(forKey: "syncOverCellular")
        syncRunning = defaults.bool(forKey: "syncRunning")

        let currentDevice = IrisDriveDevice(
            label: deviceLabel,
            state: ownerPublicKey.isEmpty ? "Not linked" : "Authorized",
            detail: ownerPublicKey.isEmpty ? "No owner profile on this device." : devicePublicKey,
            isOnline: !ownerPublicKey.isEmpty,
            canRevoke: false
        )
        let approved = loadApprovedDevices().map { device in
            IrisDriveDevice(
                label: device.label,
                state: "Authorized",
                detail: device.key,
                isOnline: syncRunning,
                canRevoke: true
            )
        }
        devices = [currentDevice] + approved
        backups = [
            IrisDriveBackup(
                label: "Blossom fallback",
                state: ownerPublicKey.isEmpty ? "Paused" : "Ready",
                detail: "Configured after profile setup."
            ),
        ]
    }

    func persist() {
        defaults.set(deviceLabel, forKey: "deviceLabel")
        defaults.set(ownerPublicKey, forKey: "ownerPublicKey")
        defaults.set(devicePublicKey, forKey: "devicePublicKey")
        defaults.set(statusTitle, forKey: "statusTitle")
        defaults.set(statusDetail, forKey: "statusDetail")
        defaults.set(relay, forKey: "relay")
        defaults.set(syncOverCellular, forKey: "syncOverCellular")
        defaults.set(syncRunning, forKey: "syncRunning")
    }

    private func loadApprovedDevices() -> [StoredDevice] {
        guard let data = defaults.data(forKey: approvedDevicesKey),
              let devices = try? JSONDecoder().decode([StoredDevice].self, from: data)
        else {
            return []
        }
        return devices
    }

    private func saveApprovedDevices(_ devices: [StoredDevice]) {
        guard let data = try? JSONEncoder().encode(devices) else { return }
        defaults.set(data, forKey: approvedDevicesKey)
    }
}
