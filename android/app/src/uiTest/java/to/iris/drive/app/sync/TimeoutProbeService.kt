package to.iris.drive.app.sync

import android.content.Intent
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue

/** Android-owned lifecycle fixture; the production timeout handler is inherited. */
class TimeoutProbeService : IrisDriveSyncService() {
    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        starts.add(startId)
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        super.onDestroy()
        instance = null
        destroyed.countDown()
    }

    companion object {
        @Volatile
        var instance: TimeoutProbeService? = null
        val starts = LinkedBlockingQueue<Int>()
        var destroyed = CountDownLatch(1)

        fun reset() {
            check(instance == null)
            starts.clear()
            destroyed = CountDownLatch(1)
        }
    }
}
