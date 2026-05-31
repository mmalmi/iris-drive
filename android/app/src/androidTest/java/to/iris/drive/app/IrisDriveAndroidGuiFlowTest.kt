package to.iris.drive.app

import android.content.Context
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.util.UUID
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.core.SyncState

@RunWith(AndroidJUnit4::class)
class IrisDriveAndroidGuiFlowTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var context: Context
    private val nativeHandles = mutableListOf<Long>()

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
        NativeCore.initializeAndroidContext(context)
    }

    @After
    fun tearDown() {
        nativeHandles.forEach(NativeCore::appFree)
        nativeHandles.clear()
    }

    @Test
    fun createProfileFlowClicksThroughFirstRunUi() {
        val handle = newNativeHandle()

        render(
            state = AppState(),
            onCreateProfile = { label -> dispatch(handle, NativeActions.createProfile(label)) },
            onAddRoot = { name, path -> dispatch(handle, NativeActions.addRoot(name, path)) },
        )

        compose.onAllNodesWithText("Setup").assertCountEquals(0)
        compose.onNodeWithTag("welcomeCreateProfile").assertIsDisplayed().activate()
        compose.onNodeWithTag("createProfileSubmit").assertIsDisplayed().activate()

        val state = appState(handle)
        assertEquals("authorized", state.account?.authorizationState)
        assertTrue(state.account?.hasOwnerSigningAuthority == true)
        assertEquals(1, state.roots.size)
    }

    @Test
    fun linkThisDeviceFlowClicksThroughSignInUi() {
        val owner = createOwnerProfile("Android UI owner")
        val linkedHandle = newNativeHandle()

        render(
            state = AppState(),
            onLinkDevice = { ownerInvite, label ->
                dispatch(linkedHandle, NativeActions.linkDevice(ownerInvite, label))
            },
        )

        compose.onNodeWithTag("welcomeSignIn").assertIsDisplayed().activate()
        compose.onNodeWithTag("openLinkDevice").assertIsDisplayed().activate()
        compose.onNodeWithTag("linkOwnerInput").assertIsDisplayed().performTextInput(owner.invite)

        val linked = appState(linkedHandle).account
        assertEquals("awaiting_approval", linked?.authorizationState)
        assertTrue(linked?.deviceLinkRequest?.isNotBlank() == true)

        dispatch(owner.handle, NativeActions.approveDevice(linked!!.deviceLinkRequest, "Android UI linked"))
        assertEquals(2, appState(owner.handle).devices.size)
    }

    @Test
    fun linkAnotherDeviceFlowApprovesFromAddDeviceDialog() {
        val owner = createOwnerProfile("Android UI owner")
        val linked = createLinkedProfile(owner.invite)

        render(
            state = appState(owner.handle),
            onApproveDevice = { request, label ->
                dispatch(owner.handle, NativeActions.approveDevice(request, label))
            },
        )

        compose.onNodeWithTag("driveContent").performScrollToNode(hasTestTag("addDeviceButton"))
        compose.onNodeWithTag("addDeviceButton").activate()
        compose.onNodeWithTag("manualDeviceId").performScrollTo().assertIsDisplayed()
            .performTextInput(linked.devicePubkey)
        compose.onNodeWithTag("manualDeviceName").performScrollTo().assertIsDisplayed()
            .performTextInput("Android UI linked")
        compose.onNodeWithTag("manualDeviceAdd").assertIsEnabled().activate()

        val updated = appState(owner.handle)
        assertEquals(2, updated.devices.size)
        assertTrue(updated.devices.any { it.label == "Android UI linked" })
    }

    @Test
    fun documentsProviderListsNativeProviderRoot() {
        cleanTargetFilesDir()
        val handle = NativeCore.appNew(context.filesDir.absolutePath, "ui-test").also(nativeHandles::add)
        dispatch(handle, NativeActions.createProfile("Android UI provider"))
        val source = File(context.cacheDir, "native-provider-source.txt")
        source.writeText("from native provider")

        val write = JSONObject(
            NativeCore.providerWriteJson(
                context.filesDir.absolutePath,
                "provider-note.txt",
                source.absolutePath,
            ),
        )
        assertTrue(write.optString("error"), write.optString("error").isBlank())

        val uri = DocumentsContract.buildChildDocumentsUri(
            context.getString(R.string.documents_provider_authority),
            "root",
        )
        context.contentResolver.query(
            uri,
            arrayOf(Document.COLUMN_DISPLAY_NAME),
            null,
            null,
            null,
        ).use { cursor ->
            assertTrue(cursor != null)
            val names = buildList {
                while (cursor!!.moveToNext()) {
                    add(cursor.getString(0))
                }
            }
            assertTrue(names.toString(), names.contains("provider-note.txt"))
        }
    }

    @Test
    fun syncPanelShowsOnlyTheAvailableAction() {
        val owner = createOwnerProfile("Android UI owner")
        val running = appState(owner.handle)

        val stateFlow = render(state = running)

        compose.onNodeWithTag("driveContent").performScrollToNode(hasText("Pause"))
        compose.onNodeWithText("Pause").assertIsDisplayed()
        compose.onAllNodesWithText("Resume").assertCountEquals(0)

        stateFlow.value = running.copy(sync = SyncState(running = false, status = "paused"))
        compose.waitForIdle()

        compose.onNodeWithTag("driveContent").performScrollToNode(hasText("Resume"))
        compose.onNodeWithText("Resume").assertIsDisplayed()
        compose.onAllNodesWithText("Pause").assertCountEquals(0)
    }

    private fun render(
        state: AppState,
        onCreateProfile: (String) -> Unit = {},
        onLinkDevice: (String, String) -> Unit = { _, _ -> },
        onApproveDevice: (String, String) -> Unit = { _, _ -> },
        onAddRoot: (String, String) -> Unit = { _, _ -> },
    ): MutableStateFlow<AppState> {
        val stateFlow = MutableStateFlow(state)
        compose.setContent {
            IrisDriveAndroidApp(
                stateFlow = stateFlow,
                onCreateProfile = onCreateProfile,
                onRestoreProfile = { _, _ -> },
                onLinkDevice = onLinkDevice,
                onCopyText = { _, _ -> },
                onOpenUrl = { _ -> },
                onOpenDriveFolder = {},
                onApproveDevice = onApproveDevice,
                onResetInvite = {},
                onRevokeDevice = { _ -> },
                onAppointAdmin = { _ -> },
                onDemoteAdmin = { _ -> },
                onLogout = {},
                onAddRelay = { _ -> },
                onRemoveRelay = { _ -> },
                onResetRelays = {},
                onAddRoot = onAddRoot,
                onStartSync = {},
                onStopSync = {},
            )
        }
        return stateFlow
    }

    private fun createOwnerProfile(label: String): TestProfile {
        val handle = newNativeHandle()
        val state = dispatch(handle, NativeActions.createProfile(label))
        val account = state.account ?: error("owner account missing")
        return TestProfile(
            handle = handle,
            invite = account.deviceLinkInvite,
            devicePubkey = account.devicePubkey,
        )
    }

    private fun createLinkedProfile(invite: String): TestProfile {
        val handle = newNativeHandle()
        val state = dispatch(handle, NativeActions.linkDevice(invite, "Android UI linked"))
        val account = state.account ?: error("linked account missing")
        assertEquals("awaiting_approval", account.authorizationState)
        return TestProfile(
            handle = handle,
            invite = account.deviceLinkInvite,
            devicePubkey = account.devicePubkey,
        )
    }

    private fun newNativeHandle(): Long {
        val dir = File(context.cacheDir, "iris-drive-ui-${UUID.randomUUID()}")
        dir.mkdirs()
        return NativeCore.appNew(dir.absolutePath, "ui-test").also(nativeHandles::add)
    }

    private fun cleanTargetFilesDir() {
        context.filesDir.listFiles().orEmpty().forEach { it.deleteRecursively() }
    }

    private fun dispatch(handle: Long, action: String): AppState {
        NativeCore.dispatchJson(handle, action)
        return appState(handle)
    }

    private fun appState(handle: Long): AppState =
        AppState.fromJson(NativeCore.stateJson(handle))

    private fun SemanticsNodeInteraction.activate() {
        performSemanticsAction(SemanticsActions.OnClick)
    }

    private data class TestProfile(
        val handle: Long,
        val invite: String,
        val devicePubkey: String,
    )
}
