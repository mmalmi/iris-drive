package to.iris.drive.app.provider

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject
import to.iris.drive.app.BuildConfig
import to.iris.drive.app.core.NativeCore

@RunWith(AndroidJUnit4::class)
class IrisDriveDocumentsProviderContractTest {
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
    fun documentsContractClientCreatesWritesReadsRenamesAndDeletesFile() {
        val resolver = context.contentResolver
        val authority = BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY
        val rootUri = DocumentsContract.buildDocumentUri(
            authority,
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
        )

        assertRootAdvertisesWritableIrisDrive(authority)

        val createdUri = DocumentsContract.createDocument(
            resolver,
            rootUri,
            "text/plain",
            "contract-write.txt",
        )
        assertNotNull("DocumentsContract.createDocument returned null", createdUri)
        val created = createdUri!!

        resolver.openOutputStream(created, "wt").use { stream ->
            assertNotNull("openOutputStream returned null", stream)
            stream!!.write("hello from android documents contract\n".toByteArray())
        }
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()

        assertEquals(
            "hello from android documents contract\n",
            waitForDocumentText(created),
        )
        assertDocument(created, displayName = "contract-write.txt", size = 38)

        val renamedUri = DocumentsContract.renameDocument(
            resolver,
            created,
            "contract-renamed.txt",
        )
        assertNotNull("DocumentsContract.renameDocument returned null", renamedUri)
        val renamed = renamedUri!!
        assertDocument(renamed, displayName = "contract-renamed.txt", size = 38)
        assertEquals(
            "hello from android documents contract\n",
            waitForDocumentText(renamed),
        )

        assertTrue(DocumentsContract.deleteDocument(resolver, renamed))
        assertFalse(rootChildDisplayNames(authority).contains("contract-renamed.txt"))
    }

    private fun assertRootAdvertisesWritableIrisDrive(authority: String) {
        val rootsUri = DocumentsContract.buildRootsUri(authority)
        context.contentResolver.query(
            rootsUri,
            arrayOf(Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE, Root.COLUMN_FLAGS),
            null,
            null,
            null,
        ).use { cursor ->
            assertNotNull("Root query returned null", cursor)
            assertTrue("Expected at least one DocumentsProvider root", cursor!!.moveToFirst())
            assertEquals(
                IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
                cursor.getString(cursor.getColumnIndexOrThrow(Root.COLUMN_DOCUMENT_ID)),
            )
            assertEquals(
                "Iris Drive",
                cursor.getString(cursor.getColumnIndexOrThrow(Root.COLUMN_TITLE)),
            )
            val flags = cursor.getInt(cursor.getColumnIndexOrThrow(Root.COLUMN_FLAGS))
            assertTrue(flags and Root.FLAG_SUPPORTS_CREATE != 0)
            assertTrue(flags and Root.FLAG_SUPPORTS_IS_CHILD != 0)
        }
    }

    private fun assertDocument(uri: Uri, displayName: String, size: Long) {
        context.contentResolver.query(
            uri,
            arrayOf(Document.COLUMN_DISPLAY_NAME, Document.COLUMN_SIZE),
            null,
            null,
            null,
        ).use { cursor ->
            assertNotNull("Document query returned null for $uri", cursor)
            assertTrue("Document row missing for $uri", cursor!!.moveToFirst())
            assertEquals(displayName, cursor.getString(cursor.getColumnIndexOrThrow(Document.COLUMN_DISPLAY_NAME)))
            assertEquals(size, cursor.getLong(cursor.getColumnIndexOrThrow(Document.COLUMN_SIZE)))
        }
    }

    private fun waitForDocumentText(uri: Uri): String {
        val deadline = System.currentTimeMillis() + 5_000L
        var last = ""
        while (System.currentTimeMillis() < deadline) {
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            last = context.contentResolver.openInputStream(uri).use { stream ->
                assertNotNull("openInputStream returned null for $uri", stream)
                stream!!.readBytes().toString(Charsets.UTF_8)
            }
            if (last.isNotEmpty()) {
                return last
            }
            Thread.sleep(100)
        }
        return last
    }

    private fun rootChildDisplayNames(authority: String): List<String> {
        val childrenUri = DocumentsContract.buildChildDocumentsUri(
            authority,
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
        )
        return context.contentResolver.query(
            childrenUri,
            arrayOf(Document.COLUMN_DISPLAY_NAME),
            null,
            null,
            null,
        ).use { cursor ->
            assertNotNull("Child document query returned null", cursor)
            buildList {
                while (cursor!!.moveToNext()) {
                    add(cursor.getString(cursor.getColumnIndexOrThrow(Document.COLUMN_DISPLAY_NAME)))
                }
            }
        }
    }

    private fun createProfile() {
        val handle = NativeCore.appNew(context.filesDir.absolutePath, "documents-contract-test")
        try {
            NativeCore.dispatchJson(
                handle,
                JSONObject()
                    .put("type", "create_profile")
                    .put("app_key_label", "Android DocumentsContract test")
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
