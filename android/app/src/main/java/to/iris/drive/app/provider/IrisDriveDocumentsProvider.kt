package to.iris.drive.app.provider

import android.content.res.AssetFileDescriptor
import android.database.Cursor
import android.database.MatrixCursor
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import java.io.File
import java.io.FileNotFoundException
import to.iris.drive.app.R

class IrisDriveDocumentsProvider : DocumentsProvider() {
    override fun onCreate(): Boolean = true

    override fun queryRoots(projection: Array<out String>?): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_ROOT_PROJECTION)
        cursor.newRow()
            .add(Root.COLUMN_ROOT_ID, IrisDriveDocumentStore.ROOT_ID)
            .add(Root.COLUMN_DOCUMENT_ID, IrisDriveDocumentStore.ROOT_DOCUMENT_ID)
            .add(Root.COLUMN_TITLE, context?.getString(R.string.app_name) ?: "Iris Drive")
            .add(Root.COLUMN_ICON, R.drawable.ic_drive)
            .add(
                Root.COLUMN_FLAGS,
                Root.FLAG_LOCAL_ONLY or Root.FLAG_SUPPORTS_CREATE or Root.FLAG_SUPPORTS_IS_CHILD,
            )
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
        val cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
        store().childDocuments(parentDocumentId).forEach { entry ->
            includeDocument(cursor, entry)
        }
        return cursor
    }

    override fun isChildDocument(parentDocumentId: String, documentId: String): Boolean =
        store().isChildDocument(parentDocumentId, documentId)

    override fun createDocument(
        parentDocumentId: String,
        mimeType: String,
        displayName: String,
    ): String = store().createDocument(parentDocumentId, mimeType, displayName).documentId

    override fun deleteDocument(documentId: String) {
        store().deleteDocument(documentId)
    }

    override fun renameDocument(documentId: String, displayName: String): String =
        store().renameDocument(documentId, displayName)

    override fun openDocument(
        documentId: String,
        mode: String,
        signal: CancellationSignal?,
    ): ParcelFileDescriptor {
        val file = store().fileForDocument(documentId)
        if (!file.isFile) {
            throw FileNotFoundException(documentId)
        }
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.parseMode(mode))
    }

    override fun openDocumentThumbnail(
        documentId: String,
        sizeHint: android.graphics.Point,
        signal: CancellationSignal?,
    ): AssetFileDescriptor {
        throw FileNotFoundException(documentId)
    }

    private fun store(): IrisDriveDocumentStore {
        val appContext = context ?: throw FileNotFoundException("context unavailable")
        return IrisDriveDocumentStore(
            File(appContext.filesDir, "drive-provider"),
            appContext.getString(R.string.app_name),
        )
    }

    private fun includeDocument(cursor: MatrixCursor, documentId: String) {
        includeDocument(cursor, store().queryDocument(documentId))
    }

    private fun includeDocument(cursor: MatrixCursor, entry: IrisDriveDocumentEntry) {
        cursor.newRow()
            .add(Document.COLUMN_DOCUMENT_ID, entry.documentId)
            .add(Document.COLUMN_DISPLAY_NAME, entry.displayName)
            .add(Document.COLUMN_MIME_TYPE, entry.mimeType)
            .add(Document.COLUMN_FLAGS, documentFlags(entry))
            .add(Document.COLUMN_ICON, R.drawable.ic_drive)
            .add(Document.COLUMN_SIZE, entry.size)
            .add(Document.COLUMN_LAST_MODIFIED, entry.lastModified)
    }

    private fun documentFlags(entry: IrisDriveDocumentEntry): Int {
        if (entry.isDirectory) {
            var flags = Document.FLAG_DIR_SUPPORTS_CREATE
            if (!entry.isRoot) {
                flags = flags or Document.FLAG_SUPPORTS_DELETE or Document.FLAG_SUPPORTS_RENAME
            }
            return flags
        }
        return Document.FLAG_SUPPORTS_WRITE or
            Document.FLAG_SUPPORTS_DELETE or
            Document.FLAG_SUPPORTS_RENAME
    }

    companion object {
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
