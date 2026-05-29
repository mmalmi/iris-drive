import Foundation

@_silgen_name("iris_drive_app_new")
private func irisDriveAppNew(
    _ dataDir: UnsafePointer<CChar>,
    _ appVersion: UnsafePointer<CChar>
) -> UnsafeMutableRawPointer?

@_silgen_name("iris_drive_app_free")
private func irisDriveAppFree(_ handle: UnsafeMutableRawPointer?)

@_silgen_name("iris_drive_app_state_json")
private func irisDriveAppStateJson(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_app_refresh_json")
private func irisDriveAppRefreshJson(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_app_dispatch_json")
private func irisDriveAppDispatchJson(
    _ handle: UnsafeMutableRawPointer?,
    _ actionJson: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_qr_matrix_json")
private func irisDriveQrMatrixJson(_ text: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_string_free")
private func irisDriveStringFree(_ value: UnsafeMutablePointer<CChar>?)

final class IrisDriveNativeCore {
    private var handle: UnsafeMutableRawPointer?

    init(dataDir: String, appVersion: String) {
        handle = dataDir.withCString { dataDirPointer in
            appVersion.withCString { versionPointer in
                irisDriveAppNew(dataDirPointer, versionPointer)
            }
        }
    }

    deinit {
        irisDriveAppFree(handle)
    }

    func stateJson() -> String {
        takeString(irisDriveAppStateJson(handle))
    }

    func refreshJson() -> String {
        takeString(irisDriveAppRefreshJson(handle))
    }

    func dispatchJson(_ actionJson: String) -> String {
        actionJson.withCString { pointer in
            takeString(irisDriveAppDispatchJson(handle, pointer))
        }
    }

    func qrMatrix(text: String) -> QrMatrix {
        let json = text.withCString { pointer in
            takeString(irisDriveQrMatrixJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let matrix = try? JSONDecoder().decode(QrMatrix.self, from: data)
        else {
            return QrMatrix()
        }
        return matrix
    }

    private func takeString(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        guard let pointer else { return #"{"error":"native app returned null"}"# }
        defer { irisDriveStringFree(pointer) }
        return String(cString: pointer)
    }
}

struct NativeAppState: Codable {
    var ui: NativeUiState
    var error: String
}

struct NativeUiState: Codable {
    var roots: [NativeSyncRoot]
    var account: NativeAccount?
    var devices: [NativeDevice]
    var relays: [String]
    var backups: [NativeBackup]
    var paths: NativePaths
    var sync: NativeSyncStatus
    var snapshotLink: String

    enum CodingKeys: String, CodingKey {
        case roots
        case account
        case devices
        case relays
        case backups
        case paths
        case sync
        case snapshotLink = "snapshot_link"
    }
}

struct NativeSyncRoot: Codable {
    var name: String
    var localPath: String
    var status: String

    enum CodingKeys: String, CodingKey {
        case name
        case localPath = "local_path"
        case status
    }
}

struct NativeAccount: Codable {
    var ownerPubkey: String
    var devicePubkey: String
    var deviceLabel: String
    var authorizationState: String
    var hasOwnerSigningAuthority: Bool
    var deviceLinkRequest: String
    var deviceLinkInvite: String

    enum CodingKeys: String, CodingKey {
        case ownerPubkey = "owner_pubkey"
        case devicePubkey = "device_pubkey"
        case deviceLabel = "device_label"
        case authorizationState = "authorization_state"
        case hasOwnerSigningAuthority = "has_owner_signing_authority"
        case deviceLinkRequest = "device_link_request"
        case deviceLinkInvite = "device_link_invite"
    }
}

struct QrMatrix: Codable, Equatable {
    var width: Int = 0
    var cells: [Bool] = []
    var error: String = ""
}

struct NativeDevice: Codable {
    var pubkey: String
    var label: String
    var role: String
    var state: String
    var detail: String
    var isCurrentDevice: Bool
    var isOnline: Bool
    var canRevoke: Bool
    var canAppointAdmin: Bool
    var canDemoteAdmin: Bool

    enum CodingKeys: String, CodingKey {
        case pubkey
        case label
        case role
        case state
        case detail
        case isCurrentDevice = "is_current_device"
        case isOnline = "is_online"
        case canRevoke = "can_revoke"
        case canAppointAdmin = "can_appoint_admin"
        case canDemoteAdmin = "can_demote_admin"
    }
}

struct NativeBackup: Codable {
    var label: String
    var state: String
    var detail: String
}

struct NativePaths: Codable {
    var dataDir: String
    var configPath: String
    var blocksDir: String

    enum CodingKeys: String, CodingKey {
        case dataDir = "data_dir"
        case configPath = "config_path"
        case blocksDir = "blocks_dir"
    }
}

struct NativeSyncStatus: Codable {
    var running: Bool
    var status: String
}
