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
    @Published var hasOwnerSigningAuthority = false
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
    @Published var snapshotURL: String?
    @Published var fileCount: Int?
    @Published var topLevelEntries: Int?
    @Published var visibleFileBytes: Int64?
    @Published var localBlockCount = 0
    @Published var localBlockBytes: Int64 = 0
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
}

struct IrisDriveFipsStatus: Equatable {
    let enabled: Bool
    let running: Bool
    let fresh: Bool
    let endpointNpub: String?
    let discoveryScope: String?
    let rosterPeerCount: Int
    let rosterConnectedPeerCount: Int
    let connectedPeerCount: Int
    let otherPeerCount: Int
    let error: String?

    init(
        enabled: Bool = false,
        running: Bool = false,
        fresh: Bool = false,
        endpointNpub: String? = nil,
        discoveryScope: String? = nil,
        rosterPeerCount: Int = 0,
        rosterConnectedPeerCount: Int = 0,
        connectedPeerCount: Int = 0,
        otherPeerCount: Int = 0,
        error: String? = nil
    ) {
        self.enabled = enabled
        self.running = running
        self.fresh = fresh
        self.endpointNpub = endpointNpub
        self.discoveryScope = discoveryScope
        self.rosterPeerCount = rosterPeerCount
        self.rosterConnectedPeerCount = rosterConnectedPeerCount
        self.connectedPeerCount = connectedPeerCount
        self.otherPeerCount = otherPeerCount
        self.error = error
    }

    init(json: [String: Any]) {
        enabled = json["enabled"] as? Bool ?? false
        running = json["running"] as? Bool ?? false
        fresh = json["fresh"] as? Bool ?? false
        endpointNpub = json["endpoint_npub"] as? String
        discoveryScope = json["discovery_scope"] as? String
        rosterPeerCount = (json["roster_peer_count"] as? NSNumber)?.intValue ?? 0
        rosterConnectedPeerCount =
            (json["roster_connected_peer_count"] as? NSNumber)?.intValue ?? 0
        connectedPeerCount = (json["connected_peer_count"] as? NSNumber)?.intValue ?? 0
        otherPeerCount = (json["other_peer_count"] as? NSNumber)?.intValue ?? 0
        error = json["error"] as? String
    }

    var stateText: String {
        if error != nil {
            return "Error"
        }
        if enabled && fresh {
            return "Running"
        }
        if enabled || running {
            return "Stale"
        }
        return "Paused"
    }

    var rosterText: String {
        "\(rosterConnectedPeerCount)/\(rosterPeerCount) direct"
    }
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

struct IrisDriveBackupTarget: Identifiable, Equatable {
    let id: String
    let kind: String
    let target: String
    let label: String?
    let state: String
    let uploaded: Int?
    let totalHashes: Int?
    let checkState: String?
    let latencyMs: Int?
    let downloadBytesPerSecond: Int?
    let sampledHashes: Int?
    let missing: Int?
    let unknown: Int?

    init(json: [String: Any]) {
        id = json["id"] as? String ?? json["target"] as? String ?? UUID().uuidString
        kind = json["kind"] as? String ?? "backup"
        target = json["target"] as? String ?? ""
        label = json["label"] as? String
        if let lastSync = json["last_sync"] as? [String: Any] {
            state = lastSync["state"] as? String ?? "synced"
            uploaded = (lastSync["uploaded"] as? NSNumber)?.intValue
            totalHashes = (lastSync["total_hashes"] as? NSNumber)?.intValue
        } else {
            state = kind == "fips" ? "Pending" : "Ready"
            uploaded = nil
            totalHashes = nil
        }
        if let lastCheck = json["last_check"] as? [String: Any] {
            checkState = lastCheck["state"] as? String
            latencyMs = (lastCheck["latency_ms"] as? NSNumber)?.intValue
            downloadBytesPerSecond =
                (lastCheck["download_bytes_per_second"] as? NSNumber)?.intValue
            sampledHashes = (lastCheck["sampled_hashes"] as? NSNumber)?.intValue
            missing = (lastCheck["missing"] as? NSNumber)?.intValue
            unknown = (lastCheck["unknown"] as? NSNumber)?.intValue
        } else {
            checkState = nil
            latencyMs = nil
            downloadBytesPerSecond = nil
            sampledHashes = nil
            missing = nil
            unknown = nil
        }
    }

    var title: String {
        if let label, !label.isEmpty {
            return label
        }
        return kind == "fips" ? shortValue(target) : target
    }

    var detail: String {
        var parts = [kind == "fips" ? shortValue(target) : target]
        if let uploaded, let totalHashes {
            parts.append("\(uploaded)/\(totalHashes)")
        }
        if let checkState {
            parts.append("check \(checkState)")
        }
        if let latencyMs {
            parts.append("\(latencyMs) ms")
        }
        if let downloadBytesPerSecond {
            parts.append("\(Self.byteString(downloadBytesPerSecond))/s")
        }
        return parts.joined(separator: " | ")
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

    private static func byteString(_ bytes: Int) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }
}

private func shortValue(_ value: String) -> String {
    guard value.count > 32 else {
        return value
    }
    return "\(value.prefix(14))...\(value.suffix(10))"
}

struct IrisDrivePeerStatus: Identifiable, Equatable {
    let id: String
    let npub: String
    let label: String?
    let role: String
    let isCurrentDevice: Bool
    let authorized: Bool
    let fipsOnline: Bool
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
        role = json["role"] as? String ?? "member"
        isCurrentDevice = json["is_current_device"] as? Bool ?? false
        authorized = json["authorized"] as? Bool ?? false
        fipsOnline = json["fips_online"] as? Bool ?? false
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
