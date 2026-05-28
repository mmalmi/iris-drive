package to.iris.drive.app.provider

import java.io.File
import java.io.FileNotFoundException
import java.net.URLConnection

internal data class IrisDriveDocumentEntry(
    val documentId: String,
    val displayName: String,
    val mimeType: String,
    val size: Long,
    val lastModified: Long,
    val isDirectory: Boolean,
    val isRoot: Boolean,
)

internal class IrisDriveDocumentStore(
    private val rootDir: File,
    private val rootTitle: String = "Iris Drive",
) {
    fun queryDocument(documentId: String): IrisDriveDocumentEntry {
        val file = fileForDocument(documentId)
        if (!file.exists()) {
            throw FileNotFoundException(documentId)
        }
        return entryFor(file)
    }

    fun childDocuments(parentDocumentId: String): List<IrisDriveDocumentEntry> {
        val parent = fileForDocument(parentDocumentId)
        if (!parent.isDirectory) {
            throw FileNotFoundException(parentDocumentId)
        }
        return parent.listFiles()
            .orEmpty()
            .map(::entryFor)
            .sortedWith(
                compareBy<IrisDriveDocumentEntry> { !it.isDirectory }
                    .thenBy { it.displayName.lowercase() },
            )
    }

    fun createDocument(
        parentDocumentId: String,
        mimeType: String,
        displayName: String,
    ): IrisDriveDocumentEntry {
        val parent = fileForDocument(parentDocumentId)
        if (!parent.isDirectory) {
            throw FileNotFoundException(parentDocumentId)
        }

        val target = uniqueTarget(parent, displayName)
        if (mimeType == DIRECTORY_MIME_TYPE) {
            if (!target.mkdirs()) {
                throw FileNotFoundException(target.name)
            }
        } else {
            target.parentFile?.mkdirs()
            if (!target.createNewFile()) {
                throw FileNotFoundException(target.name)
            }
        }
        return entryFor(target, mimeType.takeIf { it.isNotBlank() } ?: DEFAULT_FILE_MIME_TYPE)
    }

    fun deleteDocument(documentId: String) {
        if (documentId == ROOT_DOCUMENT_ID) {
            throw FileNotFoundException(documentId)
        }
        val file = fileForDocument(documentId)
        if (!file.exists() || !file.deleteRecursively()) {
            throw FileNotFoundException(documentId)
        }
    }

    fun renameDocument(documentId: String, displayName: String): String {
        if (documentId == ROOT_DOCUMENT_ID) {
            throw FileNotFoundException(documentId)
        }
        val file = fileForDocument(documentId)
        val parent = file.parentFile ?: throw FileNotFoundException(documentId)
        val sanitized = sanitizeDisplayName(displayName)
        if (file.name == sanitized) {
            return documentId
        }

        val target = uniqueTarget(parent, sanitized)
        if (!file.renameTo(target)) {
            throw FileNotFoundException(documentId)
        }
        return documentIdFor(target)
    }

    fun isChildDocument(parentDocumentId: String, documentId: String): Boolean =
        runCatching {
            val parent = fileForDocument(parentDocumentId)
            val child = fileForDocument(documentId)
            child == parent || child.path.startsWith(parent.path + File.separator)
        }.getOrDefault(false)

    fun fileForDocument(documentId: String): File {
        val root = canonicalRoot()
        if (documentId == ROOT_DOCUMENT_ID) {
            return root
        }
        if (!documentId.startsWith(ROOT_CHILD_PREFIX)) {
            throw FileNotFoundException(documentId)
        }

        val relativePath = documentId.removePrefix(ROOT_CHILD_PREFIX)
        if (relativePath.isBlank()) {
            throw FileNotFoundException(documentId)
        }
        val file = File(root, relativePath).canonicalFile
        if (file != root && !file.path.startsWith(root.path + File.separator)) {
            throw FileNotFoundException(documentId)
        }
        return file
    }

    private fun entryFor(
        file: File,
        mimeType: String = if (file.isDirectory) DIRECTORY_MIME_TYPE else inferMimeType(file.name),
    ): IrisDriveDocumentEntry {
        val isRoot = file == canonicalRoot()
        return IrisDriveDocumentEntry(
            documentId = documentIdFor(file),
            displayName = if (isRoot) rootTitle else file.name,
            mimeType = mimeType,
            size = if (file.isFile) file.length() else 0L,
            lastModified = file.lastModified(),
            isDirectory = file.isDirectory,
            isRoot = isRoot,
        )
    }

    private fun documentIdFor(file: File): String {
        val root = canonicalRoot()
        if (file == root) {
            return ROOT_DOCUMENT_ID
        }
        val relativePath = file.relativeTo(root).invariantSeparatorsPath
        return "$ROOT_DOCUMENT_ID/$relativePath"
    }

    private fun uniqueTarget(parent: File, requestedName: String): File {
        val sanitized = sanitizeDisplayName(requestedName)
        var target = File(parent, sanitized)
        if (!target.exists()) {
            return target
        }

        val dotIndex = sanitized.lastIndexOf('.').takeIf { it > 0 }
        val basename = dotIndex?.let { sanitized.substring(0, it) } ?: sanitized
        val extension = dotIndex?.let { sanitized.substring(it) }.orEmpty()
        var index = 2
        while (target.exists()) {
            target = File(parent, "$basename ($index)$extension")
            index += 1
        }
        return target
    }

    private fun canonicalRoot(): File {
        rootDir.mkdirs()
        return rootDir.canonicalFile
    }

    private fun sanitizeDisplayName(displayName: String): String {
        val cleaned = displayName
            .trim()
            .replace('\\', '/')
            .split('/')
            .filter { it.isNotBlank() && it != "." && it != ".." }
            .joinToString("_")
        return cleaned.ifBlank { "Untitled" }
    }

    companion object {
        const val ROOT_ID = "iris-drive"
        const val ROOT_DOCUMENT_ID = "root"
        const val DIRECTORY_MIME_TYPE = "vnd.android.document/directory"
        private const val DEFAULT_FILE_MIME_TYPE = "application/octet-stream"
        private const val ROOT_CHILD_PREFIX = "$ROOT_DOCUMENT_ID/"

        private fun inferMimeType(displayName: String): String =
            URLConnection.guessContentTypeFromName(displayName) ?: DEFAULT_FILE_MIME_TYPE
    }
}
