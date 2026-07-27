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
