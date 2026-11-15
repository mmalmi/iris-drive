package to.iris.drive.app

import android.content.Context
import android.content.Intent
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.ByteArrayInputStream
import java.io.File
import java.util.UUID
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.provider.IrisDriveDocumentStore

/**
 * "Save to Iris Drive" from another app's share sheet: the person sees what
 * will be saved and where, and nothing is saved until they say so.
 */
@RunWith(AndroidJUnit4::class)
class IrisDriveAndroidShareSheetTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var context: Context
    private val nativeHandles = mutableListOf<Long>()
    private val dataDirs = mutableListOf<File>()

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
        NativeCore.initializeAndroidContext(context)
    }

    @After
    fun tearDown() {
        nativeHandles.forEach(NativeCore::appFree)
        dataDirs.forEach { it.deleteRecursively() }
    }

    private fun show(
        items: List<SharedItem>,
        folders: List<String>?,
        onSave: suspend (List<SharedItem>, String) -> Unit = { _, _ -> },
        onClose: () -> Unit = {},
    ) {
        compose.setContent {
            IrisDriveTheme {
                ShareSaveScreen(items, folders, onSave, onOpenApp = {}, onClose = onClose)
            }
        }
    }

    private fun note(name: String) = SharedItem(name = name, mimeType = "text/plain", text = "hello")

    @Test
    fun aSharedFileIsRenamedAndSavedIntoTheChosenFolder() {
        val saved = mutableListOf<Pair<String, String>>()
        var closed = false
        show(
            items = listOf(note("Notes.txt")),
            folders = listOf("Photos", "Photos/Trip"),
            onSave = { items, folder -> items.forEach { saved += folder to it.name } },
            onClose = { closed = true },
        )
        compose.onNodeWithTag("shareSaveName").assertTextContains("Notes.txt")
        compose.onNodeWithTag("shareSaveFolderName").assertTextContains("My Drive")
        compose.onNodeWithTag("shareSaveName").performTextReplacement("Packing.txt")
        compose.onNodeWithTag("shareSaveFolder").performClick()
        compose.onNodeWithText("Photos/Trip").performClick()
        compose.onNodeWithTag("shareSaveFolderName").assertTextContains("My Drive/Photos/Trip")
        assertEquals(emptyList<Pair<String, String>>(), saved)

        compose.onNodeWithTag("shareSaveButton").assertIsEnabled().performClick()
        compose.waitUntil(5_000) { closed }

        assertEquals(listOf("Photos/Trip" to "Packing.txt"), saved)
    }

    @Test
    fun severalItemsGoToMyDriveUnlessAFolderIsChosen() {
        val saved = mutableListOf<Pair<String, String>>()
        var closed = false
        show(
            items = listOf(note("a.jpg"), note("b.jpg"), note("c.jpg")),
            folders = emptyList(),
            onSave = { items, folder -> items.forEach { saved += folder to it.name } },
            onClose = { closed = true },
        )
        compose.onNodeWithText("3 items").assertIsDisplayed()
        compose.onNodeWithTag("shareSaveButton").performClick()
        compose.waitUntil(5_000) { closed }
        assertEquals(listOf("" to "a.jpg", "" to "b.jpg", "" to "c.jpg"), saved)
    }

    @Test
    fun withoutAProfileItSaysToSetUpFirstAndOffersNoSave() {
        show(items = listOf(note("Notes.txt")), folders = null)
        compose.onNodeWithTag("shareSaveNotSetUp").assertIsDisplayed()
        compose.onNodeWithText("Open Iris Drive").assertIsDisplayed()
        assertEquals(0, compose.onAllNodesWithTag("shareSaveButton").fetchSemanticsNodes().size)
    }

    @Test
    fun aFailureStaysOnScreenAndSavingCanBeTriedAgain() {
        var attempts = 0
        var closed = false
        show(
            items = listOf(note("Notes.txt")),
            folders = emptyList(),
            onSave = { _, _ ->
                attempts += 1
                if (attempts == 1) error("This shared folder is read-only")
            },
            onClose = { closed = true },
        )
        compose.onNodeWithTag("shareSaveButton").performClick()
        compose.waitUntil(5_000) { compose.onAllNodesWithTag("shareSaveError").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("This shared folder is read-only").assertIsDisplayed()
        assertTrue(!closed)

        compose.onNodeWithTag("shareSaveButton").assertIsEnabled().performClick()
        compose.waitUntil(5_000) { closed }
        assertEquals(2, attempts)
    }

    @Test
    fun theDriveOffersItsFoldersAndKeepsBothFilesWhenANameIsTaken() {
        val dir = File(context.cacheDir, "iris-drive-share-${UUID.randomUUID()}").also { it.mkdirs() }
        dataDirs += dir
        val store = IrisDriveDocumentStore(dir)
        assertNull("no profile yet, so nothing to save into", store.shareDestinations())

        val handle = NativeCore.appNew(dir.absolutePath, "ui-test").also(nativeHandles::add)
        NativeCore.dispatchJson(handle, NativeActions.createProfile("Share sheet"))
        assertEquals(emptyList<String>(), store.shareDestinations())
        val photos = store.createDocument(IrisDriveDocumentStore.ROOT_DOCUMENT_ID, DIRECTORY, "Photos")
        store.createDocument(photos.documentId, DIRECTORY, "Trip")
        assertEquals(listOf("Photos", "Photos/Trip"), store.shareDestinations())

        val trip = store.documentIdForFolder("Photos/Trip")
        fun save(parent: String) =
            store.importFile(parent, "text/plain", "Note.txt", ByteArrayInputStream("hello".toByteArray()))
        save(trip)
        save(trip)
        save(store.documentIdForFolder(""))

        val inTrip = store.childDocuments(trip).map { it.displayName }
        assertEquals(2, inTrip.size)
        assertTrue(inTrip.toString(), "Note.txt" in inTrip)
        assertEquals(2, inTrip.toSet().size)
        assertTrue(store.childDocuments(IrisDriveDocumentStore.ROOT_DOCUMENT_ID).any { it.displayName == "Note.txt" })
    }

    @Test
    fun sharingTextBeforeSetUpOpensTheSheetWithoutSavingAnything() {
        assumeFalse("needs an install with no profile", File(context.filesDir, "key").exists())
        val intent = Intent(context, ShareActivity::class.java)
            .setAction(Intent.ACTION_SEND)
            .setType("text/plain")
            .putExtra(Intent.EXTRA_SUBJECT, "Trip note")
            .putExtra(Intent.EXTRA_TEXT, "Remember the tent")
        ActivityScenario.launch<ShareActivity>(intent).use {
            compose.waitUntil(10_000) {
                compose.onAllNodesWithTag("shareSaveNotSetUp").fetchSemanticsNodes().isNotEmpty()
            }
            compose.onNodeWithText("Save to Iris Drive").assertIsDisplayed()
        }
        assertTrue(!File(context.filesDir, "key").exists())
    }

    private companion object {
        const val DIRECTORY = "vnd.android.document/directory"
    }
}
