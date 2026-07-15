package to.iris.drive.app.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.drive.app.MainActivity
import to.iris.drive.app.R

class IrisDriveSyncService : Service() {
    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var syncLoop: Job? = null

    override fun onCreate() {
        super.onCreate()
        ensureNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopForegroundCompat()
            stopSelf()
            return START_NOT_STICKY
        }
        if (!startForegroundCompat()) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        startSyncLoop()
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        syncLoop?.cancel()
        syncLoop = null
        serviceScope.cancel()
        super.onDestroy()
    }

    private fun startSyncLoop() {
        if (syncLoop?.isActive == true) return
        syncLoop = serviceScope.launch {
            NativeSyncSession(applicationContext).use { session ->
                while (isActive) {
                    val state = session.runBackgroundSyncOnce()
                    IrisDriveBackgroundSync.scheduleIfNeeded(applicationContext, state)
                    if (!BackgroundSyncPolicy.shouldSchedule(state)) {
                        withContext(Dispatchers.Main) {
                            stopForegroundCompat()
                            stopSelf()
                        }
                        return@launch
                    }
                    delay(FOREGROUND_SYNC_INTERVAL_MS)
                }
            }
        }
    }

    private fun startForegroundCompat(): Boolean {
        val notification = buildNotification()
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(
                    NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
                )
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
            true
        } catch (error: RuntimeException) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
                error.javaClass.name == "android.app.ForegroundServiceStartNotAllowedException"
            ) {
                false
            } else {
                throw error
            }
        }
    }

    private fun stopForegroundCompat() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
    }

    private fun buildNotification(): Notification {
        val openAppIntent =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_drive)
            .setContentTitle(getString(R.string.app_name))
            .setContentText("Sync service active")
            .setOngoing(true)
            .setContentIntent(openAppIntent)
            .build()
    }

    private fun ensureNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NotificationManager::class.java)
        val channel =
            NotificationChannel(
                CHANNEL_ID,
                getString(R.string.sync_notification_channel),
                NotificationManager.IMPORTANCE_LOW,
            )
        manager.createNotificationChannel(channel)
    }

    companion object {
        const val ACTION_STOP = "to.iris.drive.action.STOP_SYNC"
        private const val CHANNEL_ID = "iris-drive-sync"
        private const val NOTIFICATION_ID = 17321
        private const val FOREGROUND_SYNC_INTERVAL_MS = 60_000L
    }
}
