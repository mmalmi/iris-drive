package to.iris.drive.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
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
                    if (granted) startSyncService()
                }

            IrisDriveAndroidApp(
                stateFlow = stateFlow,
                onRefresh = ::refresh,
                onAddRoot = { name, path -> dispatch(NativeActions.addRoot(name, path)) },
                onRemoveRoot = { name -> dispatch(NativeActions.removeRoot(name)) },
                onStartSync = {
                    if (needsNotificationPermission()) {
                        notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                    } else {
                        startSyncService()
                    }
                },
                onStopSync = ::stopSyncService,
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

    private fun needsNotificationPermission(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.POST_NOTIFICATIONS,
            ) != PackageManager.PERMISSION_GRANTED

    private fun handleDebugIntent(intent: Intent?) {
        when (intent?.getStringExtra(DEBUG_ACTION_EXTRA)) {
            "add-root" -> dispatch(
                NativeActions.addRoot(
                    "Android smoke",
                    "content://to.iris.drive.documents/root",
                ),
            )
            "refresh" -> refresh()
        }
    }

    companion object {
        const val DEBUG_ACTION_EXTRA = "to.iris.drive.DEBUG_ACTION"
    }
}
