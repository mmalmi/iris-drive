package to.iris.drive.app

import android.content.Context
import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.provider.IrisDriveDocumentStore

@RunWith(AndroidJUnit4::class)
class ShareActivityInstrumentedTest {
    private lateinit var context: Context

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
        NativeCore.initializeAndroidContext(context)
        resetAppStorage()
        createProfile()
    }

    @After
    fun tearDown() {
        resetAppStorage()
    }

    @Test
    fun actionSendTextImportsSharedTextIntoProviderRoot() {
        val intent = Intent(Intent.ACTION_SEND)
            .setClass(context, ShareActivity::class.java)
            .setType("text/plain")
            .putExtra(Intent.EXTRA_SUBJECT, "Mobile share API")
            .putExtra(Intent.EXTRA_TEXT, "hello from Android share API\n")
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        ActivityScenario.launch<ShareActivity>(intent).use {
            val entry = waitForProviderEntry("Mobile share API.txt")
            assertNotNull("Shared text did not appear in Iris Drive provider root", entry)
            assertEquals("hello from Android share API\n", readDocumentText(entry!!.documentId))
        }
    }

    private fun waitForProviderEntry(displayName: String): to.iris.drive.app.provider.IrisDriveDocumentEntry? {
        val deadline = System.currentTimeMillis() + 5_000L
        var lastEntries = emptyList<to.iris.drive.app.provider.IrisDriveDocumentEntry>()
        while (System.currentTimeMillis() < deadline) {
            lastEntries = store().childDocuments(IrisDriveDocumentStore.ROOT_DOCUMENT_ID)
            lastEntries.firstOrNull { it.displayName == displayName }?.let { return it }
            Thread.sleep(100)
        }
        assertTrue("Provider entries after share: $lastEntries", lastEntries.isNotEmpty())
        return null
    }

    private fun readDocumentText(documentId: String): String {
        val file = store().readDocumentToTemp(documentId)
        return try {
            file.readText()
        } finally {
            file.delete()
        }
    }

    private fun store(): IrisDriveDocumentStore = IrisDriveDocumentStore(context.filesDir)

    private fun createProfile() {
        val handle = NativeCore.appNew(context.filesDir.absolutePath, "share-activity-test")
        try {
            NativeCore.dispatchJson(
                handle,
                JSONObject()
                    .put("type", "create_profile")
                    .put("app_key_label", "Android share API test")
                    .toString(),
            )
        } finally {
            NativeCore.appFree(handle)
        }
    }

    private fun resetAppStorage() {
        deleteChildren(context.filesDir)
        deleteChildren(context.cacheDir)
    }

    private fun deleteChildren(directory: File) {
        directory.listFiles()?.forEach { it.deleteRecursively() }
    }
}
