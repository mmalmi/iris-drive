package to.iris.drive.app.core

import android.content.Context
import org.json.JSONObject

internal object NativeCore {
    init {
        System.loadLibrary("iris_drive_app_core")
    }

    external fun initializeAndroidContext(context: Context)
    external fun appNew(dataDir: String, appVersion: String): Long
    external fun appFree(handle: Long)
    external fun stateJson(handle: Long): String
    external fun refreshJson(handle: Long): String
    external fun dispatchJson(handle: Long, actionJson: String): String
    external fun qrMatrixJson(text: String): String
    external fun classifyLinkInputJson(text: String): String
    external fun providerListJson(dataDir: String): String
    external fun providerReadJson(dataDir: String, path: String, outputPath: String): String
    external fun providerWriteJson(dataDir: String, path: String, sourcePath: String): String
    external fun providerMkdirJson(dataDir: String, path: String): String
    external fun providerDeleteJson(dataDir: String, path: String): String
    external fun providerRenameJson(dataDir: String, oldPath: String, newPath: String): String
    external fun providerImportSharedFileJson(
        dataDir: String,
        displayName: String,
        sourcePath: String,
    ): String
    external fun providerResolvePathJson(
        dataDir: String,
        parentPath: String,
        displayName: String,
        excludingPath: String,
    ): String
    external fun applyOwnerSnapshotForTest(ownerDataDir: String, linkedDataDir: String): String

    fun classifyLinkInputKind(text: String): String =
        runCatching {
            JSONObject(classifyLinkInputJson(text)).optString("kind")
        }.getOrDefault("")

    fun isCompleteLinkInput(text: String): Boolean =
        runCatching {
            JSONObject(classifyLinkInputJson(text.trim())).optBoolean("is_complete")
        }.getOrDefault(false)
}
