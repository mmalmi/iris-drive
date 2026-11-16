package to.iris.drive.app.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.json.JSONObject

class AppStateTest {
    @Test
    fun shareRecipientEvidenceActionSerializesForAppCore() {
        val action = JSONObject(
            NativeActions.inviteShareMemberFromEvidence(
                shareId = "share-1",
                evidenceJson = """{"profile_id":"profile-1"}""",
                role = "editor",
                displayName = "Alice",
            ),
        )

        assertEquals("invite_share_member_from_evidence", action.getString("type"))
        assertEquals("share-1", action.getString("share_id"))
        assertEquals("""{"profile_id":"profile-1"}""", action.getString("evidence_json"))
        assertEquals("editor", action.getString("role"))
        assertEquals("Alice", action.getString("display_name"))
    }

    @Test
    fun exportShareRecipientEvidenceActionSerializesForAppCore() {
        val action = JSONObject(
            NativeActions.exportShareRecipientEvidence(displayName = "Alice"),
        )

        assertEquals("export_share_recipient_evidence", action.getString("type"))
        assertEquals("Alice", action.getString("display_name"))
    }

    @Test
    fun pendingShareInviteActionSerializesForAppCore() {
        val action = JSONObject(
            NativeActions.recordPendingShareInvite(
                shareId = "share-1",
                representativeNpubHint = "npub1alice",
                role = "reader",
                displayName = "Alice",
            ),
        )

        assertEquals("record_pending_share_invite", action.getString("type"))
        assertEquals("share-1", action.getString("share_id"))
        assertEquals("npub1alice", action.getString("representative_npub_hint"))
        assertEquals("reader", action.getString("role"))
        assertEquals("Alice", action.getString("display_name"))
    }

    @Test
    fun appCoreLabelsAreParsedFromNativeState() {
        val state = AppState.fromJson(
            """
            {
              "ui": {
                "setup_state": "authorized",
                "setup_complete": true,
                "awaiting_approval": false,
                "revoked": false,
                "setup_label": "Linked",
                "primary_status": "ready",
                "primary_status_label": "Ready",
                "local_nhash_resolver_enabled": true,
                "sites_portal_url": "http://sites.npub1portal.iris.localhost:17321/",
                "last_share_recipient_evidence": "{\"profile_id\":\"profile-a\"}",
                "sync": {
                  "running": true,
                  "status": "up to date",
                  "status_label": "Up to date"
                },
                "fips": {
                  "enabled": true,
                  "running": true,
                  "fresh": true,
                  "state": "running",
                  "state_label": "Running",
                  "endpoint_npub": "device-a",
                  "discovery_scope": "iris-drive:test",
                  "roster_label": "1/1 online",
                  "roster_peer_count": 1,
                  "roster_online_device_count": 1,
                  "roster_direct_device_count": 1,
                  "online_device_count": 1,
                  "direct_device_count": 1,
                  "mesh_device_count": 0,
                  "other_peer_count": 0,
                  "peer_statuses": [{
                    "npub": "device-b",
                    "transport_type": "tcp",
                    "srtt_ms": 12,
                    "connection_label": "TCP, 12 ms"
                  }]
                },
                "app_actors": [{
                  "actor_kind": "device",
                  "pubkey": "device-a",
                  "label": "Pixel",
                  "display_label": "Pixel",
                  "role": "admin",
                  "role_label": "Admin",
                  "state": "linked",
                  "state_label": "Linked",
                  "connection_state": "local",
                  "connection_label": "This Device",
                  "detail": "device-a",
                  "is_current_app_key": true,
                  "is_online": true
                }]
              },
              "error": ""
            }
            """.trimIndent(),
        )

        assertEquals("Linked", state.setupLabel)
        assertTrue(state.isLoaded)
        assertTrue(state.isSetupComplete)
        assertEquals("Ready", state.primaryStatusLabel)
        assertTrue(state.localNhashResolverEnabled)
        assertEquals("http://sites.npub1portal.iris.localhost:17321/", state.sitesPortalUrl)
        assertEquals("Up to date", state.sync.statusLabel)
        assertEquals("Running", state.fips.stateLabel)
        assertEquals("1/1 online", state.fips.rosterLabel)
        assertEquals(1, state.fips.rosterOnlineDeviceCount)
        assertEquals("TCP, 12 ms", state.fips.peerStatuses.single().connectionLabel)
        assertEquals("Pixel", state.devices.single().displayLabel)
        assertEquals("device", state.devices.single().actorKind)
        assertEquals("Admin", state.devices.single().roleLabel)
        assertEquals("Linked", state.devices.single().stateLabel)
        assertEquals("local", state.devices.single().connectionState)
        assertEquals("This Device", state.devices.single().connectionLabel)
        assertEquals("{\"profile_id\":\"profile-a\"}", state.lastShareRecipientEvidence)
    }

