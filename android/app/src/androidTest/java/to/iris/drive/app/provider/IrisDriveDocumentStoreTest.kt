package to.iris.drive.app.provider

import android.content.Context
import android.provider.DocumentsContract.Document
import androidx.test.core.app.ApplicationProvider
import java.io.ByteArrayInputStream
import java.io.File
import java.io.FileNotFoundException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.json.JSONObject
import to.iris.drive.app.core.NativeCore

class IrisDriveDocumentStoreTest {
    @get:Rule
    val temporaryFolder = TemporaryFolder()

    @Before
    fun setUp() {
        NativeCore.initializeAndroidContext(ApplicationProvider.getApplicationContext<Context>())
    }

    @Test
    fun createRenameAndDeleteFileUnderRoot() {
        val store = newStore()

        val created = store.createDocument(
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
            "text/plain",
            "note.txt",
        )
        store.writeDocumentFromTemp(created.documentId, textSource("hello"))

        val queried = store.queryDocument(created.documentId)
        assertEquals("note.txt", queried.displayName)
        assertEquals(5, queried.size)
        assertFalse(queried.isDirectory)

        val renamedId = store.renameDocument(created.documentId, "renamed.txt")
        assertEquals(listOf("renamed.txt"), store.childDocuments(IrisDriveDocumentStore.ROOT_DOCUMENT_ID).map { it.displayName })

        store.deleteDocument(renamedId)

        assertTrue(store.childDocuments(IrisDriveDocumentStore.ROOT_DOCUMENT_ID).isEmpty())
    }

    @Test
    fun createsDirectoriesAndRejectsTraversal() {
        val store = newStore()

        val directory = store.createDocument(
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
            Document.MIME_TYPE_DIR,
            "Projects",
        )
        val file = store.createDocument(directory.documentId, "text/markdown", "plan.md")

        assertEquals(listOf("plan.md"), store.childDocuments(directory.documentId).map { it.displayName })
        assertTrue(store.isChildDocument(IrisDriveDocumentStore.ROOT_DOCUMENT_ID, file.documentId))
        assertTrue(store.isChildDocument(directory.documentId, file.documentId))
        assertThrows(FileNotFoundException::class.java) {
            store.queryDocument("root/../outside.txt")
        }
    }

    @Test
    fun deDuplicatesNamesInOneDirectory() {
        val store = newStore()

        val first = store.createDocument(IrisDriveDocumentStore.ROOT_DOCUMENT_ID, "text/plain", "note.txt")
        val second = store.createDocument(IrisDriveDocumentStore.ROOT_DOCUMENT_ID, "text/plain", "note.txt")

        assertEquals("note.txt", first.displayName)
        assertEquals("note (2).txt", second.displayName)
        assertEquals(
            listOf("note (2).txt", "note.txt"),
            store.childDocuments(IrisDriveDocumentStore.ROOT_DOCUMENT_ID).map { it.displayName },
        )
    }

    @Test
    fun importFileWritesBytesAndSanitizesName() {
        val store = newStore()

        val imported = store.importFile(
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
            "text/plain",
            "../Shared/note.txt",
            ByteArrayInputStream("shared bytes".toByteArray()),
        )

        assertEquals("Shared_note.txt", imported.displayName)
        assertEquals("shared bytes", store.readDocumentText(imported.documentId))
    }

    @Test
    fun importFileDeDuplicatesNames() {
        val store = newStore()

        store.importFile(
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
            "text/plain",
            "note.txt",
            ByteArrayInputStream("first".toByteArray()),
        )
        val second = store.importFile(
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
            "text/plain",
            "note.txt",
            ByteArrayInputStream("second".toByteArray()),
        )

        assertEquals("note (2).txt", second.displayName)
        assertEquals("second", store.readDocumentText(second.documentId))
    }

    private fun newStore(): IrisDriveDocumentStore {
        val dataDir = temporaryFolder.newFolder("drive-provider")
        val handle = NativeCore.appNew(dataDir.absolutePath, "provider-test")
        try {
            NativeCore.dispatchJson(
                handle,
                JSONObject()
                    .put("type", "create_profile")
                    .put("app_key_label", "Android provider test")
                    .toString(),
            )
        } finally {
            NativeCore.appFree(handle)
        }
        return IrisDriveDocumentStore(dataDir)
    }

    private fun textSource(text: String): File =
        temporaryFolder.newFile().also { it.writeText(text) }

    private fun IrisDriveDocumentStore.readDocumentText(documentId: String): String {
        val file = readDocumentToTemp(documentId)
        return try {
            file.readText()
        } finally {
            file.delete()
        }
    }
}
