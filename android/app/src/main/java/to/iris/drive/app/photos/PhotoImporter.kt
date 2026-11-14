package to.iris.drive.app.photos

import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.MediaStore
import java.io.File
import java.io.IOException
import java.io.InputStream
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import org.json.JSONObject
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.core.ProfileState

internal class PhotoImporter(
    private val context: Context,
    private val dataDir: File,
    private val journal: PhotoBackupJournal,
    private val profile: ProfileState,
) {
    suspend fun import(media: MediaAsset): Boolean {
        val job = currentCoroutineContext()
        job.ensureActive()
        val sourceId = "android:${profile.devicePubkey}:${media.identity}"
        if (journal.enabledProfile() != profile.profileId || journal.contains(profile.profileId, sourceId, media.revision)) return false
        val staging = File(context.cacheDir, "photo-backup.partial")
        try {
            val input = openOriginal(media.uri)
                ?: throw IOException("Android could not open this item")
            val bytes = input.use { source ->
                staging.outputStream().buffered().use { target ->
                    val buffer = ByteArray(128 * 1024)
                    var total = 0L
                    while (true) {
                        job.ensureActive()
                        val read = source.read(buffer)
                        if (read < 0) break
                        target.write(buffer, 0, read)
                        total += read
                    }
                    total
                }
            }
            if (bytes == 0L || (media.size > 0 && media.size != bytes)) throw IOException("Incomplete media file")
            if (media.modifiedAtMs > 0) staging.setLastModified(media.modifiedAtMs)
            job.ensureActive()
            if (journal.enabledProfile() != profile.profileId) return false
            val result = JSONObject(NativeCore.photoBackupImportJson(
                dataDir.absolutePath, profile.profileId, sourceId, media.name, staging.absolutePath,
            ))
            if (!result.optBoolean("ok")) throw IOException(result.optString("error", "Could not save media"))
            journal.record(profile.profileId, sourceId, media.revision)
            return true
        } finally {
            staging.delete()
        }
    }

    private fun openOriginal(uri: Uri): InputStream? {
        if (Build.VERSION.SDK_INT < 31) return context.contentResolver.openInputStream(uri)
        val options = Bundle().apply { putBoolean(MediaStore.EXTRA_ACCEPT_ORIGINAL_MEDIA_FORMAT, true) }
        return context.contentResolver.openTypedAssetFileDescriptor(uri, "*/*", options)?.createInputStream()
    }
}
