import Combine
import Foundation

final class IrisDriveStatus: ObservableObject {
    static let shared = IrisDriveStatus()
    static let closeToMenuBarOnCloseKey = "closeToMenuBarOnClose"

    @Published var message = "Setup needed"
    @Published var daemonRunning = false
    @Published var closeToMenuBarOnClose =
        UserDefaults.standard.object(forKey: closeToMenuBarOnCloseKey) as? Bool ?? true
    @Published var localNhashResolverEnabled = true
    @Published var initialized = false
    @Published var driveName = "My Drive"
    @Published var ownerNpub: String?
    @Published var deviceNpub: String?
    @Published var deviceLinkInviteURL: String?
    @Published var inboundDeviceLinkRequests: [IrisDriveDeviceLinkRequestStatus] = []
    @Published var hasOwnerSigningAuthority = false
    @Published var setupState = "not_configured"
    @Published var setupLabel = "Not linked"
    @Published var primaryStatus = "not_setup"
    @Published var primaryStatusLabel = "Ready"
    @Published var syncStatus = "paused"
    @Published var syncStatusLabel = "Sync paused"
    @Published var authorizedDeviceCount = 0
    @Published var onlineDeviceCount = 0
    @Published var workingDirectory: String?
    @Published var configDirectory: String?
    @Published var blocksDirectory: String?
    @Published var rootCID: String?
    @Published var rootIsPrivate: Bool?
    @Published var filesIrisURL: String?
    @Published var snapshotURL: String?
    @Published var fileCount: Int?
    @Published var visibleFileBytes: Int64?
    @Published var relays: [String] = []
    @Published var relayStatuses: [IrisDriveRelayStatus] = []
    @Published var blossomServers: [String] = []
    @Published var backupTargets: [IrisDriveBackupTarget] = []
    @Published var fips = IrisDriveFipsStatus()
    @Published var peers: [IrisDrivePeerStatus] = []
    @Published var lastUpload: IrisDriveUploadStatus?
    @Published var lastEvent: String?
    @Published var copyStatus: String?

    var snapshotLinkURL: String? {
        guard let snapshotURL, !snapshotURL.isEmpty else {
            return nil
        }
        return snapshotURL
    }

    var setupComplete: Bool {
        setupState == "authorized"
    }

    var awaitingApproval: Bool {
        setupState == "awaiting_approval"
    }

    var revoked: Bool {
        setupState == "revoked"
    }
}

struct IrisDriveDeviceLinkRequestStatus: Identifiable, Equatable {
    let id: String
    let deviceNpub: String
    let label: String?
    let requestedAt: Int?
    let requestURL: String

    init(json: [String: Any]) {
        deviceNpub = json["device_npub"] as? String ?? UUID().uuidString
        id = deviceNpub
        label = json["label"] as? String
        requestedAt = (json["requested_at"] as? NSNumber)?.intValue
        requestURL = json["url"] as? String ?? deviceNpub
    }
}

struct IrisDriveFipsStatus: Equatable {
    let enabled: Bool
    let running: Bool
    let fresh: Bool
    let state: String
    let stateLabel: String
    let endpointNpub: String?
    let discoveryScope: String?
    let rosterLabel: String
    let rosterPeerCount: Int
    let rosterOnlineDeviceCount: Int
    let rosterDirectDeviceCount: Int
    let onlineDeviceCount: Int
    let directDeviceCount: Int
    let meshDeviceCount: Int
    let otherPeerCount: Int
    let peerStatuses: [IrisDriveFipsPeerStatus]
    let error: String?

