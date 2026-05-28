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
                deviceLinkRequest = "iris-drive://device-link?owner=owner&device=device-a",
            ),
            roots = listOf(
                SyncRoot(
                    name = "My Drive",
                    localPath = "content://to.iris.drive.documents/document/root",
                    status = "SAF provider root",
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

        assertEquals(1, state.topLevelEntries)
        assertEquals(1, state.publishedDeviceRoots)
        assertEquals(2, state.authorizedDeviceCount)
        assertEquals("admin", state.devices[0].role)
        assertTrue(state.devices[0].isCurrentDevice)
        assertTrue(state.devices[1].canAppointAdmin)
    }
}
