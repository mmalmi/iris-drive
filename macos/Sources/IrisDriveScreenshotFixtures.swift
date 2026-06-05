import Foundation

enum IrisDriveScreenshotFixtures {
    static var enabled: Bool {
        #if DEBUG
        let arguments = Set(CommandLine.arguments)
        if arguments.contains("--iris-drive-screenshot-fixture")
            || arguments.contains("--iris-drive-fixture-mode") {
            return true
        }
        return environmentFlag("IRIS_DRIVE_MACOS_SCREENSHOT_FIXTURE")
            || environmentFlag("IRIS_DRIVE_MACOS_FIXTURE_MODE")
        #else
        return false
        #endif
    }

    static var tabArgument: String {
        argumentValue(after: "--iris-drive-screenshot-tab")
            ?? ProcessInfo.processInfo.environment["IRIS_DRIVE_MACOS_SCREENSHOT_TAB"]
            ?? "drive"
    }

    static func apply(to status: IrisDriveStatus = .shared) {
        status.message = "Sync on"
        status.daemonRunning = true
        status.initialized = true
        status.driveName = "My Drive"
        status.currentAppKeyNpub = fakeNpub("appkey")
        status.deviceNpub = fakeNpub("mac")
        status.deviceLinkInviteURL = "https://drive.iris.to/invite/demo-device-link"
        status.inboundDeviceLinkRequests = [
            IrisDriveDeviceLinkRequestStatus(json: [
                "device_npub": fakeNpub("ipad"),
                "label": "iPad Pro",
                "requested_at": 1_779_900_000,
                "url": "iris-drive://device-link?owner=demo&device=ipad",
            ])
        ]
        status.canAdminProfile = true
        status.setupState = "authorized"
        status.setupComplete = true
        status.setupLabel = "Linked"
        status.primaryStatus = "ready"
        status.primaryStatusLabel = "Ready"
        status.authorizedDeviceCount = 4
        status.onlineDeviceCount = 3
        status.workingDirectory = "/Users/demo/Iris Drive"
        status.configDirectory = "/Users/demo/Library/Application Support/Iris Drive"
        status.blocksDirectory = "/Users/demo/Library/Application Support/Iris Drive/Hashtree"
        status.rootCID = "nhash1demoirisdrivefiles"
        status.rootIsPrivate = true
        status.filesIrisURL = "https://drive.iris.to/#/demo/main"
        status.snapshotURL = "https://drive.iris.to/#/demo/snapshot"
        status.fileCount = 1284
        status.visibleFileBytes = Int64(41_600_000_000)
        status.relays = [
            "wss://relay.damus.io",
            "wss://relay.nostr.band",
            "wss://nos.lol",
        ]
        status.relayStatuses = status.relays.map {
            let status = $0.contains("nostr.band") ? "connecting" : "connected"
            return IrisDriveRelayStatus(
                url: $0,
                status: status,
                statusLabel: status,
                health: status == "connected" ? "online" : "connecting"
            )
        }
        status.blossomServers = [
            "https://blossom.primal.net",
            "https://cdn.satellite.earth",
        ]
        status.backupTargets = [
            IrisDriveBackupTarget(json: [
                "id": "home-server",
                "kind": "fips",
                "target": fakeNpub("server"),
                "label": "Home server",
                "title": "Home server",
                "state": "synced",
                "detail": "\(shortValue(fakeNpub("server"))) | 0/0 | check ok | 24 ms | 13.5 MB/s",
                "last_sync": [
                    "state": "synced",
                    "uploaded": 0,
                    "total_hashes": 0,
                ],
                "last_check": [
                    "state": "ok",
                    "latency_ms": 24,
                    "download_bytes_per_second": 14_200_000,
                ],
            ]),
            IrisDriveBackupTarget(json: [
                "id": "archive",
                "kind": "filesystem",
                "target": "/Volumes/Archive/Iris Drive",
                "label": "Archive disk",
                "title": "Archive disk",
                "state": "synced",
                "detail": "/Volumes/Archive/Iris Drive | 0/0",
                "last_sync": [
                    "state": "synced",
                    "uploaded": 0,
                    "total_hashes": 0,
                ],
            ]),
        ]
        status.fips = IrisDriveFipsStatus(
            enabled: true,
            running: true,
            fresh: true,
            state: "running",
            stateLabel: "Running",
            endpointNpub: fakeNpub("endpoint"),
            discoveryScope: "owner",
            rosterLabel: "3/4 online",
            rosterPeerCount: 4,
            rosterOnlineDeviceCount: 3,
            rosterDirectDeviceCount: 3,
            onlineDeviceCount: 3,
            directDeviceCount: 2,
            meshDeviceCount: 1,
            otherPeerCount: 0,
            error: nil
        )
        status.peers = [
            peer(
                id: "mac",
                label: "This Mac",
                role: "admin",
                current: true,
                online: true,
                hasRoot: true
            ),
            peer(
                id: "iphone",
                label: "iPhone",
                role: "admin",
                current: false,
                online: true,
                hasRoot: true
            ),
            peer(
                id: "windows",
                label: "Windows desktop",
                role: "member",
                current: false,
                online: true,
                hasRoot: true
            ),
            peer(
                id: "linux",
                label: "Linux server",
                role: "member",
                current: false,
                online: false,
                hasRoot: true
            ),
        ]
        status.lastUpload = IrisDriveUploadStatus(json: [
            "total_hashes": 0,
            "uploaded": 0,
            "already_present": 0,
        ])
        status.lastEvent = "Fixture state"
        status.copyStatus = nil
    }

    private static func peer(
        id: String,
        label: String,
        role: String,
        current: Bool,
        online: Bool,
        hasRoot: Bool
    ) -> IrisDrivePeerStatus {
        let connectionState = current ? "local" : (online ? "direct" : "offline")
        let connectionLabel = current ? "This device" : (online ? "Online" : "Offline")
        return IrisDrivePeerStatus(json: [
            "app_key_pubkey": "fixture-\(id)",
            "device_npub": fakeNpub(id),
            "label": label,
            "display_label": current ? "This device" : label,
            "role": role,
            "role_label": role == "admin" ? "Admin" : "Member",
            "is_current_device": current,
            "authorized": true,
            "fips_online": online,
            "connection_state": connectionState,
            "connection_label": connectionLabel,
            "has_root": hasRoot,
            "root_cid": hasRoot ? "nhash1\(id)rootdemo" : "",
            "root_private": true,
            "published_at": 1_779_900_000,
            "dck_generation": 2,
        ])
    }

    private static func fakeNpub(_ seed: String) -> String {
        let alphabet = Array("023456789acdefghjklmnpqrstuvwxyz")
        var output = "npub1"
        let scalars = Array(seed.unicodeScalars.map { Int($0.value) })
        for index in 0..<58 {
            let value = scalars[index % max(scalars.count, 1)] + index * 7
            output.append(alphabet[value % alphabet.count])
        }
        return output
    }

    private static func argumentValue(after flag: String) -> String? {
        let arguments = ProcessInfo.processInfo.arguments
        guard let index = arguments.firstIndex(of: flag),
              arguments.indices.contains(index + 1)
        else {
            return nil
        }
        return arguments[index + 1]
    }

    private static func environmentFlag(_ name: String) -> Bool {
        let value = ProcessInfo.processInfo.environment[name] ?? ""
        return ["1", "true", "yes", "on"].contains(
            value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        )
    }
}
