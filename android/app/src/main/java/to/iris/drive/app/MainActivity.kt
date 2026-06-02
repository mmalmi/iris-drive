package to.iris.drive.app

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.DocumentsContract
import android.system.Os
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.sync.IrisDriveSyncService

class MainActivity : ComponentActivity() {
    private val stateFlow = MutableStateFlow(AppState())
    private var nativeHandle: Long = 0
    private var refreshJob: Job? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        applyDebugEnvironment(intent)
        NativeCore.initializeAndroidContext(applicationContext)
        nativeHandle = NativeCore.appNew(filesDir.absolutePath, BuildConfig.VERSION_NAME)
        refresh(::autoStartSyncIfNeeded)
        refreshJob = lifecycleScope.launch {
            while (true) {
                delay(2_000)
                refresh()
            }
        }
        handleDebugIntent(intent)

        setContent {
            val notificationPermissionLauncher =
                rememberLauncherForActivityResult(
                    ActivityResultContracts.RequestPermission(),
                ) { granted ->
                    if (granted) {
                        dispatch(NativeActions.startSync())
                        startSyncService()
                    }
                }

            IrisDriveAndroidApp(
                stateFlow = stateFlow,
                onCreateProfile = { deviceLabel ->
                    dispatch(NativeActions.createProfile(resolveDeviceLabel(deviceLabel)), ::autoStartSyncIfNeeded)
                },
                onRestoreProfile = { secret, deviceLabel ->
                    dispatch(
                        NativeActions.restoreProfile(secret, resolveDeviceLabel(deviceLabel)),
                        ::autoStartSyncIfNeeded,
                    )
                },
                onLinkDevice = { ownerPubkey, deviceLabel ->
                    dispatch(
                        NativeActions.linkDevice(ownerPubkey, resolveDeviceLabel(deviceLabel)),
                        ::autoStartSyncIfNeeded,
                    )
                },
                onCopyText = ::copyToClipboard,
                onOpenUrl = ::openUrl,
                onOpenDriveFolder = ::openDriveFolder,
                onApproveDevice = { request, label ->
                    dispatch(NativeActions.approveDevice(request, label), ::autoStartSyncIfNeeded)
                },
                onRejectDevice = { request ->
                    dispatch(NativeActions.rejectDevice(request))
                },
                onResetInvite = { dispatch(NativeActions.resetInvite()) },
                onDeleteDevice = { devicePubkey ->
                    dispatch(NativeActions.deleteDevice(devicePubkey))
                },
                onAppointAdmin = { devicePubkey ->
                    dispatch(NativeActions.appointAdmin(devicePubkey))
                },
                onDemoteAdmin = { devicePubkey ->
                    dispatch(NativeActions.demoteAdmin(devicePubkey))
                },
                onLogout = {
                    stopSyncService()
                    dispatch(NativeActions.logout())
                },
                onAddRelay = { url -> dispatch(NativeActions.addRelay(url)) },
                onRemoveRelay = { url -> dispatch(NativeActions.removeRelay(url)) },
                onResetRelays = { dispatch(NativeActions.resetRelays()) },
                onAddBackupTarget = { target, label ->
                    dispatch(NativeActions.addBackupTarget(target, label))
                },
                onRemoveBackupTarget = { target ->
                    dispatch(NativeActions.removeBackupTarget(target))
                },
                onAddBlossomServer = { url ->
                    dispatch(NativeActions.addBlossomServer(url))
                },
                onRemoveBlossomServer = { url ->
                    dispatch(NativeActions.removeBlossomServer(url))
                },
                onSyncBackups = { target ->
                    dispatch(NativeActions.syncBackups(target))
                },
                onCheckBackups = { target ->
                    dispatch(NativeActions.checkBackups(target))
                },
                onAddRoot = { name, path -> dispatch(NativeActions.addRoot(name, path)) },
                onStartSync = {
                    if (needsNotificationPermission()) {
                        notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                    } else {
                        dispatch(NativeActions.startSync())
                        startSyncService()
                    }
                },
                onStopSync = {
                    dispatch(NativeActions.stopSync())
                    stopSyncService()
                },
            )
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        applyDebugEnvironment(intent)
        handleDebugIntent(intent)
    }

    override fun onDestroy() {
        refreshJob?.cancel()
        refreshJob = null
        val handle = nativeHandle
        nativeHandle = 0
        if (handle != 0L) {
            NativeCore.appFree(handle)
        }
        super.onDestroy()
    }

    private fun refresh(onState: ((AppState) -> Unit)? = null) {
        val handle = nativeHandle
        if (handle == 0L) return
        lifecycleScope.launch(Dispatchers.IO) {
            val state = stateFromJson(NativeCore.refreshJson(handle))
            withContext(Dispatchers.Main) {
                stateFlow.value = state
                writeDebugState()
                onState?.invoke(state)
            }
        }
    }

    private fun dispatch(actionJson: String, onState: ((AppState) -> Unit)? = null) {
        val handle = nativeHandle
        if (handle == 0L) return
        lifecycleScope.launch(Dispatchers.IO) {
            val state = stateFromJson(NativeCore.dispatchJson(handle, actionJson))
            withContext(Dispatchers.Main) {
                stateFlow.value = state
                writeDebugState()
                onState?.invoke(state)
            }
        }
    }

