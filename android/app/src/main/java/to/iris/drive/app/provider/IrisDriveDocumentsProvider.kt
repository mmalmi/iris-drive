package to.iris.drive.app.provider

import android.content.res.AssetFileDescriptor
import android.database.Cursor
import android.database.MatrixCursor
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import java.io.FileNotFoundException
import to.iris.drive.app.R

class IrisDriveDocumentsProvider : DocumentsProvider() {
    override fun onCreate(): Boolean = true

    override fun queryRoots(projection: Array<out String>?): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_ROOT_PROJECTION)
        cursor.newRow()
            .add(Root.COLUMN_ROOT_ID, ROOT_ID)
            .add(Root.COLUMN_DOCUMENT_ID, ROOT_DOCUMENT_ID)
            .add(Root.COLUMN_TITLE, context?.getString(R.string.app_name) ?: "Iris Drive")
            .add(Root.COLUMN_ICON, R.drawable.ic_drive)
            .add(Root.COLUMN_FLAGS, Root.FLAG_LOCAL_ONLY or Root.FLAG_SUPPORTS_IS_CHILD)
        return cursor
    }

    override fun queryDocument(documentId: String, projection: Array<out String>?): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
        includeDocument(cursor, documentId)
        return cursor
    }

    override fun queryChildDocuments(
        parentDocumentId: String,
        projection: Array<out String>?,
        sortOrder: String?,
    ): Cursor {
        if (parentDocumentId != ROOT_DOCUMENT_ID) {
            throw FileNotFoundException(parentDocumentId)
        }
        return MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
    }

    override fun isChildDocument(parentDocumentId: String, documentId: String): Boolean =
        parentDocumentId == ROOT_DOCUMENT_ID && documentId == ROOT_DOCUMENT_ID

    override fun openDocument(
        documentId: String,
        mode: String,
        signal: CancellationSignal?,
    ): ParcelFileDescriptor {
        throw FileNotFoundException(documentId)
    }

    override fun openDocumentThumbnail(
        documentId: String,
        sizeHint: android.graphics.Point,
        signal: CancellationSignal?,
    ): AssetFileDescriptor {
        throw FileNotFoundException(documentId)
    }

    private fun includeDocument(cursor: MatrixCursor, documentId: String) {
        if (documentId != ROOT_DOCUMENT_ID) {
            throw FileNotFoundException(documentId)
        }
        cursor.newRow()
            .add(Document.COLUMN_DOCUMENT_ID, ROOT_DOCUMENT_ID)
            .add(Document.COLUMN_DISPLAY_NAME, "Iris Drive")
            .add(Document.COLUMN_MIME_TYPE, Document.MIME_TYPE_DIR)
            .add(Document.COLUMN_FLAGS, 0)
            .add(Document.COLUMN_ICON, R.drawable.ic_drive)
            .add(Document.COLUMN_SIZE, null)
            .add(Document.COLUMN_LAST_MODIFIED, 0)
    }

    companion object {
        private const val ROOT_ID = "iris-drive"
        private const val ROOT_DOCUMENT_ID = "root"

        private val DEFAULT_ROOT_PROJECTION =
            arrayOf(
                Root.COLUMN_ROOT_ID,
                Root.COLUMN_DOCUMENT_ID,
                Root.COLUMN_TITLE,
                Root.COLUMN_ICON,
                Root.COLUMN_FLAGS,
            )

        private val DEFAULT_DOCUMENT_PROJECTION =
            arrayOf(
                Document.COLUMN_DOCUMENT_ID,
                Document.COLUMN_DISPLAY_NAME,
                Document.COLUMN_MIME_TYPE,
                Document.COLUMN_FLAGS,
                Document.COLUMN_ICON,
                Document.COLUMN_SIZE,
                Document.COLUMN_LAST_MODIFIED,
            )
    }
}
