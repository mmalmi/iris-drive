import Foundation

struct IrisDriveDevice: Identifiable, Equatable {
    var id: String { detail }
    var label: String
    var role: String
    var state: String
    var connectionState: String
    var connectionLabel: String
    var detail: String
    var isCurrentDevice: Bool
    var isOnline: Bool
    var canRevoke: Bool
    var canAppointAdmin: Bool
    var canDemoteAdmin: Bool
}

struct IrisDriveAppKeyLinkRequest: Identifiable, Equatable {
    var id: String { devicePubkey }
    var devicePubkey: String
    var label: String
    var requestedAt: UInt64
    var requestLink: String
}

struct IrisDriveBackup: Identifiable, Equatable {
    var id: String
    var kind: String
    var target: String
    var label: String
    var configuredLabel: String
    var state: String
    var detail: String
    var enabled: Bool
}

struct IrisDriveRelayStatus: Identifiable, Equatable {
    var id: String { url }
    var url: String
    var status: String
    var statusLabel: String
    var health: String

    init(url: String, status: String, statusLabel: String, health: String) {
        self.url = url
        self.status = status
        self.statusLabel = statusLabel
        self.health = health
    }

    init(_ native: NativeRelayStatus) {
        url = native.url
        status = native.status
        statusLabel = native.statusLabel
        health = native.health
    }
}

struct IrisDriveRoot: Identifiable, Equatable {
    var id: String { name }
    var name: String
    var status: String
    var path: String
}

enum IrisDriveSharedContainer {
    static let appGroupIdentifier = "group.to.iris.drive"
    static let storageDirectoryName = "IrisDrive"

    static var baseDirectory: URL {
        let uiTestBaseDir = ProcessInfo.processInfo.environment["IRIS_DRIVE_UI_TEST_BASE_DIR"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if let uiTestBaseDir, !uiTestBaseDir.isEmpty {
            return URL(fileURLWithPath: uiTestBaseDir, isDirectory: true)
        }
        if let shared = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) {
            return shared.appendingPathComponent(storageDirectoryName, isDirectory: true)
        }
        fatalError("Iris Drive app group is unavailable")
    }
}
