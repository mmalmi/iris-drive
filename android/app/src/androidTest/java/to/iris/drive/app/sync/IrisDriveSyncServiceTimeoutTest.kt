package to.iris.drive.app.sync

import android.content.Intent
import android.content.pm.ServiceInfo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.TimeUnit
import org.junit.After
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = 35)
class IrisDriveSyncServiceTimeoutTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val intent = Intent(context, TimeoutProbeService::class.java)

    @Before
    fun resetFixture() {
        TimeoutProbeService.reset()
    }

    @After
    fun stopFixture() {
        instrumentation.runOnMainSync { context.stopService(intent) }
        assertTrue(
            "The test service must be destroyed during cleanup",
            TimeoutProbeService.destroyed.await(5, TimeUnit.SECONDS),
        )
    }

    @Test
    fun dataSyncTimeoutStopsTheStartedService() {
        val startId = startService()
        dispatchTimeout(startId)
        assertDestroyed()
    }

    @Test
    fun dataSyncTimeoutStopsDespiteANewerStartRequest() {
        val expiredStartId = startService()
        val latestStartId = startService()
        assertNotEquals(expiredStartId, latestStartId)

        dispatchTimeout(expiredStartId)
        assertDestroyed()
    }

    private fun startService(): Int {
        instrumentation.runOnMainSync {
            assertNotNull(context.startService(intent))
        }
        return requireNotNull(TimeoutProbeService.starts.poll(5, TimeUnit.SECONDS)) {
            "Android did not deliver onStartCommand"
        }
    }

    private fun dispatchTimeout(startId: Int) {
        instrumentation.runOnMainSync {
            requireNotNull(TimeoutProbeService.instance).onTimeout(
                startId,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
            )
        }
    }

    private fun assertDestroyed() {
        assertTrue(
            "The inherited timeout handler must stop the service promptly",
            TimeoutProbeService.destroyed.await(5, TimeUnit.SECONDS),
        )
    }
}
