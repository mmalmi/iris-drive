package to.iris.drive.app.sync

import android.app.job.JobParameters
import android.app.job.JobService
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class IrisDriveSyncJobService : JobService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var activeJob: Job? = null

    override fun onStartJob(params: JobParameters?): Boolean {
        if (params == null) return false
        activeJob = scope.launch {
            runCatching {
                val state = IrisDriveBackgroundSync.runOnce(applicationContext)
                IrisDriveBackgroundSync.scheduleIfNeeded(applicationContext, state)
            }.onFailure { error ->
                Log.w(TAG, "Background sync job failed", error)
            }
            withContext(Dispatchers.Main) {
                jobFinished(params, false)
            }
        }
        return true
    }

    override fun onStopJob(params: JobParameters?): Boolean {
        activeJob?.cancel()
        activeJob = null
        return true
    }

    override fun onDestroy() {
        activeJob?.cancel()
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "IrisDriveSyncJobService"
    }
}