    @Test
    fun deviceStorageIsParsedFromNativeState() {
        val state = AppState.fromJson(
            """
            {
              "ui": {
                "device_storage": {
                  "summary": "Your Drive is 2.3 GB in 1,204 files. All of it is on this device.",
                  "devices": [{
                    "pubkey": "device-a",
                    "npub": "npub1devicea",
                    "name": "Pixel",
                    "is_current": true,
                    "app": "android",
                    "icon": "phone",
                    "has_report": true,
                    "held_label": "All of it",
                    "holds_all": true,
                    "held_permille": 1000,
                    "connected": false,
                    "connection_label": "",
                    "connection_detail": "",
                    "sync_state": "",
                    "sync_label": "",
                    "sync_detail": "",
                    "space_label": "Uses 2.3 GB of 50.0 GB allowed",
                    "space_detail": "Free space on this device",
                    "age_label": "Updated 5 min ago",
                    "folders": [
                      {"label": "Photos", "depth": 0, "detail": "1.2 GB of 1.2 GB", "held_permille": 1000},
                      {"label": "2024", "depth": 1, "detail": "300.0 MB of 1.2 GB", "held_permille": 250}
                    ],
                    "notes": ["Some folders could not be read when it measured."]
                  }, {
                    "pubkey": "device-b",
                    "npub": "npub1deviceb",
                    "name": "MacBook",
                    "is_current": false,
                    "app": "",
                    "icon": "drive",
                    "has_report": false,
                    "held_label": "No report yet",
                    "holds_all": false,
                    "held_permille": 0,
                    "connected": true,
                    "connection_label": "Connected",
                    "connection_detail": "Linked directly",
                    "sync_state": "behind",
                    "sync_label": "Missing recent changes",
                    "sync_detail": "It has not seen the latest changes",
                    "space_label": "",
                    "space_detail": "",
                    "age_label": ""
                  }]
                }
              },
              "error": ""
            }
            """.trimIndent(),
        )

        val storage = state.deviceStorage
        assertEquals("Your Drive is 2.3 GB in 1,204 files. All of it is on this device.", storage.summary)
        assertEquals(2, storage.devices.size)
        val current = storage.devices[0]
        assertTrue(current.isCurrent)
        assertEquals("phone", current.icon)
        assertTrue(current.hasReport)
        assertTrue(current.holdsAll)
        assertEquals("All of it", current.heldLabel)
        assertEquals(1000, current.heldPermille)
        assertEquals("Uses 2.3 GB of 50.0 GB allowed", current.spaceLabel)
        assertEquals("Updated 5 min ago", current.ageLabel)
        assertEquals(listOf("Photos", "2024"), current.folders.map { it.label })
        assertEquals(1, current.folders[1].depth)
        assertEquals(250, current.folders[1].heldPermille)
        assertEquals("300.0 MB of 1.2 GB", current.folders[1].detail)
        assertEquals(listOf("Some folders could not be read when it measured."), current.notes)
        val other = storage.devices[1]
        assertEquals(false, other.isCurrent)
        assertEquals(false, other.hasReport)
        assertTrue(other.connected)
        assertEquals("Connected", other.connectionLabel)
        assertEquals("behind", other.syncState)
        assertEquals("Missing recent changes", other.syncLabel)
        assertEquals("It has not seen the latest changes", other.syncDetail)
        assertTrue(other.folders.isEmpty())
        assertTrue(other.notes.isEmpty())
    }

