package to.iris.drive.app

import android.content.Context
import android.os.SystemClock
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.util.UUID
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore

@RunWith(AndroidJUnit4::class)
class IrisDriveAndroidNativeStateTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val nativeHandles = mutableListOf<Long>()

    @After
    fun tearDown() {
        nativeHandles.toList().forEach { NativeCore.appFree(it) }
        nativeHandles.clear()
    }

    @Test
    fun acceptedLinkedDevicePersistsLoginAndFileCountAfterRestart() {
        val ownerDir = tempDataDir("iris-drive-owner")
        val ownerHandle = NativeCore.appNew(ownerDir.absolutePath, "ui-test").also(nativeHandles::add)
        val owner = dispatch(ownerHandle, NativeActions.createProfile("Mac"))
        val source = File(context.cacheDir, "owner-note-${UUID.randomUUID()}.txt")
        source.writeText("from owner")
        val write = JSONObject(
            NativeCore.providerWriteJson(
                ownerDir.absolutePath,
                "owner-note.txt",
                source.absolutePath,
            ),
        )
        assertTrue(write.optString("error"), write.optString("error").isBlank())
        assertEquals(write.toString(), 1, write.optInt("file_count"))

        val linkedDir = tempDataDir("iris-drive-linked")
        val linkedHandle = NativeCore.appNew(linkedDir.absolutePath, "ui-test").also(nativeHandles::add)
        val linked = dispatch(
            linkedHandle,
            NativeActions.linkDevice(owner.profile!!.appKeyLinkInvite, "Pixel"),
        )
        val approved = dispatch(
            ownerHandle,
            NativeActions.approveDevice(linked.profile!!.appKeyLinkRequest, "Pixel"),
        )
        assertTrue(approved.error, approved.error.isBlank())

        val applied = JSONObject(
            NativeCore.applyOwnerSnapshotForTest(
                ownerDir.absolutePath,
                linkedDir.absolutePath,
            ),
        )
        assertTrue(applied.optString("error"), applied.optString("error").isBlank())

        val beforeRestart = waitForAuthorizedState(linkedHandle, expectedFileCount = 1)
        assertEquals("Pixel", beforeRestart.profile?.appKeyLabel)
        assertEquals(1, beforeRestart.fileCount)

        NativeCore.appFree(linkedHandle)
        nativeHandles.remove(linkedHandle)

        val restartedHandle = NativeCore.appNew(linkedDir.absolutePath, "ui-test").also(nativeHandles::add)
        val restarted = waitForAuthorizedState(restartedHandle, expectedFileCount = 1)
        assertEquals("authorized", restarted.profile?.authorizationState)
        assertEquals("Pixel", restarted.profile?.appKeyLabel)
        assertEquals(1, restarted.fileCount)
    }

    private fun tempDataDir(prefix: String): File =
        File(context.cacheDir, "$prefix-${UUID.randomUUID()}").also { it.mkdirs() }

    private fun dispatch(handle: Long, action: String): AppState {
        NativeCore.dispatchJson(handle, action)
        return appState(handle)
    }

    private fun appState(handle: Long): AppState =
        AppState.fromJson(NativeCore.stateJson(handle))

    private fun refreshedAppState(handle: Long): AppState =
        AppState.fromJson(NativeCore.refreshJson(handle))

    private fun waitForAuthorizedState(handle: Long, expectedFileCount: Int? = null): AppState {
        var latest = refreshedAppState(handle)
        val deadline = SystemClock.elapsedRealtime() + 60_000
        while (SystemClock.elapsedRealtime() < deadline) {
            latest = refreshedAppState(handle)
            if (
                latest.profile?.authorizationState == "authorized" &&
                    (expectedFileCount == null || latest.fileCount == expectedFileCount)
            ) {
                return latest
            }
            Thread.sleep(250)
        }
        assertTrue(
            "expected authorized state with fileCount=$expectedFileCount, " +
                "lastAuthorization=${latest.profile?.authorizationState}, " +
                "lastFileCount=${latest.fileCount}, lastError=${latest.error}",
            false,
        )
        return latest
    }
}
