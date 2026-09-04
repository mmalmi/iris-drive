package to.iris.drive.app

import android.content.Context
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.sync.IrisDriveBackgroundSync

internal class BackupCheckCoordinator(
    private val context: Context,
    private val scope: CoroutineScope,
    private val nativeDispatcher: CoroutineDispatcher,
    private val progress: MutableStateFlow<BackupCheckProgress>,
    private val state: MutableStateFlow<AppState>,
) {
    fun check(target: String, nativeHandle: Long) {
        if (progress.value.isRunning || nativeHandle == 0L) return
        val targets = if (target.isBlank()) {
            state.value.backups.map { it.target.trim() }.filter { it.isNotEmpty() }
        } else {
            listOf(target.trim()).filter { it.isNotEmpty() }
        }
        if (targets.isEmpty()) return

        progress.value = BackupCheckProgress(0, targets.size, targets.first())
        scope.launch(nativeDispatcher) {
            try {
                for ((index, currentTarget) in targets.withIndex()) {
                    withContext(Dispatchers.Main) {
                        progress.value = BackupCheckProgress(index, targets.size, currentTarget)
                    }
                    val json = NativeCore.dispatchJson(
                        nativeHandle,
                        NativeActions.checkBackups(currentTarget),
                    )
                    val refreshedState = AppState.fromJson(json)
                    withContext(Dispatchers.Main) {
                        state.value = refreshedState
                        AndroidDebugSupport.writeState(context, json)
                        IrisDriveBackgroundSync.scheduleIfNeeded(context, refreshedState)
                        progress.value = BackupCheckProgress(
                            index + 1,
                            targets.size,
                            targets.getOrNull(index + 1).orEmpty(),
                        )
                    }
                }
                delay(350)
            } finally {
                withContext(Dispatchers.Main) { progress.value = BackupCheckProgress() }
            }
        }
    }
}
