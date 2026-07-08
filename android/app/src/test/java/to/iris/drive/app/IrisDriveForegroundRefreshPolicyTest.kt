package to.iris.drive.app

import org.junit.Assert.assertEquals
import org.junit.Test
import to.iris.drive.app.core.AppKeyLinkRequestState
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.ProfileState

class IrisDriveForegroundRefreshPolicyTest {
    @Test
    fun foregroundRefreshStaysFastForSetupAndApprovalStates() {
        assertEquals(ForegroundRefreshFastMs, foregroundRefreshDelayMs(AppState(isLoaded = false)))
        assertEquals(ForegroundRefreshFastMs, foregroundRefreshDelayMs(AppState(isSetupComplete = false)))
        assertEquals(
            ForegroundRefreshFastMs,
            foregroundRefreshDelayMs(AppState(isSetupComplete = true, isAwaitingApproval = true)),
        )
        assertEquals(
            ForegroundRefreshFastMs,
            foregroundRefreshDelayMs(AppState(isSetupComplete = true, isRevoked = true)),
        )
    }

    @Test
    fun foregroundRefreshSlowsDownForAuthorizedIdleState() {
        val state = AppState(isSetupComplete = true)

        assertEquals(ForegroundRefreshIdleMs, foregroundRefreshDelayMs(state))
    }

    @Test
    fun foregroundRefreshStaysFastWhileAdminHasInboundApprovalRequests() {
        val state = AppState(
            isSetupComplete = true,
            profile = ProfileState(
                profileId = "profile",
                currentAppKeyNpub = "device-a",
                devicePubkey = "device-a",
                appKeyLabel = "Pixel",
                authorizationState = "authorized",
                canAdminProfile = true,
                appKeyLinkRequest = "",
                appKeyLinkInvite = "",
                inboundAppKeyLinkRequests = listOf(
                    AppKeyLinkRequestState(
                        devicePubkey = "device-b",
                        label = "Laptop",
                        requestedAt = 1,
                        requestLink = "irisdrive://request",
                    ),
                ),
            ),
        )

        assertEquals(ForegroundRefreshFastMs, foregroundRefreshDelayMs(state))
    }
}
