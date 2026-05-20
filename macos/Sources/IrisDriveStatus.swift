import Combine
import Foundation

final class IrisDriveStatus: ObservableObject {
    static let shared = IrisDriveStatus()
    static let closeToMenuBarOnCloseKey = "closeToMenuBarOnClose"

    @Published var message = "Starting sync"
    @Published var daemonRunning = false
    @Published var closeToMenuBarOnClose =
        UserDefaults.standard.object(forKey: closeToMenuBarOnCloseKey) as? Bool ?? true
    @Published var initialized = false
    @Published var driveName = "My Drive"
    @Published var ownerNpub: String?
    @Published var deviceNpub: String?
    @Published var authorizationState: String?
    @Published var rosterSize = 0
    @Published var authorizedDeviceCount = 0
    @Published var publishedDeviceRoots = 0
    @Published var workingDirectory: String?
    @Published var configDirectory: String?
    @Published var blocksDirectory: String?
    @Published var rootCID: String?
    @Published var rootIsPrivate: Bool?
    @Published var filesIrisURL: String?
    @Published var topLevelEntries: Int?
    @Published var localBlockCount = 0
    @Published var localBlockBytes: Int64 = 0
    @Published var relays: [String] = []
    @Published var relayStatuses: [IrisDriveRelayStatus] = []
    @Published var blossomServers: [String] = []
    @Published var peers: [IrisDrivePeerStatus] = []
    @Published var lastUpload: IrisDriveUploadStatus?
    @Published var lastEvent: String?
    @Published var copyStatus: String?
}

struct IrisDriveRelayStatus: Identifiable, Equatable {
    let id: String
    let url: String
    let status: String

    init(url: String, status: String) {
        id = url
        self.url = url
        self.status = status
    }

    init(json: [String: Any]) {
        let url = json["url"] as? String ?? UUID().uuidString
        id = url
        self.url = url
        status = json["status"] as? String ?? "unknown"
    }
}

struct IrisDrivePeerStatus: Identifiable, Equatable {
    let id: String
    let npub: String
    let label: String?
    let isCurrentDevice: Bool
    let authorized: Bool
    let hasRoot: Bool
    let rootCID: String?
    let rootIsPrivate: Bool?
    let publishedAt: Int?
    let dckGeneration: Int?

    init(json: [String: Any]) {
        let pubkey = json["device_pubkey"] as? String ?? UUID().uuidString
        id = pubkey
        npub = json["device_npub"] as? String ?? pubkey
        label = json["label"] as? String
        isCurrentDevice = json["is_current_device"] as? Bool ?? false
        authorized = json["authorized"] as? Bool ?? false
        hasRoot = json["has_root"] as? Bool ?? false
        rootCID = json["root_cid"] as? String
        rootIsPrivate = json["root_private"] as? Bool
        publishedAt = (json["published_at"] as? NSNumber)?.intValue
        dckGeneration = (json["dck_generation"] as? NSNumber)?.intValue
    }
}

struct IrisDriveUploadStatus: Equatable {
    let totalHashes: Int
    let uploaded: Int
    let alreadyPresent: Int

    init(json: [String: Any]) {
        totalHashes = (json["total_hashes"] as? NSNumber)?.intValue ?? 0
        uploaded = (json["uploaded"] as? NSNumber)?.intValue ?? 0
        alreadyPresent = (json["already_present"] as? NSNumber)?.intValue ?? 0
    }
}
