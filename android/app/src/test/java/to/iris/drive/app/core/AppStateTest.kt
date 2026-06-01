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
                "setup_label": "Linked",
                "primary_status": "ready",
                "primary_status_label": "Ready",
                "devices": [{
                  "pubkey": "device-a",
                  "label": "Pixel",
                  "display_label": "This device",
                  "role": "admin",
                  "role_label": "Admin",
                  "state": "linked",
                  "state_label": "Linked",
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
        assertEquals("Ready", state.primaryStatusLabel)
        assertEquals("This device", state.devices.single().displayLabel)
        assertEquals("Admin", state.devices.single().roleLabel)
        assertEquals("Linked", state.devices.single().stateLabel)
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
                    role = "admin",
                    state = "Admin",
                    detail = "device-a",
                    isCurrentDevice = true,
                    isOnline = true,
                    canRevoke = false,
                    canAppointAdmin = false,
                    canDemoteAdmin = false,
                ),
                DeviceState(
                    pubkey = "device-b",
                    label = "Tablet",
                    role = "member",
                    state = "Linked",
                    detail = "device-b",
                    isCurrentDevice = false,
                    isOnline = false,
                    canRevoke = true,
                    canAppointAdmin = true,
                    canDemoteAdmin = false,
                ),
            ),
            setupState = "authorized",
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
