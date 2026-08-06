package to.iris.drive.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.espresso.Espresso.onView
import androidx.test.espresso.NoMatchingViewException
import androidx.test.espresso.action.ViewActions.click
import androidx.test.espresso.assertion.ViewAssertions.matches
import androidx.test.espresso.matcher.ViewMatchers.isDisplayed
import androidx.test.espresso.matcher.ViewMatchers.withText
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import org.hamcrest.Matchers.allOf
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore

@RunWith(AndroidJUnit4::class)
class MainActivityApprovalDeepLinkTest {
    private lateinit var context: Context

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
        NativeCore.initializeAndroidContext(context)
        resetAppStorage()
    }

    @After
    fun tearDown() = resetAppStorage()

    @Test
    fun approvalDeepLinkRequiresExplicitConfirmation() {
        val request = createOwnerAndJoinRequest()

        launch(request).use {
            onView(withText("Approve this device?")).check(matches(isDisplayed()))
            assertOwnerRoster()
            onView(allOf(withText("Cancel"), isDisplayed())).perform(click())
            assertOwnerRoster()
        }

        launch(request).use {
            onView(withText("Approve this device?")).check(matches(isDisplayed()))
            assertOwnerRoster()
            onView(allOf(withText("Approve"), isDisplayed())).perform(click())
            assertEquals(setOf("Android owner", "Phone"), waitForDeviceLabels(2))
        }
    }

    @Test
    fun approvalDeepLinkReceivedDuringNativeStartupIsNotLost() {
        val request = createOwnerAndJoinRequest()
        val scenario = ActivityScenario.launch<MainActivity>(
            Intent(context, MainActivity::class.java)
                .putExtra(MainActivity.DEBUG_NATIVE_START_DELAY_MS, 1_500L),
        )
        lateinit var launchIntent: Intent

        scenario.use {
            it.onActivity { activity ->
                launchIntent = activity.intent
                InstrumentationRegistry.getInstrumentation().callActivityOnNewIntent(
                    activity,
                    Intent(Intent.ACTION_VIEW, Uri.parse(request), context, MainActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }
            waitForApprovalDialog()
            onView(allOf(withText("Approve"), isDisplayed())).perform(click())
            assertEquals(setOf("Android owner", "Phone"), waitForDeviceLabels(2))
            it.onActivity { activity -> activity.intent = launchIntent }
        }
    }

    private fun createOwnerAndJoinRequest(): String {
        val owner = NativeCore.appNew(context.filesDir.absolutePath, "approval-deep-link-test")
        val linkedDir = File(context.cacheDir, "linked-${UUID.randomUUID()}").also { it.mkdirs() }
        val linked = NativeCore.appNew(linkedDir.absolutePath, "approval-deep-link-test")
        return try {
            val ownerState = dispatch(owner, NativeActions.createProfile("Android owner"))
            dispatch(linked, NativeActions.linkDevice(ownerState.profile!!.appKeyLinkInvite, "Phone"))
                .profile!!.appKeyLinkRequest
        } finally {
            NativeCore.appFree(linked)
            NativeCore.appFree(owner)
        }
    }

    private fun launch(request: String): ActivityScenario<MainActivity> =
        ActivityScenario.launch(
            Intent(Intent.ACTION_VIEW, Uri.parse(request), context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )

    private fun dispatch(handle: Long, action: String): AppState {
        NativeCore.dispatchJson(handle, action)
        return AppState.fromJson(NativeCore.stateJson(handle))
    }

    private fun assertOwnerRoster() {
        assertEquals(setOf("Android owner"), waitForDeviceLabels(1))
    }

    private fun waitForDeviceLabels(expectedCount: Int): Set<String> {
        val stateFile = File(context.filesDir, "debug-state.json")
        val deadline = System.currentTimeMillis() + 5_000
        var actual = emptySet<String>()
        while (System.currentTimeMillis() < deadline) {
            actual = runCatching {
                val devices = JSONObject(stateFile.readText())
                    .getJSONObject("ui").getJSONArray("app_actors")
                (0 until devices.length()).mapTo(mutableSetOf()) {
                    devices.getJSONObject(it).getString("label")
                }
            }.getOrDefault(emptySet())
            if (actual.size == expectedCount) return actual
            Thread.sleep(100)
        }
        return actual
    }

    private fun waitForApprovalDialog() {
        val deadline = System.currentTimeMillis() + 5_000
        var failure: Throwable? = null
        while (System.currentTimeMillis() < deadline) {
            try {
                onView(withText("Approve this device?")).check(matches(isDisplayed()))
                return
            } catch (error: NoMatchingViewException) {
                failure = error
            } catch (error: AssertionError) {
                failure = error
            }
            Thread.sleep(50)
        }
        throw AssertionError("Approval confirmation did not appear", failure)
    }

    private fun resetAppStorage() {
        context.filesDir.listFiles()?.forEach(File::deleteRecursively)
        context.cacheDir.listFiles()?.forEach(File::deleteRecursively)
    }
}
