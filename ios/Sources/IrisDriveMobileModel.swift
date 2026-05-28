import Foundation
import SwiftUI

struct IrisDriveDevice: Identifiable, Equatable {
    let id = UUID()
    var label: String
    var state: String
    var detail: String
    var isOnline: Bool
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

@MainActor
final class IrisDriveMobileModel: ObservableObject {
    @Published var driveName = "My Drive"
    @Published var statusTitle = "Ready"
    @Published var statusDetail = "Waiting for this device to be linked."
    @Published var deviceLabel = UIDevice.current.name
    @Published var ownerPublicKey = ""
    @Published var restoreSecret = ""
    @Published var relay = "wss://relay.damus.io"
    @Published var syncOverCellular = false
    @Published var devices: [IrisDriveDevice] = []
    @Published var backups: [IrisDriveBackup] = []

    private let defaults = UserDefaults.standard

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

    func refresh() {
        load()
    }

    func createProfile() {
        ownerPublicKey = "local-owner"
        statusTitle = "Linked"
        statusDetail = "This device is ready for foreground sync."
        persist()
        load()
    }

    func restoreProfile() {
        guard !restoreSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        ownerPublicKey = "restored-owner"
        statusTitle = "Restored"
        statusDetail = "Profile restored on this device."
        restoreSecret = ""
        persist()
        load()
    }

    func linkDevice() {
        guard !ownerPublicKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        statusTitle = "Link requested"
        statusDetail = "Approval is pending from an owner device."
        persist()
        load()
    }

    func resetLocalState() {
        ownerPublicKey = ""
        restoreSecret = ""
        statusTitle = "Ready"
        statusDetail = "Waiting for this device to be linked."
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
        statusTitle = defaults.string(forKey: "statusTitle") ?? statusTitle
        statusDetail = defaults.string(forKey: "statusDetail") ?? statusDetail
        relay = defaults.string(forKey: "relay") ?? relay
        syncOverCellular = defaults.bool(forKey: "syncOverCellular")

        devices = [
            IrisDriveDevice(
                label: deviceLabel,
                state: ownerPublicKey.isEmpty ? "Not linked" : "Authorized",
                detail: ownerPublicKey.isEmpty ? "No owner profile on this device." : ownerPublicKey,
                isOnline: !ownerPublicKey.isEmpty
            ),
        ]
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
        defaults.set(statusTitle, forKey: "statusTitle")
        defaults.set(statusDetail, forKey: "statusDetail")
        defaults.set(relay, forKey: "relay")
        defaults.set(syncOverCellular, forKey: "syncOverCellular")
    }
}