    @Test
    fun missingDeviceStorageDecodesAsEmpty() {
        val state = AppState.fromJson("""{"ui": {}, "error": ""}""")

        assertEquals("", state.deviceStorage.summary)
        assertTrue(state.deviceStorage.devices.isEmpty())
    }

    @Test
    fun defaultAppStateIsLoadedForFirstRunFixtures() {
        val state = AppState()

        assertTrue(state.isLoaded)
        assertEquals(false, state.isSetupComplete)
    }

    @Test
    fun recoveryActorKindFallsBackFromRoleForOlderNativeJson() {
        val state = AppState.fromJson(
            """
            {
              "ui": {
                "app_actors": [{
                  "pubkey": "npub1recovery",
                  "label": "Recovery key",
                  "display_label": "Recovery key",
                  "role": "recovery",
                  "role_label": "Recovery",
                  "state": "linked",
                  "state_label": "Linked",
                  "connection_state": "recovery",
                  "connection_label": "Recovery key",
                  "detail": "npub1recovery",
                  "is_current_app_key": false,
                  "is_online": false
                }]
              },
              "error": ""
            }
            """.trimIndent(),
        )

        assertEquals("recovery_key", state.devices.single().actorKind)
    }

    @Test
    fun shareSourcePathIsParsedFromNativeState() {
        val state = AppState.fromJson(
            """
            {
              "ui": {
                "shares": [{
                  "share_id": "123e4567-e89b-42d3-a456-426614174000",
                  "display_name": "Projects",
                  "source_path": "My Drive/Projects",
                  "shared_with_me_path": "Shared with me/Projects",
                  "role": "admin",
                  "role_label": "Admin",
                  "key_status": "available",
                  "key_status_label": "Available",
                  "write_authorization": "authorized",
                  "write_authorization_label": "Authorized",
                  "can_write": true,
                  "can_admin": true,
                  "current_key_epoch": 1,
                  "has_current_key_wrap": true,
                  "key_unavailable": false,
                  "repair_needed": false,
                  "missing_key_wraps": [],
                  "participant_count": 1,
                  "app_key_count": 1,
                  "members": [],
                  "shortcut_paths": ["My Drive/Projects shortcut"]
                }]
              },
              "error": ""
            }
            """.trimIndent(),
        )

        val share = state.shares.single()
        assertEquals("Projects", share.displayName)
        assertEquals("My Drive/Projects", share.sourcePath)
        assertEquals("Shared with me/Projects", share.sharedWithMePath)
        assertEquals(listOf("My Drive/Projects shortcut"), share.shortcutPaths)
    }

    @Test
    fun relayStatusesAreParsedFromNativeState() {
        val state = AppState.fromJson(
            """
            {
              "ui": {
                "relays": ["wss://relay.example"],
                "relay_statuses": [{
                  "url": "wss://relay.example",
                  "status": "connected",
                  "status_label": "connected",
                  "health": "online"
                }]
              },
              "error": ""
            }
            """.trimIndent(),
        )

        assertEquals("wss://relay.example", state.relayStatuses.single().url)
        assertEquals("connected", state.relayStatuses.single().status)
        assertEquals("connected", state.relayStatuses.single().statusLabel)
        assertEquals("online", state.relayStatuses.single().health)
    }

    @Test
    fun recoveryExportAndCapabilityAreParsedFromNativeJson() {
        val state = AppState.fromJson(
            """
            {
              "ui": {
                "profile": {
                  "current_app_key_npub": "npub1app",
                  "can_export_recovery_phrase": true
                }
              },
              "error": ""
            }
            """.trimIndent(),
        )
        val export = recoverySecretExportFromJson(
            """
            {
              "can_export": true,
              "recovery_phrase": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
              "words": ["abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "about"],
              "secret_key": "nsec1test"
            }
            """.trimIndent(),
        )

        assertTrue(state.profile?.canExportRecoveryPhrase == true)
        assertTrue(export.canExport)
        assertEquals(12, export.words.size)
        assertEquals("abandon", export.words.first())
        assertEquals("about", export.words.last())
        assertEquals("nsec1test", export.secretKey)
    }