    private fun autoStartSyncIfNeeded(state: AppState) {
        if (state.isRevoked) {
            stopSyncService()
            return
        }
        if (state.sync.running && (state.isSetupComplete || state.isAwaitingApproval)) {
            startSyncService()
        }
    }

    private fun startSyncService() {
        val intent = Intent(this, IrisDriveSyncService::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun stopSyncService() {
        startService(
            Intent(this, IrisDriveSyncService::class.java)
                .setAction(IrisDriveSyncService.ACTION_STOP),
        )
    }

    private fun copyToClipboard(label: String, value: String) {
        if (value.isBlank()) return
        val manager = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        manager.setPrimaryClip(ClipData.newPlainText(label, value))
        Toast.makeText(this, "$label copied", Toast.LENGTH_SHORT).show()
    }

    private fun openUrl(url: String) {
        if (url.isBlank()) return
        runCatching {
            startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
        }.onFailure {
            Toast.makeText(this, "No app can open this link", Toast.LENGTH_SHORT).show()
        }
    }

    private fun openDriveFolder() {
        runCatching {
            startActivity(irisDriveFilesIntent(BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY))
        }.onFailure {
            Toast.makeText(this, "No app can open the drive folder", Toast.LENGTH_SHORT).show()
        }
    }

    private fun stateFromJson(json: String): AppState =
        AppState.fromJson(json)

    private fun resolveDeviceLabel(label: String): String =
        label.trim().ifBlank { defaultDeviceLabel() }

    private fun defaultDeviceLabel(): String {
        val model = Build.MODEL.orEmpty().trim()
        val manufacturer = Build.MANUFACTURER.orEmpty().trim()
        val label = when {
            model.isBlank() -> "Android"
            manufacturer.isBlank() -> model
            model.startsWith(manufacturer, ignoreCase = true) -> model
            model.contains("Pixel", ignoreCase = true) -> model
            else -> "$manufacturer $model"
        }
        return label.replace(Regex("\\s+"), " ").takeIf { it.isNotBlank() } ?: "Android"
    }

    private fun needsNotificationPermission(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.POST_NOTIFICATIONS,
            ) != PackageManager.PERMISSION_GRANTED

    private fun handleDebugIntent(intent: Intent?) {
        val uri = intent?.data
        if (uri != null) {
            when (NativeCore.classifyLinkInputKind(uri.toString())) {
                "device_approval" -> {
                    dispatch(NativeActions.approveDevice(uri.toString(), defaultDeviceLabel()))
                }

                "invite" -> {
                    dispatch(NativeActions.linkDevice(uri.toString(), defaultDeviceLabel()))
                }
            }
        }
        when (intent?.getStringExtra(DEBUG_ACTION_EXTRA)) {
            "create-profile" -> dispatch(NativeActions.createProfile("Android smoke"))
            "link-device" -> {
                val owner = intent.getStringExtra(DEBUG_OWNER_EXTRA).orEmpty()
                dispatch(NativeActions.linkDevice(owner, "Android smoke"))
            }
            "approve-device" -> {
                val request = intent.getStringExtra(DEBUG_REQUEST_EXTRA).orEmpty()
                dispatch(NativeActions.approveDevice(request, "Android smoke"))
            }
            "add-root" -> dispatch(
                NativeActions.addRoot(
                    "Android smoke",
                    providerRootDocumentUri(),
                ),
            )
            "start-sync" -> dispatch(NativeActions.startSync())
            "dump-provider-list" -> writeDebugProviderList()
            "refresh" -> refresh()
        }
    }

    private fun applyDebugEnvironment(intent: Intent?) {
        if (!BuildConfig.DEBUG) return
        val extras = intent?.extras ?: return
        extras.keySet()
            .filter { it.startsWith(DEBUG_ENV_EXTRA_PREFIX) }
            .forEach { key ->
                val value = extras.getString(key) ?: return@forEach
                runCatching {
                    Os.setenv(key, value, true)
                }
            }
    }

    private fun writeDebugState() {
        if (!BuildConfig.DEBUG) return
        runCatching {
            File(filesDir, DEBUG_STATE_FILE).writeText(NativeCore.stateJson(nativeHandle))
        }
    }

    private fun writeDebugProviderList() {
        if (!BuildConfig.DEBUG) return
        runCatching {
            File(filesDir, DEBUG_PROVIDER_LIST_FILE)
                .writeText(NativeCore.providerListJson(filesDir.absolutePath))
        }
    }

    private fun providerRootDocumentUri(): String =
        DocumentsContract.buildDocumentUri(
            BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY,
            DOCUMENTS_ROOT_DOCUMENT_ID,
        ).toString()

    companion object {
        const val DEBUG_ACTION_EXTRA = "to.iris.drive.DEBUG_ACTION"
        const val DEBUG_OWNER_EXTRA = "to.iris.drive.DEBUG_OWNER"
        const val DEBUG_REQUEST_EXTRA = "to.iris.drive.DEBUG_REQUEST"
        const val DEBUG_STATE_FILE = "debug-state.json"
        const val DEBUG_PROVIDER_LIST_FILE = "debug-provider-list.json"
        private const val DEBUG_ENV_EXTRA_PREFIX = "IRIS_DRIVE_"
        private const val DOCUMENTS_ROOT_DOCUMENT_ID = "root"
    }
}
