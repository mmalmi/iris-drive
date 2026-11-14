package to.iris.drive.app.photos

import android.Manifest.permission
import org.junit.Assert.assertEquals
import org.junit.Test

class PhotoAccessTest {
    private fun access(api: Int, vararg permissions: String) =
        PhotoAccess.fromGrants(api) { it in permissions }

    @Test fun noAccessByDefaultAcrossAndroidVersions() {
        for (api in listOf(26, 32, 33, 34, 36)) {
            assertEquals(PhotoAccess(false, false, false), access(api))
        }
    }

    @Test fun androidThirteenScopesImagesAndVideosIndependently() {
        assertEquals(PhotoAccess(true, false, false), access(33, permission.READ_MEDIA_IMAGES))
        assertEquals(PhotoAccess(false, true, false), access(33, permission.READ_MEDIA_VIDEO))
    }

    @Test fun androidFourteenAllowsLimitedLibraryOrMixedGrants() {
        val selected = permission.READ_MEDIA_VISUAL_USER_SELECTED
        assertEquals(PhotoAccess(false, false, true), access(34, selected))
        assertEquals(PhotoAccess(true, false, true), access(36, permission.READ_MEDIA_IMAGES, selected))
    }

    @Test fun legacyPermissionOnlyGrantsAccessThroughAndroidTwelve() {
        assertEquals(PhotoAccess(true, true, false), access(32, permission.READ_EXTERNAL_STORAGE))
        assertEquals(PhotoAccess(false, false, false), access(33, permission.READ_EXTERNAL_STORAGE))
    }
}