    init(
        enabled: Bool = false,
        running: Bool = false,
        fresh: Bool = false,
        state: String = "paused",
        stateLabel: String = "Paused",
        endpointNpub: String? = nil,
        discoveryScope: String? = nil,
        rosterLabel: String = "0/0 online",
        rosterPeerCount: Int = 0,
        rosterOnlineDeviceCount: Int = 0,
        rosterDirectDeviceCount: Int = 0,
        onlineDeviceCount: Int = 0,
        directDeviceCount: Int = 0,
        meshDeviceCount: Int = 0,
        otherPeerCount: Int = 0,
        peerStatuses: [IrisDriveFipsPeerStatus] = [],
        error: String? = nil
    ) {
        self.enabled = enabled
        self.running = running
        self.fresh = fresh
        self.state = state
        self.stateLabel = stateLabel
        self.endpointNpub = endpointNpub
        self.discoveryScope = discoveryScope
        self.rosterLabel = rosterLabel
        self.rosterPeerCount = rosterPeerCount
        self.rosterOnlineDeviceCount = rosterOnlineDeviceCount
        self.rosterDirectDeviceCount = rosterDirectDeviceCount
        self.onlineDeviceCount = onlineDeviceCount
        self.directDeviceCount = directDeviceCount
        self.meshDeviceCount = meshDeviceCount
        self.otherPeerCount = otherPeerCount
        self.peerStatuses = peerStatuses
        self.error = error
    }

    init(json: [String: Any]) {
        enabled = json["enabled"] as? Bool ?? false
        running = json["running"] as? Bool ?? false
        fresh = json["fresh"] as? Bool ?? false
        state = json["state"] as? String ?? ""
        stateLabel = json["state_label"] as? String ?? ""
        endpointNpub = json["endpoint_npub"] as? String
        discoveryScope = json["discovery_scope"] as? String
        rosterLabel = json["roster_label"] as? String ?? ""
        rosterPeerCount = (json["roster_peer_count"] as? NSNumber)?.intValue ?? 0
        rosterOnlineDeviceCount =
            (json["roster_online_device_count"] as? NSNumber)?.intValue
            ?? 0
        rosterDirectDeviceCount =
            (json["roster_direct_device_count"] as? NSNumber)?.intValue
            ?? 0
        onlineDeviceCount =
            (json["online_device_count"] as? NSNumber)?.intValue
            ?? 0
        directDeviceCount =
            (json["direct_device_count"] as? NSNumber)?.intValue
            ?? 0
        meshDeviceCount =
            (json["mesh_device_count"] as? NSNumber)?.intValue
            ?? 0
        otherPeerCount = (json["other_peer_count"] as? NSNumber)?.intValue ?? 0
        peerStatuses = (json["peer_statuses"] as? [[String: Any]] ?? []).map(IrisDriveFipsPeerStatus.init)
        error = json["error"] as? String
    }
}

struct IrisDriveFipsPeerStatus: Identifiable, Equatable {
    let id: String
    let npub: String
    let transportType: String?
    let srttMS: Int?
    let connectionLabel: String

    init(json: [String: Any]) {
        npub = json["npub"] as? String ?? UUID().uuidString
        id = npub
        transportType = json["transport_type"] as? String
        srttMS = (json["srtt_ms"] as? NSNumber)?.intValue
        connectionLabel = json["connection_label"] as? String ?? ""
    }
}

struct IrisDriveRelayStatus: Identifiable, Equatable {
    let id: String
    let url: String
    let status: String
    let statusLabel: String
    let health: String

    init(url: String, status: String, statusLabel: String, health: String) {
        id = url
        self.url = url
        self.status = status
        self.statusLabel = statusLabel
        self.health = health
    }

    init(json: [String: Any]) {
        let url = json["url"] as? String ?? UUID().uuidString
        id = url
        self.url = url
        status = json["status"] as? String ?? "unknown"
        statusLabel = json["status_label"] as? String ?? ""
        health = json["health"] as? String ?? "unknown"
    }
}

struct IrisDriveBackupTarget: Identifiable, Equatable {
    let id: String
    let kind: String
    let target: String
    let label: String?
    let title: String
    let detail: String
    let state: String
    let uploaded: Int?
    let totalHashes: Int?
    let checkState: String?
    let checkedAt: Int?
    let latencyMs: Int?
    let downloadBytesPerSecond: Int?
    let sampledHashes: Int?
    let missing: Int?
    let unknown: Int?
    let error: String?

