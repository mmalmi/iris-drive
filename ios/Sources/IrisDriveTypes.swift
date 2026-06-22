import Foundation

struct IrisDriveDevice: Identifiable, Equatable {
    var id: String { detail }
    var label: String
    var actorKind: String
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

    var isDeviceActor: Bool {
        actorKind == "device"
    }
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

struct IrisDriveShare: Identifiable, Equatable {
    var id: String { shareId }
    var shareId: String
    var displayName: String
    var sourcePath: String
    var sharedWithMePath: String
    var role: String
    var roleLabel: String
    var keyStatus: String
    var keyStatusLabel: String
    var writeAuthorization: String
    var writeAuthorizationLabel: String
    var canWrite: Bool
    var canAdmin: Bool
    var currentKeyEpoch: UInt64?
    var hasCurrentKeyWrap: Bool
    var keyUnavailable: Bool
    var repairNeeded: Bool
    var missingKeyWraps: [String]
    var participantCount: UInt64
    var appKeyCount: UInt64
    var members: [IrisDriveShareMember]
    var pendingInvites: [IrisDrivePendingShareInvite]
    var shortcutPaths: [String]
}

struct IrisDrivePendingShareInvite: Identifiable, Equatable {
    var id: String { representativeNpubHint }
    var representativeNpubHint: String
    var displayName: String
    var role: String
    var roleLabel: String
    var status: String
    var statusLabel: String
}

struct IrisDriveShareMember: Identifiable, Equatable {
    var id: String { profileId }
    var profileId: String
    var displayName: String
    var representativeNpubHint: String
    var role: String
    var roleLabel: String
    var status: String
    var statusLabel: String
    var appKeyCount: UInt64
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
            #if DEBUG
            if uiTestBaseDir == "__TMP__" {
                return FileManager.default.temporaryDirectory
                    .appendingPathComponent(storageDirectoryName, isDirectory: true)
            }
            if uiTestBaseDir.hasPrefix("__TMP__/") {
                let suffix = String(uiTestBaseDir.dropFirst("__TMP__/".count))
                    .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
                return FileManager.default.temporaryDirectory
                    .appendingPathComponent(suffix.isEmpty ? storageDirectoryName : suffix, isDirectory: true)
            }
            #endif
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
