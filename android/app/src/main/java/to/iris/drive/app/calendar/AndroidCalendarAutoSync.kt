package to.iris.drive.app.calendar

import android.content.Context
import java.security.MessageDigest
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeCore

internal data class AndroidCalendarAutoSyncResult(
    val synced: Boolean = false,
    val unchanged: Boolean = false,
    val skippedReason: String = "",
    val eventsSynced: Int = 0,
    val eventsDeleted: Int = 0,
    val error: String = "",
)

internal object AndroidCalendarAutoSync {
    private const val PrefsName = "iris_calendar_android_sync"
    private const val KeyEnabled = "enabled"
    private const val KeyLastFingerprint = "last_fingerprint"

    fun isEnabled(context: Context): Boolean =
        prefs(context).getBoolean(KeyEnabled, false)

    fun setEnabled(context: Context, enabled: Boolean) {
        prefs(context).edit().putBoolean(KeyEnabled, enabled).apply()
    }

    fun isActive(context: Context): Boolean =
        isEnabled(context) && AndroidCalendarSync.hasPermissions(context)

    fun syncIfEnabled(
        context: Context,
        state: AppState,
        force: Boolean = false,
    ): AndroidCalendarAutoSyncResult {
        val appContext = context.applicationContext
        if (!state.isSetupComplete) {
            return AndroidCalendarAutoSyncResult(skippedReason = "not_setup")
        }
        if (!isEnabled(appContext)) {
            return AndroidCalendarAutoSyncResult(skippedReason = "disabled")
        }
        if (!AndroidCalendarSync.hasPermissions(appContext)) {
            return AndroidCalendarAutoSyncResult(skippedReason = "permission_required")
        }

        return runCatching {
            val exportJson = NativeCore.exportCalendarJson(appContext.filesDir.absolutePath)
            val fingerprint = calendarFingerprint(exportJson)
            val prefs = prefs(appContext)
            if (!force && prefs.getString(KeyLastFingerprint, "") == fingerprint) {
                return@runCatching AndroidCalendarAutoSyncResult(unchanged = true)
            }
            val snapshot = parseIrisCalendarExportJson(exportJson)
            val result = AndroidCalendarSync.sync(appContext, snapshot)
            prefs.edit().putString(KeyLastFingerprint, fingerprint).apply()
            AndroidCalendarAutoSyncResult(
                synced = true,
                eventsSynced = result.eventsSynced,
                eventsDeleted = result.eventsDeleted,
            )
        }.getOrElse { error ->
            AndroidCalendarAutoSyncResult(
                error = error.message?.takeIf { it.isNotBlank() } ?: "Android Calendar sync failed",
            )
        }
    }

    internal fun calendarFingerprint(value: String): String {
        val digest = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8))
        return digest.joinToString("") { byte -> "%02x".format(byte) }
    }

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(PrefsName, Context.MODE_PRIVATE)
}
