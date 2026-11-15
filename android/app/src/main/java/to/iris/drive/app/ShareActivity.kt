package to.iris.drive.app

import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import java.io.ByteArrayInputStream
import java.io.FileNotFoundException
import java.net.URLConnection
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import to.iris.drive.app.provider.IrisDriveDocumentStore

/** The system share sheet's "Save to Iris Drive": confirm the name and folder, then save. */
class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val store = IrisDriveDocumentStore(filesDir, getString(R.string.app_name))
        val items = sharedItems(intent)
        setContent {
            IrisDriveTheme {
                val folders by produceState<Result<List<String>?>?>(initialValue = null) {
                    value = withContext(Dispatchers.IO) { runCatching { store.shareDestinations() } }
                }
                folders?.let { loaded ->
                    ShareSaveScreen(
                        items = items,
                        folders = loaded.getOrNull(),
                        onSave = { toSave, folder -> save(store, toSave, folder) },
                        onOpenApp = {
                            startActivity(Intent(this@ShareActivity, MainActivity::class.java))
                            finish()
                        },
                        onClose = { finish() },
                    )
                }
            }
        }
    }

    private fun save(store: IrisDriveDocumentStore, items: List<SharedItem>, folder: String) {
        val parent = store.documentIdForFolder(folder)
        for (item in items) {
            val input = item.uri?.let { contentResolver.openInputStream(it) }
                ?: item.text?.let { ByteArrayInputStream(it.toByteArray()) }
                ?: throw FileNotFoundException("${item.name} can no longer be read")
            store.importFile(parent, item.mimeType, item.name, input)
        }
    }

    /** What was shared: every file, or the text as a note when there is no file. */
    private fun sharedItems(intent: Intent?): List<SharedItem> {
        if (intent == null) return emptyList()
        val files = streamUris(intent).map { uri ->
            val name = displayName(uri) ?: uri.lastPathSegment ?: "Shared file"
            SharedItem(name = name, mimeType = contentResolver.getType(uri) ?: inferMimeType(name), uri = uri)
        }
        if (files.isNotEmpty()) return files
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.takeIf { it.isNotBlank() }
            ?: return emptyList()
        val subject = intent.getStringExtra(Intent.EXTRA_SUBJECT)?.takeIf { it.isNotBlank() } ?: "Shared text"
        return listOf(SharedItem(name = "$subject.txt", mimeType = "text/plain", text = text))
    }

    private fun streamUris(intent: Intent): List<Uri> {
        val uris = linkedSetOf<Uri>()
        streamUriExtra(intent)?.let(uris::add)
        streamUriListExtra(intent)?.let(uris::addAll)
        val clipData = intent.clipData
        if (clipData != null) {
            for (index in 0 until clipData.itemCount) {
                clipData.getItemAt(index).uri?.let(uris::add)
            }
        }
        return uris.toList()
    }

    private fun streamUriExtra(intent: Intent): Uri? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableExtra(Intent.EXTRA_STREAM)
        }

    private fun streamUriListExtra(intent: Intent): ArrayList<Uri>? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
        }

    private fun displayName(uri: Uri): String? {
        var cursor: Cursor? = null
        return try {
            cursor = contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            if (cursor != null && cursor.moveToFirst()) {
                cursor.getString(0)
            } else {
                null
            }
        } finally {
            cursor?.close()
        }
    }

    private fun inferMimeType(displayName: String): String =
        URLConnection.guessContentTypeFromName(displayName) ?: "application/octet-stream"
}
