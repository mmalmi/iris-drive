import Foundation

struct IrisDriveDevice: Identifiable, Equatable {
    var id: String { detail }
    var label: String
    var role: String
    var state: String
    var detail: String
    var isCurrentDevice: Bool
    var isOnline: Bool
    var canRevoke: Bool
    var canAppointAdmin: Bool
    var canDemoteAdmin: Bool
}

struct IrisDriveDeviceLinkRequest: Identifiable, Equatable {
    var id: String { devicePubkey }
    var devicePubkey: String
    var label: String
    var requestedAt: UInt64
    var requestLink: String
}

struct IrisDriveBackup: Identifiable, Equatable {
    var id: String { detail }
    var label: String
    var state: String
    var detail: String
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
