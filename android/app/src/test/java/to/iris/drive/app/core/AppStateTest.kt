package to.iris.drive.app.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.json.JSONObject

class AppStateTest {
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
                "devices": [{
                  "pubkey": "device-a",
                  "label": "Pixel",
                  "display_label": "This device",
                  "role": "admin",
                  "role_label": "Admin",
                  "state": "linked",
                  "state_label": "Linked",
                  "connection_state": "local",
                  "connection_label": "This device",
                  "detail": "device-a",
                  "is_current_device": true,
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
        assertEquals("This device", state.devices.single().displayLabel)
        assertEquals("Admin", state.devices.single().roleLabel)
        assertEquals("Linked", state.devices.single().stateLabel)
        assertEquals("local", state.devices.single().connectionState)
        assertEquals("This device", state.devices.single().connectionLabel)
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
                "account": {
                  "owner_pubkey": "owner",
                  "device_pubkey": "owner",
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
              "recovery_phrase": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
              "words": ["abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "abandon", "art"],
              "secret_key": "nsec1test"
            }
            """.trimIndent(),
        )

        assertTrue(state.account?.canExportRecoveryPhrase == true)
        assertTrue(export.canExport)
        assertEquals(24, export.words.size)
        assertEquals("abandon", export.words.first())
        assertEquals("art", export.words.last())
        assertEquals("nsec1test", export.secretKey)
    }

    @Test
    fun deviceAdminStateFeedsDerivedStats() {
        val state = AppState(
            account = AccountState(
                ownerPubkey = "owner",
                devicePubkey = "device-a",
                deviceLabel = "Pixel",
                authorizationState = "authorized",
                hasOwnerSigningAuthority = true,
                deviceLinkRequest = "",
                deviceLinkInvite = "iris-drive://invite/test",
                inboundDeviceLinkRequests = emptyList(),
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
                    displayLabel = "This device",
                    role = "admin",
                    roleLabel = "Admin",
                    state = "Admin",
                    stateLabel = "Linked",
                    detail = "device-a",
                    isCurrentDevice = true,
                    isOnline = true,
                    connectionState = "local",
                    connectionLabel = "This device",
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
            account = AccountState(
                ownerPubkey = "owner",
                devicePubkey = "device-a",
                deviceLabel = "Pixel",
                authorizationState = "awaiting_approval",
                hasOwnerSigningAuthority = false,
                deviceLinkRequest = "iris-drive://device-link?device=device-a",
                deviceLinkInvite = "",
                inboundDeviceLinkRequests = emptyList(),
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
            account = AccountState(
                ownerPubkey = "owner",
                devicePubkey = "device-a",
                deviceLabel = "Pixel",
                authorizationState = "revoked",
                hasOwnerSigningAuthority = false,
                deviceLinkRequest = "",
                deviceLinkInvite = "",
                inboundDeviceLinkRequests = emptyList(),
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
        assertEquals("device-b", action.getString("device_pubkey"))
    }
}
