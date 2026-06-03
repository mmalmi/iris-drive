package to.iris.drive.app.sync

import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.Context
import android.util.Log
import to.iris.drive.app.BuildConfig
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore

internal enum class BackgroundSyncAction {
    None,
    RefreshOnly,
    RestartSync,
}

internal object BackgroundSyncPolicy {
    fun shouldSchedule(state: AppState): Boolean =
        actionFor(state) != BackgroundSyncAction.None

    fun actionFor(state: AppState): BackgroundSyncAction {
        if (state.isRevoked || !state.sync.running) {
            return BackgroundSyncAction.None
        }
        if (state.isSetupComplete) {
            return BackgroundSyncAction.RestartSync
        }
        if (state.isAwaitingApproval) {
            return BackgroundSyncAction.RefreshOnly
        }
        return BackgroundSyncAction.None
    }
}

internal class NativeSyncSession(context: Context) : AutoCloseable {
    private val appContext = context.applicationContext
    private var nativeHandle: Long

    init {
        NativeCore.initializeAndroidContext(appContext)
        nativeHandle = NativeCore.appNew(appContext.filesDir.absolutePath, BuildConfig.VERSION_NAME)
    }

    fun refreshState(): AppState =
        AppState.fromJson(NativeCore.refreshJson(nativeHandle))

    fun dispatch(actionJson: String): AppState =
        AppState.fromJson(NativeCore.dispatchJson(nativeHandle, actionJson))

    fun runBackgroundSyncOnce(): AppState {
        val refreshed = refreshState()
        return when (BackgroundSyncPolicy.actionFor(refreshed)) {
            BackgroundSyncAction.RestartSync -> dispatch(NativeActions.restartSync())
            BackgroundSyncAction.RefreshOnly -> refreshed
            BackgroundSyncAction.None -> refreshed
        }
    }

    override fun close() {
        val handle = nativeHandle
        nativeHandle = 0
        if (handle != 0L) {
            NativeCore.appFree(handle)
        }
    }
}

internal object IrisDriveBackgroundSync {
    private const val TAG = "IrisDriveBackgroundSync"
    private const val PERIODIC_JOB_ID = 17322
    private const val PERIODIC_INTERVAL_MS = 15 * 60 * 1_000L

    fun runOnce(context: Context): AppState =
        NativeSyncSession(context).use { session ->
            session.runBackgroundSyncOnce()
        }

    fun pause(context: Context): AppState =
        NativeSyncSession(context).use { session ->
            val state = session.dispatch(NativeActions.stopSync())
            cancel(context)
            state
        }

    fun scheduleIfNeeded(context: Context, state: AppState) {
        if (BackgroundSyncPolicy.shouldSchedule(state)) {
            schedule(context)
        } else {
            cancel(context)
        }
    }

    fun schedule(context: Context) {
        val appContext = context.applicationContext
        val scheduler = appContext.getSystemService(JobScheduler::class.java) ?: return
        if (scheduler.allPendingJobs.any { it.id == PERIODIC_JOB_ID }) return
        val component = ComponentName(appContext, IrisDriveSyncJobService::class.java)
        val job = JobInfo.Builder(PERIODIC_JOB_ID, component)
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
            .setPersisted(true)
            .setPeriodic(PERIODIC_INTERVAL_MS)
            .build()
        runCatching {
            scheduler.schedule(job)
        }.onFailure { error ->
            Log.w(TAG, "Unable to schedule background sync", error)
        }
    }

    fun cancel(context: Context) {
        val scheduler = context.applicationContext.getSystemService(JobScheduler::class.java) ?: return
        scheduler.cancel(PERIODIC_JOB_ID)
    }
}
