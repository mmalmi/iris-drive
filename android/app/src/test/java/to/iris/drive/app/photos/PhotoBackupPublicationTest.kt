package to.iris.drive.app.photos

import org.junit.Assert.*
import org.junit.Test

class PhotoBackupPublicationTest {
    @Test fun offlineAndMeteredNetworksKeepPendingBackupsVisible() {
        assertFalse(PhotoNetwork.Offline.allowsUpload(allowMetered = true))
        assertFalse(PhotoNetwork.Metered.allowsUpload(allowMetered = false))
        assertEquals("Saved in My Drive · waiting for Wi-Fi", publicationStatus(0, false, true, PhotoNetwork.Metered, false))
        assertEquals("Saved in My Drive · waiting to sync", publicationStatus(0, false, true, PhotoNetwork.Offline, true))
    }

    @Test fun allowedNetworksRetryFailuresAndFinishedUploadsReachUpToDate() {
        assertTrue(PhotoNetwork.Unmetered.allowsUpload(allowMetered = false))
        assertTrue(PhotoNetwork.Metered.allowsUpload(allowMetered = true))
        assertEquals("Saved in My Drive · waiting to sync", publicationStatus(0, false, false, PhotoNetwork.Unmetered, false))
        assertEquals("Up to date", publicationStatus(0, true, false, PhotoNetwork.Offline, false))
        assertTrue(publicationStatus(2, true, false, PhotoNetwork.Unmetered, false).contains("2 items"))
    }
}
