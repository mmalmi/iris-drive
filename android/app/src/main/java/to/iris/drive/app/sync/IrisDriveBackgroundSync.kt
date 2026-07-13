package to.iris.drive.app.sync

import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.Context
import android.util.Log
import to.iris.drive.app.BuildConfig
import to.iris.drive.app.calendar.AndroidCalendarAutoSync
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore

internal enum class BackgroundSyncAction {
    None,
    RefreshOnly,
    RestartSync,
}

internal object BackgroundSyncPolicy {
    fun shouldSchedule(state: AppState, androidCalendarSyncActive: Boolean = false): Boolean =
        actionFor(state) != BackgroundSyncAction.None ||
            shouldScheduleAndroidCalendar(state, androidCalendarSyncActive)

    fun actionFor(state: AppState): BackgroundSyncAction {
        if (state.isRevoked) {
            return BackgroundSyncAction.None
        }
        if (state.isSetupComplete) {
            return if (state.sync.running) {
                BackgroundSyncAction.RestartSync
            } else {
                BackgroundSyncAction.RefreshOnly
            }
        }
        if (state.isAwaitingApproval) {
            return BackgroundSyncAction.RefreshOnly
        }
        return BackgroundSyncAction.None
    }

    private fun shouldScheduleAndroidCalendar(
        state: AppState,
        androidCalendarSyncActive: Boolean,
    ): Boolean =
        androidCalendarSyncActive && state.isSetupComplete && !state.isRevoked
}

internal object BackgroundSyncScheduleGuard {
    const val RESCHEDULE_GUARD_MS = 14 * 60 * 1_000L

    fun shouldAttemptSchedule(
        pending: Boolean,
        desired: Boolean,
        nowMs: Long,
        lastScheduledAtMs: Long,
    ): Boolean {
        if (pending) return false
        if (!desired) return true
        if (lastScheduledAtMs <= 0L) return true
        val elapsedMs = nowMs - lastScheduledAtMs
        return elapsedMs < 0L || elapsedMs >= RESCHEDULE_GUARD_MS
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
        val state = when (BackgroundSyncPolicy.actionFor(refreshed)) {
            BackgroundSyncAction.RestartSync -> dispatch(NativeActions.restartSync())
            BackgroundSyncAction.RefreshOnly -> refreshed
            BackgroundSyncAction.None -> refreshed
        }
        AndroidCalendarAutoSync.syncIfEnabled(appContext, state)
        return state
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
    private const val PREFS_NAME = "iris_drive_background_sync"
    private const val PREF_PERIODIC_DESIRED = "periodic_desired"
    private const val PREF_LAST_SCHEDULED_AT_MS = "last_scheduled_at_ms"
    private const val PERIODIC_JOB_ID = 17322
    private const val PERIODIC_INTERVAL_MS = 15 * 60 * 1_000L

    fun runOnce(context: Context): AppState =
        NativeSyncSession(context).use { session ->
            session.runBackgroundSyncOnce()
        }

    fun scheduleIfNeeded(context: Context, state: AppState) {
        val androidCalendarSyncActive = AndroidCalendarAutoSync.isActive(context)
        if (BackgroundSyncPolicy.shouldSchedule(state, androidCalendarSyncActive)) {
            schedule(context)
        } else {
            cancel(context)
        }
    }

    fun schedule(context: Context) {
        val appContext = context.applicationContext
        val scheduler = appContext.getSystemService(JobScheduler::class.java) ?: return
        val prefs = appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val pending = scheduler.allPendingJobs.any { it.id == PERIODIC_JOB_ID }
        val desired = prefs.getBoolean(PREF_PERIODIC_DESIRED, false)
        val nowMs = System.currentTimeMillis()
        val lastScheduledAtMs = prefs.getLong(PREF_LAST_SCHEDULED_AT_MS, 0L)
        if (!BackgroundSyncScheduleGuard.shouldAttemptSchedule(
                pending = pending,
                desired = desired,
                nowMs = nowMs,
                lastScheduledAtMs = lastScheduledAtMs,
            )
        ) {
            if (pending && !desired) {
                prefs.edit().putBoolean(PREF_PERIODIC_DESIRED, true).apply()
            }
            return
        }
        val component = ComponentName(appContext, IrisDriveSyncJobService::class.java)
        val job = JobInfo.Builder(PERIODIC_JOB_ID, component)
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
            .setPersisted(true)
            .setPeriodic(PERIODIC_INTERVAL_MS)
            .build()
        runCatching {
            scheduler.schedule(job)
        }.onSuccess { result ->
            if (result == JobScheduler.RESULT_SUCCESS) {
                prefs.edit()
                    .putBoolean(PREF_PERIODIC_DESIRED, true)
                    .putLong(PREF_LAST_SCHEDULED_AT_MS, nowMs)
                    .apply()
            }
        }.onFailure { error ->
            Log.w(TAG, "Unable to schedule background sync", error)
        }
    }

    fun cancel(context: Context) {
        val appContext = context.applicationContext
        val scheduler = appContext.getSystemService(JobScheduler::class.java) ?: return
        scheduler.cancel(PERIODIC_JOB_ID)
        appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(PREF_PERIODIC_DESIRED, false)
            .remove(PREF_LAST_SCHEDULED_AT_MS)
            .apply()
    }
}
