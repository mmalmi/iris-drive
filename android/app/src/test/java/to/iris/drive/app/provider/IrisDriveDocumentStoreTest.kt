package to.iris.drive.app.provider

import java.io.FileNotFoundException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class IrisDriveDocumentStoreTest {
    @get:Rule
    val temporaryFolder = TemporaryFolder()

    @Test
    fun createRenameAndDeleteFileUnderRoot() {
        val store = newStore()

        val created = store.createDocument(
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
            "text/plain",
            "note.txt",
        )
        store.fileForDocument(created.documentId).writeText("hello")

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
            IrisDriveDocumentStore.DIRECTORY_MIME_TYPE,
            "Projects",
        )
        val file = store.createDocument(directory.documentId, "text/markdown", "plan.md")

        assertEquals(listOf("plan.md"), store.childDocuments(directory.documentId).map { it.displayName })
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

    private fun newStore(): IrisDriveDocumentStore =
        IrisDriveDocumentStore(temporaryFolder.newFolder("drive-provider"))
}
