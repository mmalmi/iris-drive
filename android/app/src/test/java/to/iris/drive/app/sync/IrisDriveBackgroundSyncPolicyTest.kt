package to.iris.drive.app.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.SyncState

class IrisDriveBackgroundSyncPolicyTest {
    @Test
    fun backgroundSyncIsScheduledForLinkedRunningProfiles() {
        val state = AppState(
            isSetupComplete = true,
            sync = SyncState(running = true),
        )

        assertTrue(BackgroundSyncPolicy.shouldSchedule(state))
        assertEquals(BackgroundSyncAction.RestartSync, BackgroundSyncPolicy.actionFor(state))
    }

    @Test
    fun backgroundSyncKeepsPendingAppKeyLinkRequestsMoving() {
        val state = AppState(
            isAwaitingApproval = true,
            sync = SyncState(running = true),
        )

        assertTrue(BackgroundSyncPolicy.shouldSchedule(state))
        assertEquals(BackgroundSyncAction.RefreshOnly, BackgroundSyncPolicy.actionFor(state))
    }

    @Test
    fun pausedOrRevokedProfilesDoNotScheduleBackgroundSync() {
        val paused = AppState(
            isSetupComplete = true,
            sync = SyncState(running = false),
        )
        val revoked = AppState(
            isSetupComplete = true,
            isRevoked = true,
            sync = SyncState(running = true),
        )

        assertFalse(BackgroundSyncPolicy.shouldSchedule(paused))
        assertEquals(BackgroundSyncAction.None, BackgroundSyncPolicy.actionFor(paused))
        assertFalse(BackgroundSyncPolicy.shouldSchedule(revoked))
        assertEquals(BackgroundSyncAction.None, BackgroundSyncPolicy.actionFor(revoked))
    }

    @Test
    fun androidCalendarSyncKeepsPeriodicJobsScheduledForPausedLinkedProfiles() {
        val state = AppState(
            isSetupComplete = true,
            sync = SyncState(running = false),
        )

        assertFalse(BackgroundSyncPolicy.shouldSchedule(state))
        assertTrue(BackgroundSyncPolicy.shouldSchedule(state, androidCalendarSyncActive = true))
        assertEquals(BackgroundSyncAction.None, BackgroundSyncPolicy.actionFor(state))
    }

    @Test
    fun androidCalendarSyncDoesNotScheduleRevokedProfiles() {
        val state = AppState(
            isSetupComplete = true,
            isRevoked = true,
            sync = SyncState(running = false),
        )

        assertFalse(BackgroundSyncPolicy.shouldSchedule(state, androidCalendarSyncActive = true))
    }
}
