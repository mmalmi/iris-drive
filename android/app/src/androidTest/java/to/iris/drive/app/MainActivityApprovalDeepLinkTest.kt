package to.iris.drive.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.view.View
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.espresso.Espresso.onView
import androidx.test.espresso.UiController
import androidx.test.espresso.ViewAction
import androidx.test.espresso.assertion.ViewAssertions.matches
import androidx.test.espresso.matcher.RootMatchers.isDialog
import androidx.test.espresso.matcher.ViewMatchers.isClickable
import androidx.test.espresso.matcher.ViewMatchers.isDisplayed
import androidx.test.espresso.matcher.ViewMatchers.isEnabled
import androidx.test.espresso.matcher.ViewMatchers.withText
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import java.util.concurrent.atomic.AtomicLong
import org.hamcrest.Matcher
import org.hamcrest.Matchers.allOf
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore

@RunWith(AndroidJUnit4::class)
class MainActivityApprovalDeepLinkTest {
    private lateinit var context: Context
    private lateinit var blossomServer: String

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
        NativeCore.initializeAndroidContext(context)
        resetAppStorage()
        blossomServer = requireBlossomServerArgument()
    }

    @After
    fun tearDown() = resetAppStorage()

    @Test
    fun approvalDeepLinkRequiresExplicitConfirmation() {
        val request = createOwnerAndJoinRequest()

        launch(request).use {
            waitForApprovalDialog()
            assertOwnerRoster()
            clickApprovalDialogButton("Cancel")
            assertOwnerRoster()
        }

        launch(request).use {
            waitForApprovalDialog()
            assertOwnerRoster()
            clickApprovalDialogButton("Approve")
            assertEquals(setOf("Android owner", "Phone"), waitForDeviceLabels(2))
        }
    }

    @Test
    fun approvalDeepLinkReceivedDuringNativeStartupIsNotLost() {
        val request = createOwnerAndJoinRequest()
        val scenario = ActivityScenario.launch<MainActivity>(
            Intent(context, MainActivity::class.java)
                .putExtra(MainActivity.DEBUG_NATIVE_START_DELAY_MS, 1_500L)
                .putExtra(MainActivity.DEBUG_FIRST_REFRESH_APPLY_DELAY_MS, 1_000L),
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
            clickApprovalDialogButton("Approve")
            assertEquals(setOf("Android owner", "Phone"), waitForDeviceLabels(2))
            it.onActivity { activity -> activity.intent = launchIntent }
        }
    }

    @Test
    fun approvalPromptIsNotBlockedByNativeStartup() {
        val request = createOwnerAndJoinRequest()
        val launchedAt = System.currentTimeMillis()
        val promptObservedAt = AtomicLong()
        val observer = observeApprovalDialog(promptObservedAt)
        val scenario = ActivityScenario.launch<MainActivity>(
            Intent(Intent.ACTION_VIEW, Uri.parse(request), context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
                .putExtra(MainActivity.DEBUG_NATIVE_START_DELAY_MS, 1_500L),
        )

        scenario.use {
            observer.join(3_000)
            waitForApprovalDialog(timeoutMs = 1_000)
            val observedAt = promptObservedAt.get()
            assertTrue(
                "Approval prompt waited for native startup",
                observedAt in launchedAt until launchedAt + 1_000,
            )
            clickApprovalDialogButton("Cancel")
        }
    }

    private fun createOwnerAndJoinRequest(): String {
        val owner = NativeCore.appNew(context.filesDir.absolutePath, "approval-deep-link-test")
        val linkedDir = File(context.cacheDir, "linked-${UUID.randomUUID()}").also { it.mkdirs() }
        val linked = NativeCore.appNew(linkedDir.absolutePath, "approval-deep-link-test")
        return try {
            dispatch(owner, NativeActions.createProfile("Android owner"))
            val ownerState = configureApprovalBlossom(owner)
            dispatch(linked, NativeActions.linkDevice(ownerState.profile!!.appKeyLinkInvite, "Phone"))
                .profile!!.appKeyLinkRequest
        } finally {
            NativeCore.appFree(linked)
            NativeCore.appFree(owner)
        }
    }

    private fun configureApprovalBlossom(owner: Long): AppState {
        val removedDefault = dispatch(
            owner,
            NativeActions.removeBlossomServer(DEFAULT_BLOSSOM_SERVER),
        )
        assertTrue(removedDefault.error, removedDefault.error.isEmpty())
        val configured = dispatch(owner, NativeActions.addBlossomServer(blossomServer))
        assertTrue(configured.error, configured.error.isEmpty())
        val configuredBlossomServers = configured.backups
            .filter { it.kind == "blossom" }
            .mapTo(mutableSetOf()) { it.target }
        assertEquals(setOf(blossomServer), configuredBlossomServers)
        return configured
    }

    private fun requireBlossomServerArgument(): String =
        requireNotNull(
            InstrumentationRegistry.getArguments()
                .getString(BLOSSOM_SERVER_ARGUMENT)
                ?.trim()
                ?.takeIf { it.startsWith("http://127.0.0.1:") },
        ) {
            "Android approval tests require a loopback Blossom fixture via " +
                "instrumentation argument $BLOSSOM_SERVER_ARGUMENT"
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

    private fun waitForApprovalDialog(timeoutMs: Long = 5_000) {
        val deadline = System.currentTimeMillis() + timeoutMs
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        while (System.currentTimeMillis() < deadline) {
            instrumentation.waitForIdleSync()
            val approvalIsVisible = instrumentation.uiAutomation.rootInActiveWindow
                ?.findAccessibilityNodeInfosByText(APPROVAL_DIALOG_TITLE)
                .orEmpty()
                .any { node ->
                    node.isVisibleToUser && node.text?.toString() == APPROVAL_DIALOG_TITLE
                }
            if (approvalIsVisible) {
                onView(withText("Approve this device?"))
                    .inRoot(isDialog())
                    .check(matches(isDisplayed()))
                return
            }
            Thread.sleep(50)
        }
        throw AssertionError("Approval confirmation did not appear")
    }

    private fun observeApprovalDialog(observedAt: AtomicLong): Thread = Thread {
        val uiAutomation = InstrumentationRegistry.getInstrumentation().uiAutomation
        val deadline = System.currentTimeMillis() + 3_000
        while (System.currentTimeMillis() < deadline) {
            val visible = uiAutomation.rootInActiveWindow
                ?.findAccessibilityNodeInfosByText(APPROVAL_DIALOG_TITLE)
                .orEmpty()
                .any { it.isVisibleToUser && it.text?.toString() == APPROVAL_DIALOG_TITLE }
            if (visible) {
                observedAt.compareAndSet(0, System.currentTimeMillis())
                return@Thread
            }
            Thread.sleep(25)
        }
    }.also(Thread::start)

    private fun clickApprovalDialogButton(label: String) {
        onView(allOf(withText(label), isDisplayed(), isEnabled(), isClickable()))
            .inRoot(isDialog())
            .perform(DirectPerformClick)
    }

    private fun resetAppStorage() {
        context.filesDir.listFiles()?.forEach(File::deleteRecursively)
        context.cacheDir.listFiles()?.forEach(File::deleteRecursively)
    }

    private companion object {
        const val BLOSSOM_SERVER_ARGUMENT = "blossom_server"
        const val DEFAULT_BLOSSOM_SERVER = "https://upload.iris.to"
        const val APPROVAL_DIALOG_TITLE = "Approve this device?"

        val DirectPerformClick = object : ViewAction {
            override fun getConstraints(): Matcher<View> =
                allOf(isDisplayed(), isEnabled(), isClickable())

            override fun getDescription(): String =
                "invoke the uniquely matched dialog button without injecting a motion event"

            override fun perform(uiController: UiController, view: View) {
                if (!view.performClick()) {
                    throw AssertionError("Approval confirmation button rejected performClick")
                }
                uiController.loopMainThreadUntilIdle()
            }
        }
    }
}
