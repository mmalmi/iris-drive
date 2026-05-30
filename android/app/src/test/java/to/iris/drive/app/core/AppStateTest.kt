package to.iris.drive.app.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AppStateTest {
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
                    state = "Authorized",
                    detail = "device-b",
                    isCurrentDevice = false,
                    isOnline = false,
                    canRevoke = true,
                    canAppointAdmin = true,
                    canDemoteAdmin = false,
                ),
            ),
        )

        assertEquals(2, state.authorizedDeviceCount)
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
        )

        assertTrue(state.isAwaitingApproval)
        assertEquals(false, state.isSetupComplete)
        assertEquals(0, state.authorizedDeviceCount)
    }
}
