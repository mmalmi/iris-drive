package to.iris.drive.app.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidSelfUpdateTest {
    @Test
    fun parsesNativeUpdateResult() {
        val result =
            parseUpdateResult(
                """
                {
                  "available": true,
                  "latest_version": "0.2.0",
                  "tag": "v0.2.0",
                  "asset": "iris-drive-v0.2.0-android-arm64.apk",
                  "path": "/tmp/iris-drive.apk",
                  "error": ""
                }
                """.trimIndent(),
            )

        assertTrue(result.available)
        assertEquals("v0.2.0", result.tag)
        assertEquals("iris-drive-v0.2.0-android-arm64.apk", result.asset)
        assertEquals("/tmp/iris-drive.apk", result.path)
    }

    @Test
    fun updateButtonTextFollowsState() {
        assertEquals("Check for updates", AndroidSelfUpdateState().buttonText())
        assertEquals("Checking...", AndroidSelfUpdateState(checking = true).buttonText())
        assertEquals("Downloading...", AndroidSelfUpdateState(downloading = true).buttonText())
        assertEquals("Download update", AndroidSelfUpdateState(available = true).buttonText())
        assertEquals("Install update", AndroidSelfUpdateState(downloaded = true).buttonText())
    }
}