    @Test
    fun deviceAdminStateFeedsDerivedStats() {
        val state = AppState(
            profile = ProfileState(
                profileId = "profile-a",
                currentAppKeyNpub = "app-key",
                devicePubkey = "device-a",
                appKeyLabel = "Pixel",
                authorizationState = "authorized",
                canAdminProfile = true,
                appKeyLinkRequest = "",
                appKeyLinkInvite = "https://drive.iris.to/invite/test",
                inboundAppKeyLinkRequests = emptyList(),
            ),
            roots = listOf(
                SyncRoot(
                    name = "My Drive",
                    localPath = "content://to.iris.drive.documents/document/root",
                ),
            ),
            devices = listOf(
                DeviceState(
                    pubkey = "device-a",
                    label = "Pixel",
                    displayLabel = "Pixel",
                    role = "admin",
                    roleLabel = "Admin",
                    state = "Admin",
                    stateLabel = "Linked",
                    detail = "device-a",
                    isCurrentDevice = true,
                    isOnline = true,
                    connectionState = "local",
                    connectionLabel = "This Device",
                    canRevoke = false,
                    canAppointAdmin = false,
                    canDemoteAdmin = false,
                ),
                DeviceState(
                    pubkey = "device-b",
                    label = "Tablet",
                    displayLabel = "Tablet",
                    role = "member",
                    roleLabel = "Member",
                    state = "Linked",
                    stateLabel = "Linked",
                    detail = "device-b",
                    isCurrentDevice = false,
                    isOnline = false,
                    connectionState = "offline",
                    connectionLabel = "Offline",
                    canRevoke = true,
                    canAppointAdmin = true,
                    canDemoteAdmin = false,
                ),
            ),
            setupState = "authorized",
            isSetupComplete = true,
            authorizedDeviceCount = 2,
            onlineDeviceCount = 1,
        )

        assertEquals(2, state.authorizedDeviceCount)
        assertEquals(1, state.onlineDeviceCount)
        assertTrue(state.isSetupComplete)
        assertEquals("admin", state.devices[0].role)
        assertTrue(state.devices[0].isCurrentDevice)
        assertTrue(state.devices[1].canAppointAdmin)
    }

    @Test
    fun pendingApprovalDoesNotCompleteSetup() {
        val state = AppState(
            profile = ProfileState(
                profileId = "profile-a",
                currentAppKeyNpub = "app-key",
                devicePubkey = "device-a",
                appKeyLabel = "Pixel",
                authorizationState = "awaiting_approval",
                canAdminProfile = false,
                appKeyLinkRequest = "https://drive.iris.to/approve-device/demo-request",
                appKeyLinkInvite = "",
                inboundAppKeyLinkRequests = emptyList(),
            ),
            setupState = "awaiting_approval",
            isAwaitingApproval = true,
        )

        assertTrue(state.isAwaitingApproval)
        assertEquals(false, state.isSetupComplete)
        assertEquals(0, state.authorizedDeviceCount)
    }

    @Test
    fun revokedDeviceDoesNotCompleteSetup() {
        val state = AppState(
            profile = ProfileState(
                profileId = "profile-a",
                currentAppKeyNpub = "app-key",
                devicePubkey = "device-a",
                appKeyLabel = "Pixel",
                authorizationState = "revoked",
                canAdminProfile = false,
                appKeyLinkRequest = "",
                appKeyLinkInvite = "",
                inboundAppKeyLinkRequests = emptyList(),
            ),
            setupState = "revoked",
            isRevoked = true,
        )

        assertTrue(state.isRevoked)
        assertEquals(false, state.isAwaitingApproval)
        assertEquals(false, state.isSetupComplete)
    }

    @Test
    fun deleteDeviceActionUsesDeleteDeviceType() {
        val action = JSONObject(NativeActions.deleteDevice("device-b"))

        assertEquals("delete_device", action.getString("type"))
        assertEquals("device-b", action.getString("app_key_pubkey"))
    }
}
