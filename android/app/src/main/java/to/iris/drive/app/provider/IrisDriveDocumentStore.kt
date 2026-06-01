package to.iris.drive.app.provider

import java.io.File
import java.io.FileNotFoundException
import java.io.InputStream
import java.net.URLConnection
import org.json.JSONArray
import org.json.JSONObject
import to.iris.drive.app.core.NativeCore

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
    private val dataDir: File,
    private val rootTitle: String = "Iris Drive",
) {
    fun queryDocument(documentId: String): IrisDriveDocumentEntry {
        if (documentId == ROOT_DOCUMENT_ID) {
            return rootEntry()
        }
        val path = pathForDocumentId(documentId)
        return providerEntries()
            .firstOrNull { it.path == path }
            ?.toDocumentEntry()
            ?: throw FileNotFoundException(documentId)
    }

    fun childDocuments(parentDocumentId: String): List<IrisDriveDocumentEntry> {
        val parentPath = if (parentDocumentId == ROOT_DOCUMENT_ID) {
            ""
        } else {
            val path = pathForDocumentId(parentDocumentId)
            val parent = providerEntries().firstOrNull { it.path == path }
                ?: throw FileNotFoundException(parentDocumentId)
            if (!parent.isDirectory) throw FileNotFoundException(parentDocumentId)
            path
        }
        return providerEntries()
            .filter { it.parentPath == parentPath }
            .map { it.toDocumentEntry() }
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
        val parentPath = directoryPathForDocument(parentDocumentId)
        val targetPath = resolvedProviderPath(parentPath, displayName)
        if (mimeType == DIRECTORY_MIME_TYPE) {
            requireNativeOk(NativeCore.providerMkdirJson(dataDir.absolutePath, targetPath))
        } else {
            val source = tempFile("create")
            source.writeBytes(ByteArray(0))
            try {
                requireNativeOk(
                    NativeCore.providerWriteJson(dataDir.absolutePath, targetPath, source.absolutePath),
                )
            } finally {
                source.delete()
            }
        }
        return queryDocument(documentIdForPath(targetPath))
    }

    fun importFile(
        parentDocumentId: String,
        mimeType: String,
        displayName: String,
        input: InputStream,
    ): IrisDriveDocumentEntry {
        val parentPath = directoryPathForDocument(parentDocumentId)
        val targetPath = resolvedProviderPath(parentPath, displayName)
        val source = tempFile("import")
        try {
            input.use { stream ->
                source.outputStream().use { target -> stream.copyTo(target) }
            }
            requireNativeOk(
                NativeCore.providerWriteJson(dataDir.absolutePath, targetPath, source.absolutePath),
            )
        } finally {
            source.delete()
        }
        val entry = queryDocument(documentIdForPath(targetPath))
        return if (mimeType.isBlank()) entry else entry.copy(mimeType = mimeType)
    }

    fun deleteDocument(documentId: String) {
        if (documentId == ROOT_DOCUMENT_ID) {
            throw FileNotFoundException(documentId)
        }
        requireNativeOk(NativeCore.providerDeleteJson(dataDir.absolutePath, pathForDocumentId(documentId)))
    }

    fun renameDocument(documentId: String, displayName: String): String {
        if (documentId == ROOT_DOCUMENT_ID) {
            throw FileNotFoundException(documentId)
        }
        val oldPath = pathForDocumentId(documentId)
        val parentPath = providerEntries()
            .firstOrNull { it.path == oldPath }
            ?.parentPath
            ?: throw FileNotFoundException(documentId)
        val targetPath = resolvedProviderPath(parentPath, displayName, excluding = oldPath)
        if (targetPath == oldPath) return documentId
        requireNativeOk(NativeCore.providerRenameJson(dataDir.absolutePath, oldPath, targetPath))
        return documentIdForPath(targetPath)
    }

    fun isChildDocument(parentDocumentId: String, documentId: String): Boolean =
        runCatching {
            val parentPath = if (parentDocumentId == ROOT_DOCUMENT_ID) {
                ""
            } else {
                pathForDocumentId(parentDocumentId)
            }
            val childPath = if (documentId == ROOT_DOCUMENT_ID) {
                ""
            } else {
                pathForDocumentId(documentId)
            }
            NativeCore.providerPathIsChildDocument(parentPath, childPath)
        }.getOrDefault(false)

    fun readDocumentToTemp(documentId: String): File {
        val path = pathForDocumentId(documentId)
        val target = tempFile("read")
        val result = NativeCore.providerReadJson(dataDir.absolutePath, path, target.absolutePath)
        return try {
            requireNativeOk(result)
            target
        } catch (error: FileNotFoundException) {
            target.delete()
            throw error
        }
    }

    fun writeDocumentFromTemp(documentId: String, source: File) {
        val path = pathForDocumentId(documentId)
        requireNativeOk(NativeCore.providerWriteJson(dataDir.absolutePath, path, source.absolutePath))
    }

    fun emptyWriteTemp(): File = tempFile("write")

    private fun directoryPathForDocument(documentId: String): String {
        if (documentId == ROOT_DOCUMENT_ID) return ""
        val path = pathForDocumentId(documentId)
        val entry = providerEntries().firstOrNull { it.path == path }
            ?: throw FileNotFoundException(documentId)
        if (!entry.isDirectory) throw FileNotFoundException(documentId)
        return path
    }

    private fun rootEntry(): IrisDriveDocumentEntry =
        IrisDriveDocumentEntry(
            documentId = ROOT_DOCUMENT_ID,
            displayName = rootTitle,
            mimeType = DIRECTORY_MIME_TYPE,
            size = 0,
            lastModified = 0,
            isDirectory = true,
            isRoot = true,
        )

    private fun providerEntries(): List<ProviderEntry> {
        val json = JSONObject(NativeCore.providerListJson(dataDir.absolutePath))
        val error = json.optString("error").takeIf { it.isNotBlank() }
        if (error != null) return emptyList()
        return json.optJSONArray("entries").orEmptyObjects().mapNotNull { entry ->
            val path = entry.optString("path")
            if (path.isBlank()) {
                null
            } else {
                ProviderEntry(
                    path = path,
                    parentPath = entry.optString("parent_path"),
                    displayName = entry.optString("display_name"),
                    kind = entry.optString("kind"),
                    size = entry.optLong("size"),
                )
            }
        }
    }

    private fun ProviderEntry.toDocumentEntry(): IrisDriveDocumentEntry =
        IrisDriveDocumentEntry(
            documentId = documentIdForPath(path),
            displayName = displayName,
            mimeType = if (isDirectory) DIRECTORY_MIME_TYPE else inferMimeType(displayName),
            size = if (isDirectory) 0 else size,
            lastModified = 0,
            isDirectory = isDirectory,
            isRoot = false,
        )

    private fun resolvedProviderPath(
        parentPath: String,
        requestedName: String,
        excluding: String? = null,
    ): String {
        val json = JSONObject(
            NativeCore.providerResolvePathJson(
                dataDir.absolutePath,
                parentPath,
                requestedName,
                excluding.orEmpty(),
            ),
        )
        val error = json.optString("error").takeIf { it.isNotBlank() }
        if (error != null) throw FileNotFoundException(error)
        return json.optString("path").takeIf { it.isNotBlank() }
            ?: throw FileNotFoundException("provider path resolver returned no path")
    }

    private fun pathForDocumentId(documentId: String): String {
        if (!documentId.startsWith(ROOT_CHILD_PREFIX)) {
            throw FileNotFoundException(documentId)
        }
        val path = documentId.removePrefix(ROOT_CHILD_PREFIX)
        return NativeCore.normalizedProviderPath(path) ?: throw FileNotFoundException(documentId)
    }

    private fun documentIdForPath(path: String): String {
        val normalized = NativeCore.normalizedProviderPath(path)
            ?: throw FileNotFoundException(path)
        return "$ROOT_DOCUMENT_ID/$normalized"
    }

    private fun tempFile(prefix: String): File {
        val dir = File(dataDir, "provider-tmp")
        dir.mkdirs()
        return File.createTempFile(prefix, ".tmp", dir)
    }

    private fun requireNativeOk(json: String) {
        val error = JSONObject(json).optString("error").takeIf { it.isNotBlank() }
        if (error != null) throw FileNotFoundException(error)
    }

    private data class ProviderEntry(
        val path: String,
        val parentPath: String,
        val displayName: String,
        val kind: String,
        val size: Long,
    ) {
        val isDirectory: Boolean
            get() = kind == "directory"
    }

    companion object {
        const val ROOT_ID = "iris-drive"
        const val ROOT_DOCUMENT_ID = "root"
        private const val ROOT_CHILD_PREFIX = "$ROOT_DOCUMENT_ID/"
        private const val DIRECTORY_MIME_TYPE = "vnd.android.document/directory"
        private const val DEFAULT_FILE_MIME_TYPE = "application/octet-stream"

        fun inferMimeType(displayName: String): String =
            URLConnection.guessContentTypeFromName(displayName) ?: DEFAULT_FILE_MIME_TYPE
    }
}

private fun JSONArray?.orEmptyObjects(): List<JSONObject> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            optJSONObject(index)?.let(::add)
        }
    }
}
