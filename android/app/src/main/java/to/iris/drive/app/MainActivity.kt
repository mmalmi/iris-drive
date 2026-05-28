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
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.sync.IrisDriveSyncService

class MainActivity : ComponentActivity() {
    private val stateFlow = MutableStateFlow(AppState())
    private var nativeHandle: Long = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        NativeCore.initializeAndroidContext(applicationContext)
        nativeHandle = NativeCore.appNew(filesDir.absolutePath, BuildConfig.VERSION_NAME)
        refresh()
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
                    dispatch(NativeActions.createProfile(deviceLabel))
                },
                onRestoreProfile = { secret, deviceLabel ->
                    dispatch(NativeActions.restoreProfile(secret, deviceLabel))
                },
                onLinkDevice = { ownerPubkey, deviceLabel ->
                    dispatch(NativeActions.linkDevice(ownerPubkey, deviceLabel))
                },
                onCopyText = ::copyToClipboard,
                onOpenUrl = ::openUrl,
                onOpenDriveFolder = ::openDriveFolder,
                onApproveDevice = { request, label ->
                    dispatch(NativeActions.approveDevice(request, label))
                },
                onRevokeDevice = { devicePubkey ->
                    dispatch(NativeActions.revokeDevice(devicePubkey))
                },
                onAppointAdmin = { devicePubkey ->
                    dispatch(NativeActions.appointAdmin(devicePubkey))
                },
                onDemoteAdmin = { devicePubkey ->
                    dispatch(NativeActions.demoteAdmin(devicePubkey))
                },
                onAddRelay = { url -> dispatch(NativeActions.addRelay(url)) },
                onRemoveRelay = { url -> dispatch(NativeActions.removeRelay(url)) },
                onResetRelays = { dispatch(NativeActions.resetRelays()) },
                onAddRoot = { name, path -> dispatch(NativeActions.addRoot(name, path)) },
                onRemoveRoot = { name -> dispatch(NativeActions.removeRoot(name)) },
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
        handleDebugIntent(intent)
    }

    override fun onDestroy() {
        val handle = nativeHandle
        nativeHandle = 0
        if (handle != 0L) {
            NativeCore.appFree(handle)
        }
        super.onDestroy()
    }

    private fun refresh() {
        val handle = nativeHandle
        if (handle == 0L) return
        lifecycleScope.launch(Dispatchers.IO) {
            val state = AppState.fromJson(NativeCore.refreshJson(handle))
            withContext(Dispatchers.Main) {
                stateFlow.value = state
            }
        }
    }

    private fun dispatch(actionJson: String) {
        val handle = nativeHandle
        if (handle == 0L) return
        lifecycleScope.launch(Dispatchers.IO) {
            val state = AppState.fromJson(NativeCore.dispatchJson(handle, actionJson))
            withContext(Dispatchers.Main) {
                stateFlow.value = state
            }
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
        val uri = DocumentsContract.buildRootUri(
            getString(R.string.documents_provider_authority),
            DOCUMENTS_ROOT_ID,
        )
        runCatching {
            startActivity(
                Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
                    .putExtra(DocumentsContract.EXTRA_INITIAL_URI, uri)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    .addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                    .addFlags(Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION),
            )
        }.onFailure {
            Toast.makeText(this, "No app can open the drive folder", Toast.LENGTH_SHORT).show()
        }
    }

    private fun needsNotificationPermission(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.POST_NOTIFICATIONS,
            ) != PackageManager.PERMISSION_GRANTED

    private fun handleDebugIntent(intent: Intent?) {
        val uri = intent?.data
        if (uri != null && isDeviceLinkUri(uri)) {
            dispatch(NativeActions.approveDevice(uri.toString(), "Android"))
        }
        when (intent?.getStringExtra(DEBUG_ACTION_EXTRA)) {
            "create-profile" -> dispatch(NativeActions.createProfile("Android smoke"))
            "add-root" -> dispatch(
                NativeActions.addRoot(
                    "Android smoke",
                    providerRootDocumentUri(),
                ),
            )
            "refresh" -> refresh()
        }
    }

    private fun providerRootDocumentUri(): String =
        DocumentsContract.buildDocumentUri(
            getString(R.string.documents_provider_authority),
            DOCUMENTS_ROOT_DOCUMENT_ID,
        ).toString()

    private fun isDeviceLinkUri(uri: Uri): Boolean =
        (uri.scheme == "iris-drive" && uri.host == "device-link") ||
            (uri.scheme == "https" && uri.host == "drive.iris.to" && uri.path == "/device-link")

    companion object {
        const val DEBUG_ACTION_EXTRA = "to.iris.drive.DEBUG_ACTION"
        private const val DOCUMENTS_ROOT_ID = "iris-drive"
        private const val DOCUMENTS_ROOT_DOCUMENT_ID = "root"
    }
}