    init(json: [String: Any]) {
        id = json["id"] as? String ?? json["target"] as? String ?? UUID().uuidString
        kind = json["kind"] as? String ?? "backup"
        target = json["target"] as? String ?? ""
        label = json["label"] as? String
        title = json["title"] as? String ?? "Backup"
        detail = json["detail"] as? String ?? target
        if let lastSync = json["last_sync"] as? [String: Any] {
            state = json["state"] as? String ?? lastSync["state"] as? String ?? "synced"
            uploaded = (lastSync["uploaded"] as? NSNumber)?.intValue
            totalHashes = (lastSync["total_hashes"] as? NSNumber)?.intValue
        } else {
            state = json["state"] as? String ?? (kind == "fips" ? "pending" : "ready")
            uploaded = nil
            totalHashes = nil
        }
        if let lastCheck = json["last_check"] as? [String: Any] {
            checkState = lastCheck["state"] as? String
            checkedAt = (lastCheck["checked_at"] as? NSNumber)?.intValue
            latencyMs = (lastCheck["latency_ms"] as? NSNumber)?.intValue
            downloadBytesPerSecond =
                (lastCheck["download_bytes_per_second"] as? NSNumber)?.intValue
            sampledHashes = (lastCheck["sampled_hashes"] as? NSNumber)?.intValue
            missing = (lastCheck["missing"] as? NSNumber)?.intValue
            unknown = (lastCheck["unknown"] as? NSNumber)?.intValue
            error = lastCheck["error"] as? String
        } else {
            checkState = nil
            checkedAt = nil
            latencyMs = nil
            downloadBytesPerSecond = nil
            sampledHashes = nil
            missing = nil
            unknown = nil
            error = nil
        }
    }

    var iconName: String {
        switch kind {
        case "fips":
            return "person.badge.shield.checkmark.fill"
        case "filesystem":
            return "externaldrive.fill"
        case "lmdb":
            return "cylinder.split.1x2.fill"
        default:
            return "cloud.fill"
        }
    }
}

func shortValue(_ value: String) -> String {
    guard value.count > 32 else {
        return value
    }
    return "\(value.prefix(14))...\(value.suffix(10))"
}

struct IrisDrivePeerStatus: Identifiable, Equatable {
    let id: String
    let npub: String
    let label: String?
    let displayLabel: String
    let role: String
    let roleLabel: String
    let isCurrentDevice: Bool
    let authorized: Bool
    let fipsOnline: Bool
    let connectionState: String
    let connectionLabel: String
    let hasRoot: Bool
    let rootCID: String?
    let rootIsPrivate: Bool?
    let publishedAt: Int?
    let dckGeneration: Int?

    init(json: [String: Any]) {
        let pubkey = json["device_pubkey"] as? String ?? UUID().uuidString
        let labelValue = json["label"] as? String
        let isCurrentDeviceValue = json["is_current_device"] as? Bool ?? false
        let authorizedValue = json["authorized"] as? Bool ?? false
        let fipsOnlineValue = json["fips_online"] as? Bool ?? false
        id = pubkey
        npub = json["device_npub"] as? String ?? pubkey
        label = labelValue
        displayLabel = json["display_label"] as? String ?? ""
        role = json["role"] as? String ?? ""
        roleLabel = json["role_label"] as? String ?? ""
        isCurrentDevice = isCurrentDeviceValue
        authorized = authorizedValue
        fipsOnline = fipsOnlineValue
        connectionState = json["connection_state"] as? String ?? ""
        connectionLabel = json["connection_label"] as? String ?? ""
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

    var completedHashes: Int {
        min(totalHashes, uploaded + alreadyPresent)
    }

    var isInProgress: Bool {
        totalHashes > 0 && completedHashes < totalHashes
    }
}
