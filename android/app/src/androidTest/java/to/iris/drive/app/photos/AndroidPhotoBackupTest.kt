package to.iris.drive.app.photos

import android.content.ContentValues
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import androidx.test.core.app.ApplicationProvider
import androidx.test.filters.SdkSuppress
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.provider.IrisDriveDocumentStore

class AndroidPhotoBackupTest {
    @get:Rule val temp = TemporaryFolder()
    private val context: Context get() = ApplicationProvider.getApplicationContext()

    @Test fun consentAndReceiptsSurviveRestartAndStayScopedToProfile() {
        val path = temp.newFolder().resolve("backup.db")
        PhotoBackupJournal(context, path).use { journal ->
            assertNull(journal.enabledProfile())
            journal.enable("first")
            assertFalse(journal.allowMetered("first"))
            journal.setAllowMetered("first", true)
            journal.record("first", "photo:1", "v1")
        }
        PhotoBackupJournal(context, path).use { journal ->
            assertEquals("first", journal.enabledProfile())
            assertTrue(journal.allowMetered("first"))
            assertTrue(journal.contains("first", "photo:1", "v1"))
            assertFalse(journal.contains("second", "photo:1", "v1"))
            assertFalse(journal.contains("first", "photo:1", "v2"))
            journal.record("first", "photo:1", "v2")
            assertEquals(1L, journal.count("first"))
            journal.disable()
            assertNull(journal.enabledProfile())
            assertTrue(journal.contains("first", "photo:1", "v2"))
        }
    }

    @Test @SdkSuppress(minSdkVersion = Build.VERSION_CODES.Q)
    fun mediaStorePhotoReachesProviderOnceAndSurvivesRemovalFromLibrary() = runBlocking {
        val dataDir = temp.newFolder()
        NativeCore.initializeAndroidContext(context)
        val handle = NativeCore.appNew(dataDir.absolutePath, "photo-backup-test")
        val state = try {
            AppState.fromJson(NativeCore.dispatchJson(handle,
                """{"type":"create_profile","app_key_label":"Photo test"}"""))
        } finally {
            NativeCore.appFree(handle)
        }
        val profile = requireNotNull(state.profile)
        val bitmap = Bitmap.createBitmap(4, 4, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(Color.BLUE)
        val bytes = ByteArrayOutputStream().use {
            assertTrue(bitmap.compress(Bitmap.CompressFormat.JPEG, 90, it))
            it.toByteArray()
        }
        bitmap.recycle()
        val uri = insertTestPhoto(bytes)
        var removed = false
        try {
            PhotoBackupJournal(context, temp.newFolder().resolve("backup.db")).use { journal ->
                val importer = PhotoImporter(context, dataDir, journal, profile)
                var found: MediaAsset? = null
                visitPhotoLibrary(context, PhotoAccess(true, false, false)) {
                    if (it.uri == uri) found = it
                    found == null
                }
                val media = requireNotNull(found)
                importer.import(media)
                assertEquals(0L, journal.count(profile.profileId))
                journal.enable(profile.profileId)
                importer.import(media)
                importer.import(media)
                assertEquals(1L, journal.count(profile.profileId))
                val pending = JSONObject(NativeCore.photoBackupPublishJson(dataDir.absolutePath, profile.profileId, false))
                assertFalse(pending.optBoolean("ok"))
                assertTrue(pending.optBoolean("pending"))
                assertEquals(1, context.contentResolver.delete(uri, null, null))
                removed = true
                val store = IrisDriveDocumentStore(dataDir)
                val photo = store.childDocuments("root/Photos").single()
                val content = store.readDocumentToTemp(photo.documentId)
                try {
                    assertArrayEquals(bytes, content.readBytes())
                } finally {
                    content.delete()
                }
            }
        } finally {
            if (!removed) context.contentResolver.delete(uri, null, null)
        }
    }

    private fun insertTestPhoto(bytes: ByteArray): Uri {
        val collection = MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, "iris-photo-backup-test.jpg")
            put(MediaStore.MediaColumns.MIME_TYPE, "image/jpeg")
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = requireNotNull(context.contentResolver.insert(collection, values))
        requireNotNull(context.contentResolver.openOutputStream(uri)).use { it.write(bytes) }
        context.contentResolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
        return uri
    }
}
