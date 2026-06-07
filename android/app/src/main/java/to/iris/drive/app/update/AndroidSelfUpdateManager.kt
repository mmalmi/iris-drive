package to.iris.drive.app.update

import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import java.io.File
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONObject
import to.iris.drive.app.BuildConfig
import to.iris.drive.app.core.NativeCore

data class AndroidSelfUpdateState(
    val supported: Boolean = false,
    val autoCheckEnabled: Boolean = true,
    val checking: Boolean = false,
    val downloading: Boolean = false,
    val available: Boolean = false,
    val version: String = "",
    val asset: String = "",
    val status: String = "",
    val downloaded: Boolean = false,
) {
    val busy: Boolean get() = checking || downloading
}

data class SelfUpdateActions(
    val setAutoCheck: (Boolean) -> Unit,
    val check: () -> Unit,
    val download: () -> Unit,
    val install: () -> Unit,
)

internal data class IrisDriveUpdateResult(
    val available: Boolean = false,
    val latestVersion: String = "",
    val tag: String = "",
    val asset: String = "",
    val path: String = "",
    val error: String = "",
)

class AndroidSelfUpdateManager(
    context: Context,
    private val scope: CoroutineScope,
    private val ioDispatcher: CoroutineDispatcher,
    private val dataDir: File,
) {
    private val appContext = context.applicationContext
    private val prefs: SharedPreferences =
        appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    private val stateFlow =
        MutableStateFlow(
            AndroidSelfUpdateState(
                supported = supportsSelfUpdate(),
                autoCheckEnabled = prefs.getBoolean(AUTO_CHECK_KEY, true),
            ),
        )
    val state: StateFlow<AndroidSelfUpdateState> = stateFlow.asStateFlow()

    private val updateMutex = Mutex()
    private var autoCheckJob: Job? = null
    private var startupCheckDone = false
    private var lastCheckStartedAtMs = 0L
    private var downloadedApk: File? = null

    fun setAutoCheckEnabled(enabled: Boolean) {
        if (!stateFlow.value.supported) return
        stateFlow.update { it.copy(autoCheckEnabled = enabled) }
        prefs.edit().putBoolean(AUTO_CHECK_KEY, enabled).apply()
        if (enabled) {
            startAutomaticChecks()
        } else {
            stopAutomaticChecks()
        }
    }

    fun startAutomaticChecks() {
        val snapshot = stateFlow.value
        if (!snapshot.supported || !snapshot.autoCheckEnabled || autoCheckJob != null) return
        autoCheckJob =
            scope.launch(ioDispatcher) {
                checkIfDue()
                while (isActive) {
                    delay(updatePollIntervalMs())
                    checkIfDue()
                }
            }
    }

    fun stopAutomaticChecks() {
        autoCheckJob?.cancel()
        autoCheckJob = null
    }

    fun check(manual: Boolean = true) {
        if (!stateFlow.value.supported) return
        scope.launch(ioDispatcher) { checkForUpdate(manual = manual) }
    }

    fun download() {
        if (!stateFlow.value.supported) return
        scope.launch(ioDispatcher) { downloadAvailableUpdate() }
    }

    fun install(context: Context) {
        if (!stateFlow.value.supported) return
        val apk = downloadedApk?.takeIf { it.exists() }
        if (apk == null) {
            stateFlow.update { it.copy(status = "Download update first", downloaded = false) }
            return
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
            !appContext.packageManager.canRequestPackageInstalls()
        ) {
            stateFlow.update { it.copy(status = "Allow app installs, then tap Install again") }
            val intent =
                Intent(
                    Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:${appContext.packageName}"),
                )
            context.startActivitySafely(intent)
            return
        }

        val uri =
            FileProvider.getUriForFile(
                appContext,
                "${appContext.packageName}.fileprovider",
                apk,
            )
        val intent =
            Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, APK_MIME_TYPE)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        context.startActivitySafely(intent)
        stateFlow.update { it.copy(status = "Installer opened") }
    }

    private suspend fun checkIfDue() {
        val snapshot = stateFlow.value
        if (snapshot.available || snapshot.downloaded || snapshot.busy) return
        val now = System.currentTimeMillis()
        val due =
            if (!startupCheckDone) {
                startupCheckDone = true
                true
            } else {
                now - lastCheckStartedAtMs >= updatePollIntervalMs()
            }
        if (due) checkForUpdate(manual = false)
    }

    private suspend fun checkForUpdate(manual: Boolean) {
        if (!stateFlow.value.supported) return
        updateMutex.withLock {
            if (stateFlow.value.busy) return
            lastCheckStartedAtMs = System.currentTimeMillis()
            if (manual) {
                stateFlow.update { it.copy(checking = true, status = "Checking for updates") }
            } else {
                stateFlow.update { it.copy(checking = true) }
            }
            try {
                val result = callUpdateCheck()
                if (result.error.isNotBlank()) {
                    throw IllegalStateException(result.error)
                }
                downloadedApk = null
                stateFlow.update {
                    it.copy(
                        checking = false,
                        available = result.available,
                        version = result.versionLabel(),
                        asset = result.asset,
                        downloaded = false,
                        status =
                            when {
                                result.available -> "Update ${result.versionLabel()} available"
                                manual -> "Up to date"
                                else -> ""
                            },
                    )
                }
            } catch (error: Exception) {
                stateFlow.update {
                    it.copy(
                        checking = false,
                        status = if (manual) error.message ?: "Update check failed" else it.status,
                    )
                }
            }
        }
    }

    private suspend fun downloadAvailableUpdate() {
        if (!stateFlow.value.available || stateFlow.value.downloading) return
        updateMutex.withLock {
            stateFlow.update { it.copy(downloading = true, status = "Downloading ${it.version}") }
            try {
                val result = callUpdateDownload()
                if (result.error.isNotBlank()) {
                    throw IllegalStateException(result.error)
                }
                val apk = result.path.takeIf { it.endsWith(".apk", ignoreCase = true) }?.let(::File)
                    ?: throw IllegalStateException("Update did not include an Android APK")
                verifyDownloadedApk(apk)
                downloadedApk = apk
                stateFlow.update {
                    it.copy(
                        downloading = false,
                        available = result.available,
                        version = result.versionLabel().ifBlank { it.version },
                        asset = result.asset.ifBlank { it.asset },
                        downloaded = true,
                        status = "Ready to install",
                    )
                }
            } catch (error: Exception) {
                downloadedApk = null
                stateFlow.update {
                    it.copy(
                        downloading = false,
                        downloaded = false,
                        status = error.message ?: "Download failed",
                    )
                }
            }
        }
    }

    private suspend fun callUpdateCheck(): IrisDriveUpdateResult =
        withContext(ioDispatcher) {
            parseUpdateResult(
                NativeCore.updateCheckJson(dataDir.absolutePath, BuildConfig.VERSION_NAME, "app"),
            )
        }

    private suspend fun callUpdateDownload(): IrisDriveUpdateResult =
        withContext(ioDispatcher) {
            val downloadDir = File(appContext.cacheDir, "updates").apply { mkdirs() }
            downloadDir.listFiles()?.forEach { file ->
                if (file.extension.equals("apk", ignoreCase = true)) file.delete()
            }
            parseUpdateResult(
                NativeCore.updateDownloadJson(
                    dataDir.absolutePath,
                    BuildConfig.VERSION_NAME,
                    "app",
                    downloadDir.absolutePath,
                ),
            )
        }

    private fun verifyDownloadedApk(file: File) {
        val info =
            appContext.packageManager.getPackageArchiveInfo(file.absolutePath, 0)
                ?: throw IllegalStateException("Downloaded file was not an app")
        if (info.packageName != appContext.packageName) {
            throw IllegalStateException("Downloaded app did not match Iris Drive")
        }
        val downloadedVersion = info.longVersionCodeCompat()
        val currentVersion = appContext.packageManager.currentPackageInfo().longVersionCodeCompat()
        if (downloadedVersion <= currentVersion) {
            throw IllegalStateException("Downloaded app was not newer")
        }
    }

    private fun supportsSelfUpdate(): Boolean =
        BuildConfig.SELF_UPDATE_ENABLED && !isKnownStoreInstall()

    private fun isKnownStoreInstall(): Boolean {
        val installer = appContext.packageManager.installerPackageNameCompat(appContext.packageName)
            ?: return false
        return installer in STORE_INSTALLERS
    }

    private fun PackageManager.installerPackageNameCompat(packageName: String): String? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            runCatching { getInstallSourceInfo(packageName).installingPackageName }.getOrNull()
        } else {
            @Suppress("DEPRECATION")
            runCatching { getInstallerPackageName(packageName) }.getOrNull()
        }

    private fun PackageManager.currentPackageInfo(): PackageInfo =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getPackageInfo(appContext.packageName, PackageManager.PackageInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            getPackageInfo(appContext.packageName, 0)
        }

    private fun PackageInfo.longVersionCodeCompat(): Long =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            longVersionCode
        } else {
            @Suppress("DEPRECATION")
            versionCode.toLong()
        }

    private fun Context.startActivitySafely(intent: Intent) {
        val launchIntent = intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { startActivity(launchIntent) }
            .onFailure {
                stateFlow.update { state -> state.copy(status = "Installer unavailable") }
            }
    }

    private companion object {
        private const val APK_MIME_TYPE = "application/vnd.android.package-archive"
        private const val PREFS_NAME = "android_self_update"
        private const val AUTO_CHECK_KEY = "auto_check_enabled"
        private val STORE_INSTALLERS =
            setOf(
                "com.android.vending",
                "com.google.android.feedback",
                "org.fdroid.fdroid",
                "com.zapstore.app",
                "com.sec.android.app.samsungapps",
                "com.amazon.venezia",
            )
    }
}

internal fun parseUpdateResult(body: String): IrisDriveUpdateResult {
    val json = JSONObject(body)
    return IrisDriveUpdateResult(
        available = json.optBoolean("available"),
        latestVersion = json.optString("latest_version"),
        tag = json.optString("tag"),
        asset = json.optString("asset"),
        path = json.optString("path"),
        error = json.optString("error"),
    )
}

internal fun AndroidSelfUpdateState.buttonText(): String =
    when {
        checking -> "Checking..."
        downloading -> "Downloading..."
        downloaded -> "Install update"
        available -> "Download update"
        else -> "Check for updates"
    }

private fun IrisDriveUpdateResult.versionLabel(): String =
    tag.ifBlank { latestVersion }

private fun updatePollIntervalMs(): Long =
    BuildConfig.UPDATE_POLL_SECONDS
        .takeIf { it > 0L }
        ?.let { it * 1_000L }
        ?: (6 * 60 * 60 * 1_000L)
