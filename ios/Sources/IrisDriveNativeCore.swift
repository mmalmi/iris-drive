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

@_silgen_name("iris_drive_classify_link_input_json")
private func irisDriveClassifyLinkInputJson(_ text: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_validate_link_input_json")
private func irisDriveValidateLinkInputJson(_ text: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_export_recovery_secret_json")
private func irisDriveExportRecoverySecretJson(_ dataDir: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_list_json")
private func irisDriveProviderListJson(_ dataDir: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_read_json")
private func irisDriveProviderReadJson(
    _ dataDir: UnsafePointer<CChar>,
    _ path: UnsafePointer<CChar>,
    _ outputPath: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_write_json")
private func irisDriveProviderWriteJson(
    _ dataDir: UnsafePointer<CChar>,
    _ path: UnsafePointer<CChar>,
    _ sourcePath: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_mkdir_json")
private func irisDriveProviderMkdirJson(
    _ dataDir: UnsafePointer<CChar>,
    _ path: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_delete_json")
private func irisDriveProviderDeleteJson(
    _ dataDir: UnsafePointer<CChar>,
    _ path: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_rename_json")
private func irisDriveProviderRenameJson(
    _ dataDir: UnsafePointer<CChar>,
    _ oldPath: UnsafePointer<CChar>,
    _ newPath: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_import_shared_file_json")
private func irisDriveProviderImportSharedFileJson(
    _ dataDir: UnsafePointer<CChar>,
    _ displayName: UnsafePointer<CChar>,
    _ sourcePath: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_resolve_path_json")
private func irisDriveProviderResolvePathJson(
    _ dataDir: UnsafePointer<CChar>,
    _ parentPath: UnsafePointer<CChar>,
    _ displayName: UnsafePointer<CChar>,
    _ excludingPath: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("iris_drive_provider_normalize_path_json")
private func irisDriveProviderNormalizePathJson(
    _ path: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

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

    static func exportRecoverySecret(dataDir: String) -> NativeRecoverySecretExport {
        let json = dataDir.withCString { pointer in
            takeString(irisDriveExportRecoverySecretJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let value = try? JSONDecoder().decode(NativeRecoverySecretExport.self, from: data)
        else {
            return NativeRecoverySecretExport(error: "native recovery export returned invalid JSON")
        }
        return value
    }

    private func takeString(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        guard let pointer else { return #"{"error":"native app returned null"}"# }
        defer { irisDriveStringFree(pointer) }
        return String(cString: pointer)
    }

    private static func takeString(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        guard let pointer else { return #"{"error":"native app returned null"}"# }
        defer { irisDriveStringFree(pointer) }
        return String(cString: pointer)
    }
}

extension IrisDriveNativeCore: @unchecked Sendable {}

enum IrisDriveNativeLinkInput {
    static func classify(_ text: String) -> NativeLinkInputClassification {
        let json = text.withCString { pointer in
            takeString(irisDriveClassifyLinkInputJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let value = try? JSONDecoder().decode(NativeLinkInputClassification.self, from: data)
        else {
            return NativeLinkInputClassification(error: "native link classifier returned invalid JSON")
        }
        return value
    }

    static func validate(_ text: String) -> NativeLinkInputClassification {
        let json = text.withCString { pointer in
            takeString(irisDriveValidateLinkInputJson(pointer))
        }
        guard let data = json.data(using: .utf8),
              let value = try? JSONDecoder().decode(NativeLinkInputClassification.self, from: data)
        else {
            return NativeLinkInputClassification(error: "native link validator returned invalid JSON")
        }
        return value
    }

    static func isComplete(_ text: String) -> Bool {
        validate(text).isComplete
    }

    private static func takeString(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        guard let pointer else { return #"{"error":"native link classifier returned null"}"# }
        defer { irisDriveStringFree(pointer) }
        return String(cString: pointer)
    }
}

enum IrisDriveNativeProvider {
    static func list(dataDir: String) -> String {
        dataDir.withCString { dataDirPointer in
            takeString(irisDriveProviderListJson(dataDirPointer))
        }
    }

    static func read(dataDir: String, path: String, outputPath: String) -> String {
        dataDir.withCString { dataDirPointer in
            path.withCString { pathPointer in
                outputPath.withCString { outputPointer in
                    takeString(irisDriveProviderReadJson(dataDirPointer, pathPointer, outputPointer))
                }
            }
        }
    }

    static func write(dataDir: String, path: String, sourcePath: String) -> String {
        dataDir.withCString { dataDirPointer in
            path.withCString { pathPointer in
                sourcePath.withCString { sourcePointer in
                    takeString(irisDriveProviderWriteJson(dataDirPointer, pathPointer, sourcePointer))
                }
            }
        }
    }

    static func mkdir(dataDir: String, path: String) -> String {
        dataDir.withCString { dataDirPointer in
            path.withCString { pathPointer in
                takeString(irisDriveProviderMkdirJson(dataDirPointer, pathPointer))
            }
        }
    }

    static func delete(dataDir: String, path: String) -> String {
        dataDir.withCString { dataDirPointer in
            path.withCString { pathPointer in
                takeString(irisDriveProviderDeleteJson(dataDirPointer, pathPointer))
            }
        }
    }

    static func rename(dataDir: String, oldPath: String, newPath: String) -> String {
        dataDir.withCString { dataDirPointer in
            oldPath.withCString { oldPointer in
                newPath.withCString { newPointer in
                    takeString(irisDriveProviderRenameJson(dataDirPointer, oldPointer, newPointer))
                }
            }
        }
    }

    static func importSharedFile(dataDir: String, displayName: String, sourcePath: String) -> String {
        dataDir.withCString { dataDirPointer in
            displayName.withCString { namePointer in
                sourcePath.withCString { sourcePointer in
                    takeString(
                        irisDriveProviderImportSharedFileJson(dataDirPointer, namePointer, sourcePointer)
                    )
                }
            }
        }
    }

    static func resolvePath(
        dataDir: String,
        parentPath: String,
        displayName: String,
        excludingPath: String = ""
    ) -> NativeProviderResolvedPath {
        let json = dataDir.withCString { dataDirPointer in
            parentPath.withCString { parentPointer in
                displayName.withCString { namePointer in
                    excludingPath.withCString { excludingPointer in
                        takeString(
                            irisDriveProviderResolvePathJson(
                                dataDirPointer,
                                parentPointer,
                                namePointer,
                                excludingPointer
                            )
                        )
                    }
                }
            }
        }
        guard let data = json.data(using: .utf8),
              let value = try? JSONDecoder().decode(NativeProviderResolvedPath.self, from: data)
        else {
            return NativeProviderResolvedPath(error: "native provider path resolver returned invalid JSON")
        }
        return value
    }

    static func normalizePath(path: String) -> NativeProviderResolvedPath {
        let json = path.withCString { pathPointer in
            takeString(irisDriveProviderNormalizePathJson(pathPointer))
        }
        guard let data = json.data(using: .utf8),
              let value = try? JSONDecoder().decode(NativeProviderResolvedPath.self, from: data)
        else {
            return NativeProviderResolvedPath(error: "native provider path normalizer returned invalid JSON")
        }
        return value
    }

    private static func takeString(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        guard let pointer else { return #"{"error":"native provider returned null"}"# }
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
    var profile: NativeProfile?
    var devices: [NativeDevice]
    var relays: [String]
    var relayStatuses: [NativeRelayStatus]
    var backups: [NativeBackup]
    var paths: NativePaths
    var sync: NativeSyncStatus
    var fips: NativeFipsStatus
    var setupState: String
    var setupComplete: Bool
    var awaitingApproval: Bool
    var revoked: Bool
    var setupLabel: String
    var primaryStatus: String
    var primaryStatusLabel: String
    var authorizedDeviceCount: UInt64
    var onlineDeviceCount: UInt64
    var fileCount: UInt64
    var visibleFileBytes: UInt64
    var providerChangeKey: String
    var providerDirectoryPaths: [String]
    var snapshotLink: String

    enum CodingKeys: String, CodingKey {
        case roots
        case profile
        case devices
        case relays
        case relayStatuses = "relay_statuses"
        case backups
        case paths
        case sync
        case fips
        case setupState = "setup_state"
        case setupComplete = "setup_complete"
        case awaitingApproval = "awaiting_approval"
        case revoked
        case setupLabel = "setup_label"
        case primaryStatus = "primary_status"
        case primaryStatusLabel = "primary_status_label"
        case authorizedDeviceCount = "authorized_device_count"
        case onlineDeviceCount = "online_device_count"
        case fileCount = "file_count"
        case visibleFileBytes = "visible_file_bytes"
        case providerChangeKey = "provider_change_key"
        case providerDirectoryPaths = "provider_directory_paths"
        case snapshotLink = "snapshot_link"
    }
}

struct NativeLinkInputClassification: Codable {
    var kind: String = ""
    var isComplete: Bool = false
    var isValid: Bool = false
    var normalizedInput: String = ""
    var appKeyPubkey: String = ""
    var adminAppKeyPubkey: String = ""
    var hasLinkSecret: Bool = false
    var error: String = ""

    init(error: String = "") {
        self.error = error
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case isComplete = "is_complete"
        case isValid = "is_valid"
        case normalizedInput = "normalized_input"
        case appKeyPubkey = "app_key_pubkey"
        case adminAppKeyPubkey = "admin_app_key_pubkey"
        case hasLinkSecret = "has_link_secret"
        case error
    }
}

struct NativeProviderResolvedPath: Codable {
    var parentPath: String = ""
    var displayName: String = ""
    var path: String = ""
    var error: String = ""

    init(error: String = "") {
        self.error = error
    }

    enum CodingKeys: String, CodingKey {
        case parentPath = "parent_path"
        case displayName = "display_name"
        case path
        case error
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

struct NativeProfile: Codable {
    var profileId: String
    var currentAppKeyPubkey: String
    var currentAppKeyNpub: String
    var currentAppKeyLabel: String
    var appKeyLabel: String
    var authorizationState: String
    var canAdminProfile: Bool
    var canWriteRoots: Bool
    var canExportRecoveryPhrase: Bool
    var appKeyLinkRequest: String
    var appKeyLinkInvite: String
    var inboundAppKeyLinkRequests: [NativeDeviceLinkRequest]

    var devicePubkey: String { currentAppKeyNpub }

    enum CodingKeys: String, CodingKey {
        case profileId = "profile_id"
        case currentAppKeyPubkey = "current_app_key_pubkey"
        case currentAppKeyNpub = "current_app_key_npub"
        case currentAppKeyLabel = "current_app_key_label"
        case appKeyLabel = "app_key_label"
        case authorizationState = "authorization_state"
        case canAdminProfile = "can_admin_profile"
        case canWriteRoots = "can_write_roots"
        case canExportRecoveryPhrase = "can_export_recovery_phrase"
        case appKeyLinkRequest = "app_key_link_request"
        case appKeyLinkInvite = "app_key_link_invite"
        case inboundAppKeyLinkRequests = "inbound_app_key_link_requests"
    }
}

struct NativeRecoverySecretExport: Codable {
    var canExport: Bool = false
    var recoveryPhrase: String = ""
    var words: [String] = []
    var secretKey: String = ""
    var error: String = ""

    enum CodingKeys: String, CodingKey {
        case canExport = "can_export"
        case recoveryPhrase = "recovery_phrase"
        case words
        case secretKey = "secret_key"
        case error
    }
}

struct NativeDeviceLinkRequest: Codable {
    var devicePubkey: String
    var label: String
    var requestedAt: UInt64
    var requestLink: String

    enum CodingKeys: String, CodingKey {
        case devicePubkey = "app_key_pubkey"
        case label
        case requestedAt = "requested_at"
        case requestLink = "request_link"
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
    var displayLabel: String
    var role: String
    var roleLabel: String
    var state: String
    var stateLabel: String
    var connectionState: String
    var connectionLabel: String
    var detail: String
    var isCurrentDevice: Bool
    var isOnline: Bool
    var canRevoke: Bool
    var canAppointAdmin: Bool
    var canDemoteAdmin: Bool

    enum CodingKeys: String, CodingKey {
        case pubkey
        case label
        case displayLabel = "display_label"
        case role
        case roleLabel = "role_label"
        case state
        case stateLabel = "state_label"
        case connectionState = "connection_state"
        case connectionLabel = "connection_label"
        case detail
        case isCurrentDevice = "is_current_device"
        case isOnline = "is_online"
        case canRevoke = "can_revoke"
        case canAppointAdmin = "can_appoint_admin"
        case canDemoteAdmin = "can_demote_admin"
    }
}

struct NativeBackup: Codable {
    var id: String
    var kind: String
    var target: String
    var label: String
    var configuredLabel: String
    var state: String
    var detail: String
    var enabled: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case kind
        case target
        case label
        case configuredLabel = "configured_label"
        case state
        case detail
        case enabled
    }
}

struct NativeRelayStatus: Codable {
    var url: String
    var status: String
    var statusLabel: String
    var health: String

    enum CodingKeys: String, CodingKey {
        case url
        case status
        case statusLabel = "status_label"
        case health
    }
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
    var statusLabel: String

    enum CodingKeys: String, CodingKey {
        case running
        case status
        case statusLabel = "status_label"
    }
}

struct NativeFipsStatus: Codable {
    var enabled: Bool
    var running: Bool
    var fresh: Bool
    var state: String
    var stateLabel: String
    var endpointNpub: String
    var discoveryScope: String
    var rosterLabel: String
    var rosterPeerCount: UInt64
    var rosterOnlineDeviceCount: UInt64
    var rosterDirectDeviceCount: UInt64
    var onlineDeviceCount: UInt64
    var directDeviceCount: UInt64
    var meshDeviceCount: UInt64
    var otherPeerCount: UInt64
    var onlineDevices: [String]
    var directDevices: [String]
    var meshDevices: [String]
    var peerStatuses: [NativeFipsPeerStatus]
    var error: String

    enum CodingKeys: String, CodingKey {
        case enabled
        case running
        case fresh
        case state
        case stateLabel = "state_label"
        case endpointNpub = "endpoint_npub"
        case discoveryScope = "discovery_scope"
        case rosterLabel = "roster_label"
        case rosterPeerCount = "roster_peer_count"
        case rosterOnlineDeviceCount = "roster_online_device_count"
        case rosterDirectDeviceCount = "roster_direct_device_count"
        case onlineDeviceCount = "online_device_count"
        case directDeviceCount = "direct_device_count"
        case meshDeviceCount = "mesh_device_count"
        case otherPeerCount = "other_peer_count"
        case onlineDevices = "online_devices"
        case directDevices = "direct_devices"
        case meshDevices = "mesh_devices"
        case peerStatuses = "peer_statuses"
        case error
    }
}

struct NativeFipsPeerStatus: Codable {
    var npub: String
    var transportType: String
    var srttMS: UInt64?
    var connectionLabel: String

    enum CodingKeys: String, CodingKey {
        case npub
        case transportType = "transport_type"
        case srttMS = "srtt_ms"
        case connectionLabel = "connection_label"
    }
}
