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
                  "display_label": "This Device",
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
        assertTrue(state.isSetupComplete)
        assertEquals("Ready", state.primaryStatusLabel)
        assertEquals("Up to date", state.sync.statusLabel)
        assertEquals("Running", state.fips.stateLabel)
        assertEquals("1/1 online", state.fips.rosterLabel)
        assertEquals(1, state.fips.rosterOnlineDeviceCount)
        assertEquals("TCP, 12 ms", state.fips.peerStatuses.single().connectionLabel)
        assertEquals("This Device", state.devices.single().displayLabel)
        assertEquals("device", state.devices.single().actorKind)
        assertEquals("Admin", state.devices.single().roleLabel)
        assertEquals("Linked", state.devices.single().stateLabel)
        assertEquals("local", state.devices.single().connectionState)
        assertEquals("This Device", state.devices.single().connectionLabel)
        assertEquals("{\"profile_id\":\"profile-a\"}", state.lastShareRecipientEvidence)
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
                appKeyLinkInvite = "iris-drive://invite/test",
                inboundAppKeyLinkRequests = emptyList(),
            ),
            roots = listOf(
                SyncRoot(
                    name = "My Drive",
                    localPath = "content://to.iris.drive.documents/document/root",
                    status = "ready",
                ),
            ),
            devices = listOf(
                DeviceState(
                    pubkey = "device-a",
                    label = "Pixel",
                    displayLabel = "This Device",
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
                appKeyLinkRequest = "iris-drive://app-key-link?app_key=device-a",
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
