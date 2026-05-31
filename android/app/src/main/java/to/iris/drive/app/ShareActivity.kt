package to.iris.drive.app

import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.lifecycle.lifecycleScope
import java.io.ByteArrayInputStream
import java.io.File
import java.net.URLConnection
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.drive.app.provider.IrisDriveDocumentStore

class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        importSharedContent(intent)
    }

    private fun importSharedContent(intent: Intent?) {
        lifecycleScope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching { importIntent(intent) }
            }
            val count = result.getOrDefault(0)
            val message = when {
                result.isFailure -> "Could not save to Iris Drive"
                count == 1 -> "Saved to Iris Drive"
                count > 1 -> "Saved $count items to Iris Drive"
                else -> "Nothing to save"
            }
            Toast.makeText(this@ShareActivity, message, Toast.LENGTH_SHORT).show()
            finish()
        }
    }

    private fun importIntent(intent: Intent?): Int {
        if (intent == null) return 0
        val store = IrisDriveDocumentStore(
            filesDir,
            getString(R.string.app_name),
        )
        var imported = 0
        for (uri in streamUris(intent)) {
            val name = displayName(uri) ?: uri.lastPathSegment ?: "Shared file"
            val mimeType = contentResolver.getType(uri) ?: inferMimeType(name)
            val input = contentResolver.openInputStream(uri) ?: continue
            store.importFile(IrisDriveDocumentStore.ROOT_DOCUMENT_ID, mimeType, name, input)
            imported += 1
        }
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
        if (!text.isNullOrBlank() && imported == 0) {
            val subject = intent.getStringExtra(Intent.EXTRA_SUBJECT)
                ?.takeIf { it.isNotBlank() }
                ?: "Shared text"
            store.importFile(
                IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
                "text/plain",
                "$subject.txt",
                ByteArrayInputStream(text.toByteArray()),
            )
            imported += 1
        }
        return imported
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
