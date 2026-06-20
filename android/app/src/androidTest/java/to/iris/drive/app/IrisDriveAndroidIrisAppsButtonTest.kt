package to.iris.drive.app

import androidx.activity.compose.setContent
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.ProfileState
import to.iris.drive.app.core.RecoverySecretExport
import to.iris.drive.app.update.AndroidSelfUpdateState
import to.iris.drive.app.update.SelfUpdateActions

@RunWith(AndroidJUnit4::class)
class IrisDriveAndroidIrisAppsButtonTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun openIrisAppsButtonStartsGatewayReadinessEvenBeforePortalUrlExists() {
        var opens = 0
        render(
            state = AppState(
                profile = profileState(),
                setupState = "authorized",
                isSetupComplete = true,
                localNhashResolverEnabled = true,
                sitesPortalUrl = "",
            ),
            onOpenIrisApps = {
                opens += 1
                assertEquals("", it)
            },
        )

        compose.onNodeWithTag("driveContent").performScrollToNode(hasText("Open Iris Apps"))
        compose.onNodeWithText("Open Iris Apps").assertIsEnabled().performClick()

        assertEquals(1, opens)
    }

    private fun render(
        state: AppState,
        onOpenIrisApps: (String) -> Unit = {},
        isOpeningIrisApps: Boolean = false,
    ) {
        compose.setContent {
            IrisDriveAndroidApp(
                stateFlow = MutableStateFlow(state),
                shareDialogFlow = MutableStateFlow(null),
                selfUpdateStateFlow = MutableStateFlow(AndroidSelfUpdateState()),
                backupCheckProgressFlow = MutableStateFlow(BackupCheckProgress()),
                isOpeningIrisAppsFlow = MutableStateFlow(isOpeningIrisApps),
                selfUpdateActions = SelfUpdateActions(
                    setAutoCheck = {},
                    check = {},
                    download = {},
                    install = {},
                ),
                onCreateProfile = {},
                onRestoreProfile = { _, _ -> },
                onLinkDevice = { _, _ -> },
                onCopyText = { _, _ -> },
                onExportRecoverySecret = { RecoverySecretExport() },
                onOpenUrl = {},
                onOpenIrisApps = onOpenIrisApps,
                onOpenDriveFolder = {},
                onApproveDevice = { _, _ -> },
                onRejectDevice = {},
                onResetInvite = {},
                onAddRecoveryKey = {},
                onDeleteDevice = {},
                onAppointAdmin = {},
                onDemoteAdmin = {},
                onLogout = {},
                onAddRelay = {},
                onRemoveRelay = {},
                onResetRelays = {},
                onAddRoot = { _, _ -> },
                onAddBackupTarget = { _, _ -> },
                onRemoveBackupTarget = {},
                onAddBlossomServer = {},
                onRemoveBlossomServer = {},
                onSyncBackups = {},
                onCheckBackups = {},
                onCreateShare = { _, _ -> },
                onInviteShareMember = { _, _, _, _, _, _, _ -> },
                onInviteShareMemberFromEvidence = { _, _, _, _ -> },
                onExportShareRecipientEvidence = {},
                onRecordPendingShareInvite = { _, _, _, _ -> },
                onAcceptShareInvite = {},
                onRevokeShareMember = { _, _ -> },
                onOpenSharePath = {},
                onDeleteShare = {},
                onAddShareShortcut = { _, _ -> },
                onRepairShareWraps = {},
                onStartSync = {},
                onStopSync = {},
            )
        }
        compose.waitForIdle()
    }

    private fun profileState() = ProfileState(
        profileId = "profile-a",
        currentAppKeyNpub = "app-key",
        devicePubkey = "device-a",
        appKeyLabel = "Pixel",
        authorizationState = "authorized",
        canAdminProfile = true,
        appKeyLinkRequest = "",
        appKeyLinkInvite = "https://drive.iris.to/invite/test",
        inboundAppKeyLinkRequests = emptyList(),
    )
}
