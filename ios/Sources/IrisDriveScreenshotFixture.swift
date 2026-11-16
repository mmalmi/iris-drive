import Foundation

extension IrisDriveMobileModel {
    var screenshotFixtureEnabled: Bool {
        #if DEBUG
        ProcessInfo.processInfo.environment[
            "IRIS_DRIVE_UI_TEST_SCREENSHOT_FIXTURE"
        ] == "1"
        #else
        false
        #endif
    }

    func applyScreenshotFixtureIfRequested() {
        #if DEBUG
        guard screenshotFixtureEnabled else {
            return
        }

        driveName = "Iris Drive"
        statusTitle = "Up to date"
        statusDetail = "All changes are synced across your devices."
        deviceLabel = "iPhone"
        devicePublicKey = "device1irisdriveiphone"
        authorizationState = "Linked"
        syncRunning = true
        authorizedDeviceCount = 3
        onlineDeviceCount = 3
        fileCount = 247
        visibleFileBytes = 9_234_567_890
        fileProviderStatus = "Available in Files"
        localNhashResolverEnabled = true

        devices = [
            IrisDriveDevice(
                label: "iPhone",
                actorKind: "device",
                role: "Admin",
                state: "Linked",
                connectionState: "online",
                connectionLabel: "Online",
                detail: "device1irisdriveiphone",
                isCurrentDevice: true,
                isOnline: true,
                canRevoke: false,
                canAppointAdmin: false,
                canDemoteAdmin: false
            ),
            IrisDriveDevice(
                label: "MacBook",
                actorKind: "device",
                role: "Admin",
                state: "Linked",
                connectionState: "online",
                connectionLabel: "Online",
                detail: "device1irisdrivemacbook",
                isCurrentDevice: false,
                isOnline: true,
                canRevoke: true,
                canAppointAdmin: false,
                canDemoteAdmin: true
            ),
            IrisDriveDevice(
                label: "Home server",
                actorKind: "device",
                role: "Member",
                state: "Linked",
                connectionState: "online",
                connectionLabel: "Online",
                detail: "device1irisdrivehomeserver",
                isCurrentDevice: false,
                isOnline: true,
                canRevoke: true,
                canAppointAdmin: true,
                canDemoteAdmin: false
            ),
        ]

        deviceStorage = NativeDeviceStorage(
            summary: "Your Drive is 9.2 GB in 247 files. All of it is on 2 devices.",
            devices: [
                NativeDeviceStorageRow(
                    pubkey: "fixture-iphone",
                    name: "iPhone",
                    isCurrent: true,
                    app: "ios",
                    icon: "phone",
                    hasReport: true,
                    heldLabel: "All of it",
                    holdsAll: true,
                    heldPermille: 1000,
                    spaceLabel: "Uses 9.2 GB of 64.0 GB allowed",
                    ageLabel: "Updated just now",
                    folders: [
                        NativeDeviceStorageFolder(label: "Photos", depth: 0, detail: "All 6.1 GB", heldPermille: 1000),
                        NativeDeviceStorageFolder(label: "Projects", depth: 0, detail: "All 3.1 GB", heldPermille: 1000),
                    ]
                ),
                NativeDeviceStorageRow(
                    pubkey: "fixture-macbook",
                    name: "MacBook",
                    app: "macos",
                    icon: "computer",
                    hasReport: true,
                    heldLabel: "All of it",
                    holdsAll: true,
                    heldPermille: 1000,
                    connected: true,
                    connectionLabel: "Connected",
                    syncState: "in-sync",
                    syncLabel: "In sync",
                    spaceLabel: "Uses 9.2 GB of 200.0 GB allowed",
                    ageLabel: "Updated 5 min ago"
                ),
                NativeDeviceStorageRow(
                    pubkey: "fixture-home-server",
                    name: "Home server",
                    app: "linux",
                    icon: "drive",
                    hasReport: true,
                    heldLabel: "6.1 GB · 66%",
                    heldPermille: 663,
                    connectionLabel: "Not connected",
                    syncState: "behind",
                    syncLabel: "Missing recent changes",
                    spaceLabel: "Uses 6.1 GB of 500.0 GB allowed",
                    ageLabel: "Updated 3 h ago",
                    folders: [
                        NativeDeviceStorageFolder(label: "Photos", depth: 0, detail: "All 6.1 GB", heldPermille: 1000),
                        NativeDeviceStorageFolder(label: "Projects", depth: 0, detail: "0 B of 3.1 GB", heldPermille: 0),
                    ]
                ),
            ]
        )

        relayStatuses = [
            IrisDriveRelayStatus(
                url: "wss://relay.iris.to",
                status: "connected",
                statusLabel: "Connected",
                health: "online"
            ),
        ]
        relays = ["wss://relay.iris.to"]
        relay = "wss://relay.iris.to"

        backups = [
            IrisDriveBackup(
                id: "fixture-home-server",
                kind: "fips",
                target: "fips://home-server",
                label: "Home server",
                configuredLabel: "Home server",
                state: "Up to date",
                detail: "247 files verified",
                enabled: true
            ),
            IrisDriveBackup(
                id: "fixture-archive",
                kind: "filesystem",
                target: "file:///Iris%20Archive",
                label: "Iris Archive",
                configuredLabel: "Iris Archive",
                state: "Up to date",
                detail: "Last checked today",
                enabled: true
            ),
        ]

        let ownerProfileId = localProfileId.isEmpty ? "fixture-owner" : localProfileId
        shares = [
            IrisDriveShare(
                shareId: "fixture-family-photos",
                displayName: "Family Photos",
                sourcePath: "/Photos/Family",
                sharedWithMePath: "",
                role: "admin",
                roleLabel: "Admin",
                keyStatus: "ready",
                keyStatusLabel: "Encrypted",
                writeAuthorization: "authorized",
                writeAuthorizationLabel: "Can edit",
                canWrite: true,
                canAdmin: true,
                currentKeyEpoch: 4,
                hasCurrentKeyWrap: true,
                keyUnavailable: false,
                repairNeeded: false,
                missingKeyWraps: [],
                participantCount: 3,
                appKeyCount: 5,
                members: [
                    IrisDriveShareMember(
                        profileId: ownerProfileId,
                        name: "You",
                        detail: "Admin",
                        displayName: "You",
                        representativeNpubHint: "npub1you",
                        role: "admin",
                        roleLabel: "Admin",
                        status: "active",
                        statusLabel: "Active",
                        appKeyCount: 3
                    ),
                    IrisDriveShareMember(
                        profileId: "fixture-alex",
                        name: "Alex",
                        detail: "Editor",
                        displayName: "Alex",
                        representativeNpubHint: "npub1alex",
                        role: "editor",
                        roleLabel: "Editor",
                        status: "active",
                        statusLabel: "Active",
                        appKeyCount: 1
                    ),
                    IrisDriveShareMember(
                        profileId: "fixture-maya",
                        name: "Maya",
                        detail: "Reader",
                        displayName: "Maya",
                        representativeNpubHint: "npub1maya",
                        role: "reader",
                        roleLabel: "Reader",
                        status: "active",
                        statusLabel: "Active",
                        appKeyCount: 1
                    ),
                ],
                pendingInvites: [],
                shortcutPaths: ["/Shared/Family Photos"]
            ),
            IrisDriveShare(
                shareId: "fixture-project",
                displayName: "Studio Project",
                sourcePath: "/Projects/Studio",
                sharedWithMePath: "",
                role: "admin",
                roleLabel: "Admin",
                keyStatus: "ready",
                keyStatusLabel: "Encrypted",
                writeAuthorization: "authorized",
                writeAuthorizationLabel: "Can edit",
                canWrite: true,
                canAdmin: true,
                currentKeyEpoch: 2,
                hasCurrentKeyWrap: true,
                keyUnavailable: false,
                repairNeeded: false,
                missingKeyWraps: [],
                participantCount: 2,
                appKeyCount: 3,
                members: [],
                pendingInvites: [],
                shortcutPaths: ["/Shared/Studio Project"]
            ),
        ]
        #endif
    }
}
