package to.iris.drive.app.photos

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import java.io.IOException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONObject
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.sync.NativeSyncSession

internal object PhotoBackupJobs {
    private const val PERIODIC = 17323
    private const val NOW = 17324
    val changes = MutableStateFlow(0L)
    fun notifyChange() = changes.update { it + 1 }

    fun schedule(context: Context) {
        val scheduler = context.getSystemService(JobScheduler::class.java)
        val component = ComponentName(context, PhotoBackupJob::class.java)
        if (scheduler.getPendingJob(PERIODIC) == null) {
            val periodic = JobInfo.Builder(PERIODIC, component)
                .setPeriodic(15 * 60_000L).setPersisted(true).setRequiresStorageNotLow(true).build()
            check(scheduler.schedule(periodic) == JobScheduler.RESULT_SUCCESS) { "Android could not schedule photo backup" }
        }
        if (scheduler.getPendingJob(NOW) == null) {
            val now = JobInfo.Builder(NOW, component).setMinimumLatency(0).setRequiresStorageNotLow(true).build()
            check(scheduler.schedule(now) == JobScheduler.RESULT_SUCCESS) { "Android could not start photo backup" }
        }
    }

    fun disable(context: Context) {
        PhotoBackupJournal(context).use { it.disable() }
        val scheduler = context.getSystemService(JobScheduler::class.java)
        scheduler.cancel(NOW)
        scheduler.cancel(PERIODIC)
        notifyChange()
    }
}

/** Separate local-only jobs keep a large camera roll from occupying network sync's job. */
class PhotoBackupJob : JobService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val runs = mutableMapOf<Int, Job>()

    override fun onStartJob(params: JobParameters): Boolean {
        runs[params.jobId] = scope.launch {
            var retry = false
            try {
                withContext(Dispatchers.IO) {
                    scanLock.withLock { scan() }
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                retry = true
            }
            runs.remove(params.jobId)
            jobFinished(params, retry)
        }
        return true
    }

    override fun onStopJob(params: JobParameters): Boolean {
        runs.remove(params.jobId)?.cancel()
        return true
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private suspend fun scan() {
        PhotoBackupJournal(this).use { journal ->
            val owner = journal.enabledProfile() ?: return
            fun report(message: String) {
                journal.setStatus(owner, message)
                PhotoBackupJobs.notifyChange()
            }
            try {
                val access = PhotoAccess.get(this)
                NativeSyncSession(this).use { session ->
                    val state = session.refreshState()
                    val profile = state.profile
                    if (!state.isSetupComplete || state.isRevoked || profile?.profileId != owner) {
                        report("Waiting for the linked drive")
                        return
                    }
                    val importer = PhotoImporter(this, filesDir, journal, profile)
                    var failed = 0
                    if (access.granted) report("Backing up")
                    if (access.granted) visitPhotoLibrary(this, access) { media ->
                        currentCoroutineContext().ensureActive()
                        if (journal.enabledProfile() != owner) return@visitPhotoLibrary false
                        try {
                            if (importer.import(media)) PhotoBackupJobs.notifyChange()
                        } catch (_: IOException) {
                            failed++
                        } catch (_: SecurityException) {
                            failed++
                        }
                        true
                    }
                    currentCoroutineContext().ensureActive()
                    if (journal.enabledProfile() != owner) return
                    val network = PhotoNetwork.current(this)
                    val allowMetered = journal.allowMetered(owner)
                    val result = JSONObject(NativeCore.photoBackupPublishJson(
                        filesDir.absolutePath, owner, network.allowsUpload(allowMetered),
                    ))
                    val synced = result.optBoolean("ok")
                    val status = publicationStatus(failed, synced, result.optBoolean("pending"), network, allowMetered)
                    report(when {
                        access.granted -> status
                        synced -> "Photo access is required"
                        else -> "Photo access is required. $status"
                    })
                }
            } catch (cancelled: CancellationException) {
                report("Waiting to continue")
                throw cancelled
            } catch (error: Exception) {
                report("Backup interrupted. Will retry.")
                throw error
            }
        }
    }

    companion object { private val scanLock = Mutex() }
}
