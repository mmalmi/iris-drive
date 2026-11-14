package to.iris.drive.app.photos

import android.Manifest.permission
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager.PERMISSION_GRANTED
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.webkit.MimeTypeMap

internal data class PhotoAccess(val images: Boolean, val videos: Boolean, val selected: Boolean) {
    val granted: Boolean get() = images || videos || selected

    companion object {
        fun permissions(): Array<String> = when {
            Build.VERSION.SDK_INT >= 34 -> arrayOf(permission.READ_MEDIA_IMAGES, permission.READ_MEDIA_VIDEO, permission.READ_MEDIA_VISUAL_USER_SELECTED)
            Build.VERSION.SDK_INT >= 33 -> arrayOf(permission.READ_MEDIA_IMAGES, permission.READ_MEDIA_VIDEO)
            else -> arrayOf(permission.READ_EXTERNAL_STORAGE)
        }

        fun get(context: Context): PhotoAccess =
            fromGrants(Build.VERSION.SDK_INT) { context.checkSelfPermission(it) == PERMISSION_GRANTED }

        fun fromGrants(api: Int, has: (String) -> Boolean): PhotoAccess {
            if (api < 33) {
                val library = has(permission.READ_EXTERNAL_STORAGE)
                return PhotoAccess(library, library, false)
            }
            return PhotoAccess(has(permission.READ_MEDIA_IMAGES), has(permission.READ_MEDIA_VIDEO),
                api >= 34 && has(permission.READ_MEDIA_VISUAL_USER_SELECTED))
        }
    }
}

internal data class MediaAsset(
    val uri: Uri,
    val identity: String,
    val revision: String,
    val name: String,
    val size: Long,
    val modifiedAtMs: Long = 0L,
)

/** Query all currently visible rows each pass so expanding limited access also finds older items. */
internal suspend fun visitPhotoLibrary(context: Context, access: PhotoAccess, visit: suspend (MediaAsset) -> Boolean) {
    val projection = buildList {
        add(MediaStore.MediaColumns._ID)
        add(MediaStore.MediaColumns.DISPLAY_NAME)
        add(MediaStore.MediaColumns.MIME_TYPE)
        add(MediaStore.MediaColumns.SIZE)
        add(MediaStore.MediaColumns.DATE_MODIFIED)
        add(MediaStore.Images.ImageColumns.DATE_TAKEN)
        if (Build.VERSION.SDK_INT >= 30) add(MediaStore.MediaColumns.GENERATION_MODIFIED)
    }.toTypedArray()
    val volumes = if (Build.VERSION.SDK_INT >= 29) MediaStore.getExternalVolumeNames(context) else setOf("external")
    for (volume in volumes) {
        val version = if (Build.VERSION.SDK_INT >= 30) MediaStore.getVersion(context, volume) else MediaStore.getVersion(context)
        val collections = buildList {
            if (access.images || access.selected) add(MediaStore.Images.Media.getContentUri(volume))
            if (access.videos || access.selected) add(MediaStore.Video.Media.getContentUri(volume))
        }
        val selection = when {
            Build.VERSION.SDK_INT >= 30 -> "is_pending=0 AND is_trashed=0"
            Build.VERSION.SDK_INT >= 29 -> "is_pending=0"
            else -> null
        }
        for (collection in collections) {
            val cursor = context.contentResolver.query(collection, projection, selection, null, "_id ASC")
                ?: error("The Android media library is unavailable")
            cursor.use {
                while (it.moveToNext()) {
                    val id = it.getLong(0)
                    val mime = it.getString(2).orEmpty()
                    val size = it.getLong(3)
                    val modifiedSeconds = it.getLong(4)
                    val capturedMs = it.getLong(5).takeIf { date -> date > 0 } ?: modifiedSeconds * 1000
                    val revision = if (Build.VERSION.SDK_INT >= 30) it.getLong(6) else modifiedSeconds
                    val uri = ContentUris.withAppendedId(collection, id)
                    val identity = "$uri:$version"
                    val original = it.getString(1).orEmpty().ifBlank { "Media-$id" }
                    val ext = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime)
                    val name = if ('.' !in original && !ext.isNullOrEmpty()) "$original.$ext" else original
                    if (!visit(MediaAsset(uri, identity, "$revision:$size", name, size, capturedMs))) return
                }
            }
        }
    }
}
