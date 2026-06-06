package to.iris.drive.app

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.DocumentsContract
import android.widget.Toast
import androidx.activity.ComponentActivity
import to.iris.drive.app.provider.IrisDriveDocumentStore

class ShareDocumentSettingsActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        openShareDialogForDocument(intent)
        finish()
    }

    private fun openShareDialogForDocument(intent: Intent?) {
        val documentUri = intent?.data
        val documentId = documentUri?.let { uri ->
            runCatching { DocumentsContract.getDocumentId(uri) }.getOrNull()
        }
        if (documentId.isNullOrBlank()) {
            showUnavailable()
            return
        }

        val store = IrisDriveDocumentStore(filesDir, getString(R.string.app_name))
        val entry = runCatching { store.queryDocument(documentId) }.getOrNull()
        if (entry == null || !entry.isDirectory || entry.isRoot) {
            showUnavailable()
            return
        }

        val sourcePath = runCatching { store.providerPathForDocumentId(documentId) }.getOrNull()
        val shareLink = sourcePath?.let { irisDriveShareDialogLink(it, entry.displayName) }
        if (shareLink.isNullOrBlank()) {
            showUnavailable()
            return
        }

        startActivity(
            Intent(this, MainActivity::class.java)
                .setAction(Intent.ACTION_VIEW)
                .setData(Uri.parse(shareLink))
                .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
        )
    }

    private fun showUnavailable() {
        Toast.makeText(this, "Choose an Iris Drive folder to share", Toast.LENGTH_SHORT).show()
    }
}
