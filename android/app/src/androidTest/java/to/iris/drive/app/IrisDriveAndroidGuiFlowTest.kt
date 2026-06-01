package to.iris.drive.app

import android.content.Context
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.core.RelayStatus
import to.iris.drive.app.core.SyncState
import to.iris.drive.app.provider.IrisDriveDocumentStore

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
        dismissSoftKeyboard()

        val linked = appState(linkedHandle).account
        assertEquals("awaiting_approval", linked?.authorizationState)
        assertTrue(linked?.deviceLinkRequest?.isNotBlank() == true)

        dispatch(owner.handle, NativeActions.approveDevice(linked!!.deviceLinkRequest, "Android UI linked"))
        assertEquals(2, appState(owner.handle).devices.size)
    }

    @Test
    fun linkDeviceSubmitRequiresCompleteNativeLinkInput() {
        render(state = AppState())

        compose.onNodeWithTag("welcomeSignIn").assertIsDisplayed().activate()
        compose.onNodeWithTag("openLinkDevice").assertIsDisplayed().activate()
        compose.onNodeWithTag("linkOwnerInput").assertIsDisplayed().performTextInput("npub1short")

        compose.onNodeWithTag("linkDeviceSubmit").assertIsNotEnabled()
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

        compose.onNodeWithTag("tabDevices").activate()
        compose.onNodeWithTag("devicesContent").performScrollToNode(hasTestTag("addDeviceButton"))
        compose.onNodeWithTag("addDeviceButton").activate()
        compose.onNodeWithTag("manualDeviceId").performScrollTo().assertIsDisplayed()
            .performTextInput(linked.devicePubkey)
        compose.onNodeWithTag("manualDeviceName").performScrollTo().assertIsDisplayed()
            .performTextInput("Android UI linked")
        dismissSoftKeyboard()
        compose.onNodeWithTag("manualDeviceAdd").assertIsEnabled().activate()

        val updated = appState(owner.handle)
        assertEquals(2, updated.devices.size)
        assertTrue(updated.devices.any { it.label == "Android UI linked" })
    }

    @Test
    fun addDeviceDialogRequiresCompleteNativeLinkInput() {
        val state = AppState(
            account = accountState(),
            setupState = "authorized",
        )

        render(state = state)

        compose.onNodeWithTag("tabDevices").activate()
        compose.onNodeWithTag("devicesContent").performScrollToNode(hasTestTag("addDeviceButton"))
        compose.onNodeWithTag("addDeviceButton").activate()
        compose.onNodeWithTag("manualDeviceId").performScrollTo().assertIsDisplayed()
            .performTextInput("npub1short")

        compose.onNodeWithTag("manualDeviceAdd").assertIsNotEnabled()
    }

    @Test
    fun documentsProviderListsNativeProviderRoot() {
        val dataDir = tempDataDir("iris-drive-provider")
        val handle = NativeCore.appNew(dataDir.absolutePath, "ui-test").also(nativeHandles::add)
        dispatch(handle, NativeActions.createProfile("Android UI provider"))
        val source = File(context.cacheDir, "native-provider-source-${UUID.randomUUID()}.txt")
        source.writeText("from native provider")

        val write = JSONObject(
            NativeCore.providerWriteJson(
                dataDir.absolutePath,
                "provider-note.txt",
                source.absolutePath,
            ),
        )
        assertTrue(write.optString("error"), write.optString("error").isBlank())

        val names = IrisDriveDocumentStore(dataDir)
            .childDocuments(IrisDriveDocumentStore.ROOT_DOCUMENT_ID)
            .map { it.displayName }
        assertTrue(names.toString(), names.contains("provider-note.txt"))
    }

    @Test
    fun acceptedLinkedDeviceShowsSyncedFileCountInGui() {
        val ownerDir = tempDataDir("iris-drive-owner")
        val ownerHandle = NativeCore.appNew(ownerDir.absolutePath, "ui-test").also(nativeHandles::add)
        val owner = dispatch(ownerHandle, NativeActions.createProfile("Mac"))
        val source = File(context.cacheDir, "owner-note-${UUID.randomUUID()}.txt")
        source.writeText("from owner")
        val write = JSONObject(
            NativeCore.providerWriteJson(
                ownerDir.absolutePath,
                "owner-note.txt",
                source.absolutePath,
            ),
        )
        assertTrue(write.optString("error"), write.optString("error").isBlank())

        val linkedDir = tempDataDir("iris-drive-linked")
        val linkedHandle = NativeCore.appNew(linkedDir.absolutePath, "ui-test").also(nativeHandles::add)
        val linked = dispatch(
            linkedHandle,
            NativeActions.linkDevice(owner.account!!.deviceLinkInvite, "Pixel"),
        )
        val approved = dispatch(
            ownerHandle,
            NativeActions.approveDevice(linked.account!!.deviceLinkRequest, "Pixel"),
        )
        assertTrue(approved.error, approved.error.isBlank())

        val applied = JSONObject(
            NativeCore.applyOwnerSnapshotForTest(
                ownerDir.absolutePath,
                linkedDir.absolutePath,
            ),
        )
        assertTrue(applied.optString("error"), applied.optString("error").isBlank())

        val linkedState = refreshedAppState(linkedHandle)
        assertEquals("authorized", linkedState.account?.authorizationState)
        assertEquals(1, linkedState.fileCount)

        render(state = linkedState)
        compose.onNodeWithTag("driveContent").performScrollToNode(hasText("1 files"))
        compose.onNodeWithText("1 files").assertIsDisplayed()
    }

    @Test
    fun acceptedLinkedDevicePersistsLoginAndFileCountAfterRestartInGui() {
        val ownerDir = tempDataDir("iris-drive-owner")
        val ownerHandle = NativeCore.appNew(ownerDir.absolutePath, "ui-test").also(nativeHandles::add)
        val owner = dispatch(ownerHandle, NativeActions.createProfile("Mac"))
        val source = File(context.cacheDir, "owner-note-${UUID.randomUUID()}.txt")
        source.writeText("from owner")
        val write = JSONObject(
            NativeCore.providerWriteJson(
                ownerDir.absolutePath,
                "owner-note.txt",
                source.absolutePath,
            ),
        )
        assertTrue(write.optString("error"), write.optString("error").isBlank())

        val linkedDir = tempDataDir("iris-drive-linked")
        val linkedHandle = NativeCore.appNew(linkedDir.absolutePath, "ui-test").also(nativeHandles::add)
        val linked = dispatch(
            linkedHandle,
            NativeActions.linkDevice(owner.account!!.deviceLinkInvite, "Pixel"),
        )
        val approved = dispatch(
            ownerHandle,
            NativeActions.approveDevice(linked.account!!.deviceLinkRequest, "Pixel"),
        )
        assertTrue(approved.error, approved.error.isBlank())

        val applied = JSONObject(
            NativeCore.applyOwnerSnapshotForTest(
                ownerDir.absolutePath,
                linkedDir.absolutePath,
            ),
        )
        assertTrue(applied.optString("error"), applied.optString("error").isBlank())
        NativeCore.appFree(linkedHandle)
        nativeHandles.remove(linkedHandle)

        val restartedHandle = NativeCore.appNew(linkedDir.absolutePath, "ui-test").also(nativeHandles::add)
        val restarted = refreshedAppState(restartedHandle)
        assertEquals("authorized", restarted.account?.authorizationState)
        assertEquals("Pixel", restarted.account?.deviceLabel)
        assertEquals(1, restarted.fileCount)

        render(state = restarted)
        compose.onNodeWithTag("driveContent").performScrollToNode(hasText("1 files"))
        compose.onNodeWithText("1 files").assertIsDisplayed()
    }

    @Test
    fun authenticatedAppShowsBottomTabsAndSeparateDevicesView() {
        val owner = createOwnerProfile("Android UI owner")

        render(state = appState(owner.handle))

        compose.onNodeWithTag("tabMyDrive").assertIsDisplayed()
        compose.onNodeWithTag("tabDevices").assertIsDisplayed()
        compose.onNodeWithTag("tabBackups").assertIsDisplayed()
        compose.onNodeWithTag("tabSettings").assertIsDisplayed()
        compose.onNodeWithTag("driveContent").assertIsDisplayed()

        compose.onNodeWithTag("tabDevices").activate()

        compose.onNodeWithTag("devicesContent").assertIsDisplayed()
        compose.onAllNodesWithTag("driveContent").assertCountEquals(0)
    }

    @Test
    fun settingsViewUsesNativeRelayStatusRows() {
        val state = AppState(
            account = accountState(),
            setupState = "authorized",
            relayStatuses = listOf(
                RelayStatus(
                    url = "wss://relay.example",
                    status = "connected",
                    statusLabel = "connected",
                    health = "online",
                ),
            ),
        )

        render(state = state)

        compose.onNodeWithTag("tabSettings").activate()
        compose.onNodeWithTag("settingsContent").performScrollToNode(hasText("wss://relay.example"))
        compose.onNodeWithText("wss://relay.example").assertIsDisplayed()
        compose.onNodeWithText("connected").assertIsDisplayed()
    }

    @Test
    fun devicesViewUsesOnlineStatusDots() {
        val state = AppState(
            account = accountState(),
            setupState = "authorized",
            devices = listOf(
                deviceState(
                    pubkey = "device-a",
                    label = "Pixel",
                    isCurrentDevice = true,
                    canRevoke = false,
                ),
                deviceState(
                    pubkey = "device-b",
                    label = "Tablet",
                    isCurrentDevice = false,
                    canRevoke = true,
                ),
            ),
        )

        render(state = state)

        compose.onNodeWithTag("tabDevices").activate()
        compose.onNodeWithContentDescription("Pixel online").assertIsDisplayed()
        compose.onNodeWithContentDescription("Tablet offline").assertIsDisplayed()
        compose.onNodeWithTag("deviceStatusDotOnline").assertIsDisplayed()
        compose.onNodeWithTag("deviceStatusDotOffline").assertIsDisplayed()
        compose.onNodeWithText("Admin | Linked | This device").assertIsDisplayed()
        compose.onNodeWithText("Member | Linked | Offline").assertIsDisplayed()
    }

    @Test
    fun acceptedLinkedDeviceThatIsNotOnlineShowsOfflineInGui() {
        val owner = createOwnerProfile("Mac")
        val linked = createLinkedProfile(owner.invite)
        val approved = dispatch(
            owner.handle,
            NativeActions.approveDevice(linked.devicePubkey, "Pixel"),
        )
        assertTrue(approved.error, approved.error.isBlank())
        val pixel = approved.devices.single { it.label == "Pixel" }
        assertFalse(pixel.isOnline)

        render(state = approved)
        compose.onNodeWithText("0/2 devices", substring = true).assertIsDisplayed()
        compose.onNodeWithTag("tabDevices").activate()
        compose.onNodeWithTag("devicesContent").performScrollToNode(hasText("Pixel"))
        compose.onNodeWithText("Pixel").assertIsDisplayed()
        compose.onNodeWithContentDescription("Pixel offline").assertIsDisplayed()
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

    @Test
    fun deleteDeviceRequiresConfirmation() {
        val deletedDevices = mutableListOf<String>()
        val state = AppState(
            account = accountState(),
            setupState = "authorized",
            devices = listOf(
                deviceState(
                    pubkey = "device-a",
                    label = "Pixel",
                    isCurrentDevice = true,
                    canRevoke = false,
                ),
                deviceState(
                    pubkey = "device-b",
                    label = "Tablet",
                    isCurrentDevice = false,
                    canRevoke = true,
                ),
            ),
        )

        render(
            state = state,
            onDeleteDevice = { deletedDevices += it },
        )

        compose.onNodeWithTag("tabDevices").activate()
        compose.onNodeWithTag("devicesContent").performScrollToNode(hasContentDescription("Delete Tablet"))
        compose.onNodeWithContentDescription("Delete Tablet").assertIsDisplayed().activate()
        assertTrue(deletedDevices.isEmpty())
        compose.onNodeWithText("Delete device?").assertIsDisplayed()

        compose.onNodeWithTag("confirmDeleteDevice").assertIsDisplayed().activate()

        assertEquals(listOf("device-b"), deletedDevices)
    }

    private fun render(
        state: AppState,
        onCreateProfile: (String) -> Unit = {},
        onLinkDevice: (String, String) -> Unit = { _, _ -> },
        onApproveDevice: (String, String) -> Unit = { _, _ -> },
        onDeleteDevice: (String) -> Unit = {},
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
                onDeleteDevice = onDeleteDevice,
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
        val dir = tempDataDir("iris-drive-ui")
        return NativeCore.appNew(dir.absolutePath, "ui-test").also(nativeHandles::add)
    }

    private fun tempDataDir(prefix: String): File =
        File(context.cacheDir, "$prefix-${UUID.randomUUID()}").also { it.mkdirs() }

    private fun dispatch(handle: Long, action: String): AppState {
        NativeCore.dispatchJson(handle, action)
        return appState(handle)
    }

    private fun appState(handle: Long): AppState =
        AppState.fromJson(NativeCore.stateJson(handle))

    private fun refreshedAppState(handle: Long): AppState =
        AppState.fromJson(NativeCore.refreshJson(handle))

    private fun SemanticsNodeInteraction.activate() {
        performSemanticsAction(SemanticsActions.OnClick)
    }

    private fun dismissSoftKeyboard() {
        InstrumentationRegistry.getInstrumentation()
            .uiAutomation
            .executeShellCommand("input keyevent KEYCODE_BACK")
            .close()
        compose.waitForIdle()
    }

    private fun accountState() = to.iris.drive.app.core.AccountState(
        ownerPubkey = "owner",
        devicePubkey = "device-a",
        deviceLabel = "Pixel",
        authorizationState = "authorized",
        hasOwnerSigningAuthority = true,
        deviceLinkRequest = "",
        deviceLinkInvite = "iris-drive://invite/test",
        inboundDeviceLinkRequests = emptyList(),
    )

    private fun deviceState(
        pubkey: String,
        label: String,
        isCurrentDevice: Boolean,
        canRevoke: Boolean,
    ) = to.iris.drive.app.core.DeviceState(
        pubkey = pubkey,
        label = label,
        displayLabel = label,
        role = if (isCurrentDevice) "admin" else "member",
        roleLabel = if (isCurrentDevice) "Admin" else "Member",
        state = "linked",
        stateLabel = "Linked",
        connectionState = if (isCurrentDevice) "local" else "offline",
        connectionLabel = if (isCurrentDevice) "This device" else "Offline",
        detail = pubkey,
        isCurrentDevice = isCurrentDevice,
        isOnline = isCurrentDevice,
        canRevoke = canRevoke,
        canAppointAdmin = canRevoke,
        canDemoteAdmin = false,
    )

    private data class TestProfile(
        val handle: Long,
        val invite: String,
        val devicePubkey: String,
    )
}
