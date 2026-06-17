package to.iris.drive.app

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.BackupState
import to.iris.drive.app.core.PendingShareInviteState
import to.iris.drive.app.core.RecoverySecretExport
import to.iris.drive.app.core.ShareMemberState
import to.iris.drive.app.core.ShareState
import to.iris.drive.app.update.AndroidSelfUpdateState
import to.iris.drive.app.update.SelfUpdateActions
import to.iris.drive.app.update.buttonText

private data class ShareInvitePrefill(val profileId: String = "", val npubHint: String = "", val displayName: String = "")

@Composable
internal fun AuthenticatedContent(
    padding: PaddingValues,
    selectedTab: MainTab,
    onSelectTab: (MainTab) -> Unit,
    shareDialogRequest: ShareDialogRequest?,
    state: AppState,
    selfUpdateState: AndroidSelfUpdateState,
    selfUpdateActions: SelfUpdateActions,
    backupCheckProgress: BackupCheckProgress,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
    onCopyAppKey: () -> Unit,
    onCopyDeviceKey: () -> Unit,
    onCopyText: (String, String) -> Unit,
    onExportRecoverySecret: () -> RecoverySecretExport,
    onCopyLinkInvite: () -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
    onOpenDriveFolder: () -> Unit,
    onAddRecoveryKey: (String) -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRejectDevice: (String) -> Unit,
    onResetInvite: () -> Unit,
    onDeleteDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
    onLogout: () -> Unit,
    onAddRelay: (String) -> Unit,
    onRemoveRelay: (String) -> Unit,
    onResetRelays: () -> Unit,
    onAddBackupTarget: (String, String) -> Unit,
    onRemoveBackupTarget: (String) -> Unit,
    onAddBlossomServer: (String) -> Unit,
    onRemoveBlossomServer: (String) -> Unit,
    onSyncBackups: (String) -> Unit,
    onCheckBackups: (String) -> Unit,
    onCreateShare: (String, String) -> Unit,
    onInviteShareMember: (String, String, String, String, String, String, String) -> Unit,
    onInviteShareMemberFromEvidence: (String, String, String, String) -> Unit,
    onExportShareRecipientEvidence: (String) -> Unit,
    onRecordPendingShareInvite: (String, String, String, String) -> Unit,
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
    onOpenSharePath: (String) -> Unit,
    onDeleteShare: (String) -> Unit,
    onAddShareShortcut: (String, String) -> Unit,
    onRepairShareWraps: (String) -> Unit,
) {
    when (selectedTab) {
        MainTab.MyDrive -> DriveContent(
            padding = padding,
            state = state,
            selfUpdateState = selfUpdateState,
            selfUpdateActions = selfUpdateActions,
            onShowDevices = { onSelectTab(MainTab.Devices) },
            onStartSync = onStartSync,
            onStopSync = onStopSync,
            onCopySnapshotLink = onCopySnapshotLink,
            onOpenSnapshotLink = onOpenSnapshotLink,
            onOpenDriveFolder = onOpenDriveFolder,
        )
        MainTab.Devices -> DevicesContent(
            padding = padding,
            state = state,
            onCopyLinkInvite = onCopyLinkInvite,
            onAddRecoveryKey = onAddRecoveryKey,
            onApproveDevice = onApproveDevice,
            onRejectDevice = onRejectDevice,
            onResetInvite = onResetInvite,
            onDeleteDevice = onDeleteDevice,
            onAppointAdmin = onAppointAdmin,
            onDemoteAdmin = onDemoteAdmin,
        )
        MainTab.Backups -> BackupsContent(
            padding = padding,
            state = state,
            backupCheckProgress = backupCheckProgress,
            onAddBackupTarget = onAddBackupTarget,
            onRemoveBackupTarget = onRemoveBackupTarget,
            onSyncBackups = onSyncBackups,
            onCheckBackups = onCheckBackups,
        )
        MainTab.Shares -> SharesContent(
            padding = padding,
            state = state,
            shareDialogRequest = shareDialogRequest,
            onCopyText = onCopyText,
            onCreateShare = onCreateShare,
            onInviteShareMember = onInviteShareMember,
            onInviteShareMemberFromEvidence = onInviteShareMemberFromEvidence,
            onExportShareRecipientEvidence = onExportShareRecipientEvidence,
            onRecordPendingShareInvite = onRecordPendingShareInvite,
            onAcceptShareInvite = onAcceptShareInvite,
            onRevokeShareMember = onRevokeShareMember,
            onOpenSharePath = onOpenSharePath,
            onDeleteShare = onDeleteShare,
            onAddShareShortcut = onAddShareShortcut,
            onRepairShareWraps = onRepairShareWraps,
        )
        MainTab.Settings -> SettingsContent(
            padding = padding,
            state = state,
            selfUpdateState = selfUpdateState,
            selfUpdateActions = selfUpdateActions,
            onCopyAppKey = onCopyAppKey,
            onCopyDeviceKey = onCopyDeviceKey,
            onCopyText = onCopyText,
            onExportRecoverySecret = onExportRecoverySecret,
            onLogout = onLogout,
            onAddRelay = onAddRelay,
            onRemoveRelay = onRemoveRelay,
            onResetRelays = onResetRelays,
        )
    }
}

@Composable
private fun DriveContent(
    padding: PaddingValues,
    state: AppState,
    selfUpdateState: AndroidSelfUpdateState,
    selfUpdateActions: SelfUpdateActions,
    onShowDevices: () -> Unit,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
    onOpenDriveFolder: () -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .testTag("driveContent"),
        contentPadding = PaddingValues(18.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (state.error.isNotBlank()) {
            item { Notice(state.error) }
        }
        if (selfUpdateState.supported && (selfUpdateState.available || selfUpdateState.downloaded)) {
            item { SelfUpdateBanner(state = selfUpdateState, actions = selfUpdateActions) }
        }
        item {
            StatusPanel(state = state)
        }
        item {
            SummaryPanel(state = state, onShowDevices = onShowDevices)
        }
        item {
            ProviderPanel(
                snapshotLink = state.snapshotLink,
                onOpenDriveFolder = onOpenDriveFolder,
                onCopySnapshotLink = onCopySnapshotLink,
                onOpenSnapshotLink = onOpenSnapshotLink,
            )
        }
        item {
            SyncPanel(
                state = state,
                onStartSync = onStartSync,
                onStopSync = onStopSync,
            )
        }
    }
}

@Composable
private fun DevicesContent(
    padding: PaddingValues,
    state: AppState,
    onCopyLinkInvite: () -> Unit,
    onAddRecoveryKey: (String) -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRejectDevice: (String) -> Unit,
    onResetInvite: () -> Unit,
    onDeleteDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .testTag("devicesContent"),
        contentPadding = PaddingValues(18.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (state.error.isNotBlank()) {
            item { Notice(state.error) }
        }
        item {
            DevicesPanel(
                devices = state.devices,
                linkInvite = state.profile?.appKeyLinkInvite.orEmpty(),
                inboundRequests = state.profile?.inboundAppKeyLinkRequests.orEmpty(),
                canApprove = state.profile?.canAdminProfile == true,
                onCopyLinkInvite = onCopyLinkInvite,
                onAddRecoveryKey = onAddRecoveryKey,
                onApproveDevice = onApproveDevice,
                onRejectDevice = onRejectDevice,
                onResetInvite = onResetInvite,
                onDeleteDevice = onDeleteDevice,
                onAppointAdmin = onAppointAdmin,
                onDemoteAdmin = onDemoteAdmin,
            )
        }
    }
}

@Composable
private fun BackupsContent(
    padding: PaddingValues,
    state: AppState,
    backupCheckProgress: BackupCheckProgress,
    onAddBackupTarget: (String, String) -> Unit,
    onRemoveBackupTarget: (String) -> Unit,
    onSyncBackups: (String) -> Unit,
    onCheckBackups: (String) -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .testTag("backupsContent"),
        contentPadding = PaddingValues(18.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (state.error.isNotBlank()) {
            item { Notice(state.error) }
        }
        item {
            BackupsPanel(
                backups = state.backups,
                backupCheckProgress = backupCheckProgress,
                onAddBackupTarget = onAddBackupTarget,
                onRemoveBackupTarget = onRemoveBackupTarget,
                onSyncBackups = onSyncBackups,
                onCheckBackups = onCheckBackups,
            )
        }
    }
}

@Composable
private fun SharesContent(
    padding: PaddingValues,
    state: AppState,
    shareDialogRequest: ShareDialogRequest?,
    onCopyText: (String, String) -> Unit,
    onCreateShare: (String, String) -> Unit,
    onInviteShareMember: (String, String, String, String, String, String, String) -> Unit,
    onInviteShareMemberFromEvidence: (String, String, String, String) -> Unit,
    onExportShareRecipientEvidence: (String) -> Unit,
    onRecordPendingShareInvite: (String, String, String, String) -> Unit,
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
    onOpenSharePath: (String) -> Unit,
    onDeleteShare: (String) -> Unit,
    onAddShareShortcut: (String, String) -> Unit,
    onRepairShareWraps: (String) -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .testTag("sharesContent"),
        contentPadding = PaddingValues(18.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (state.error.isNotBlank()) {
            item { Notice(state.error) }
        }
        item {
            SharesPanel(
                state = state,
                shareDialogRequest = shareDialogRequest,
                onCopyText = onCopyText,
                onCreateShare = onCreateShare,
                onInviteShareMember = onInviteShareMember,
                onInviteShareMemberFromEvidence = onInviteShareMemberFromEvidence,
                onExportShareRecipientEvidence = onExportShareRecipientEvidence,
                onRecordPendingShareInvite = onRecordPendingShareInvite,
                onAcceptShareInvite = onAcceptShareInvite,
                onRevokeShareMember = onRevokeShareMember,
                onOpenSharePath = onOpenSharePath,
                onDeleteShare = onDeleteShare,
                onAddShareShortcut = onAddShareShortcut,
                onRepairShareWraps = onRepairShareWraps,
            )
        }
    }
}

@Composable
private fun SettingsContent(
    padding: PaddingValues,
    state: AppState,
    selfUpdateState: AndroidSelfUpdateState,
    selfUpdateActions: SelfUpdateActions,
    onCopyAppKey: () -> Unit,
    onCopyDeviceKey: () -> Unit,
    onCopyText: (String, String) -> Unit,
    onExportRecoverySecret: () -> RecoverySecretExport,
    onLogout: () -> Unit,
    onAddRelay: (String) -> Unit,
    onRemoveRelay: (String) -> Unit,
    onResetRelays: () -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .testTag("settingsContent"),
        contentPadding = PaddingValues(18.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (state.error.isNotBlank()) {
            item { Notice(state.error) }
        }
        item {
            SettingsPanel(
                state = state,
                selfUpdateState = selfUpdateState,
                selfUpdateActions = selfUpdateActions,
                onCopyAppKey = onCopyAppKey,
                onCopyDeviceKey = onCopyDeviceKey,
                onCopyText = onCopyText,
                onExportRecoverySecret = onExportRecoverySecret,
                onLogout = onLogout,
                onAddRelay = onAddRelay,
                onRemoveRelay = onRemoveRelay,
                onResetRelays = onResetRelays,
            )
        }
    }
}

@Composable
private fun StatusPanel(state: AppState) {
    val statusText = state.primaryStatusLabel
    CardSection(title = "My Drive", trailing = statusText.lowercase()) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Image(
                painter = painterResource(id = R.drawable.brand_icon),
                contentDescription = "Iris Drive",
                modifier = Modifier.size(56.dp),
            )
            Spacer(Modifier.size(14.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text("Iris Drive", fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.titleLarge)
                Text(statusText, color = statusColor(state.primaryStatus), fontWeight = FontWeight.SemiBold)
            }
        }
        Text(
            "${state.fileCount} files - ${byteString(state.visibleFileBytes)} - ${state.onlineDeviceCount}/${state.authorizedDeviceCount} devices",
            color = Muted,
        )
    }
}

@Composable
private fun SummaryPanel(state: AppState, onShowDevices: () -> Unit) {
    CardSection(title = "Summary", trailing = "${state.fileCount} files") {
        StatRow("Files", state.fileCount.toString())
        StatRow("Storage", byteString(state.visibleFileBytes))
        TextButton(
            onClick = onShowDevices,
            modifier = Modifier
                .fillMaxWidth()
                .testTag("devicesSummaryButton"),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Devices", color = Muted)
                Text(
                    "${state.onlineDeviceCount}/${state.authorizedDeviceCount} online",
                    color = Ink,
                )
            }
        }
    }
}

@Composable
private fun SyncPanel(
    state: AppState,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
) {
    CardSection(title = "Sync", trailing = state.sync.statusLabel) {
        StatRow("State", state.sync.statusLabel)
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            if (state.sync.running) {
                OutlinedButton(onClick = onStopSync) {
                    Icon(painterResource(R.drawable.ic_stop), contentDescription = null)
                    Spacer(Modifier.size(8.dp))
                    Text("Pause")
                }
            } else {
                Button(onClick = onStartSync) {
                    Icon(painterResource(R.drawable.ic_play), contentDescription = null)
                    Spacer(Modifier.size(8.dp))
                    Text("Resume")
                }
            }
        }
    }
}

@Composable
private fun ProviderPanel(
    snapshotLink: String,
    onOpenDriveFolder: () -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
) {
    CardSection(title = "Files", trailing = "files") {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .background(SoftTeal, RoundedCornerShape(8.dp))
                .padding(14.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(painterResource(R.drawable.ic_drive), contentDescription = null, tint = Teal)
                Spacer(Modifier.size(12.dp))
                Column(Modifier.weight(1f)) {
                    Text("Iris Drive", fontWeight = FontWeight.SemiBold)
                    Text("Available in Android Files", color = Muted, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        Button(onClick = onOpenDriveFolder) {
            Icon(painterResource(R.drawable.ic_drive), contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Open in Files")
        }
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedButton(
                onClick = onCopySnapshotLink,
                enabled = snapshotLink.isNotBlank(),
            ) {
                Text("Copy drive.iris.to link")
            }
            OutlinedButton(
                onClick = onOpenSnapshotLink,
                enabled = snapshotLink.isNotBlank(),
            ) {
                Text("View on drive.iris.to")
            }
        }
    }
}

@Composable
private fun BackupsPanel(
    backups: List<BackupState>,
    backupCheckProgress: BackupCheckProgress,
    onAddBackupTarget: (String, String) -> Unit,
    onRemoveBackupTarget: (String) -> Unit,
    onSyncBackups: (String) -> Unit,
    onCheckBackups: (String) -> Unit,
) {
    var backupInput by remember { mutableStateOf("") }
    var backupLabel by remember { mutableStateOf("") }

    CardSection(title = "Backup", trailing = "${backups.size}") {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                onClick = { onSyncBackups("") },
                enabled = backups.isNotEmpty(),
            ) {
                Text("Sync Now")
            }
            OutlinedButton(
                onClick = { onCheckBackups("") },
                enabled = backups.isNotEmpty() && !backupCheckProgress.isRunning,
            ) {
                Text(if (backupCheckProgress.isRunning) backupCheckProgress.label else "Check All")
            }
        }
        if (backupCheckProgress.isRunning) {
            BackupProgressIndicator(backupCheckProgress)
        }
        OutlinedTextField(
            value = backupInput,
            onValueChange = { backupInput = it },
            label = { Text("Destination URL, User ID, or folder path") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = backupLabel,
            onValueChange = { backupLabel = it },
            label = { Text("Name") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Button(
            onClick = {
                onAddBackupTarget(backupInput, backupLabel)
                backupInput = ""
                backupLabel = ""
            },
            enabled = backupInput.isNotBlank(),
        ) {
            Text("Add Backup")
        }
        if (backups.isEmpty()) {
            Text("No backups configured", color = Muted)
        }
        backups.forEach { backup ->
            val checkingThisTarget =
                backupCheckProgress.isRunning && backupCheckProgress.activeTarget == backup.target
            Text(backup.label, fontWeight = FontWeight.SemiBold)
            Text(
                if (checkingThisTarget) backupCheckProgress.label else backup.state,
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
            )
            Text(backup.detail, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(
                    onClick = { onCheckBackups(backup.target) },
                    enabled = !backupCheckProgress.isRunning,
                ) {
                    Text(if (checkingThisTarget) "Checking 0 of 1" else "Check")
                }
                TextButton(onClick = { onRemoveBackupTarget(backup.target) }) {
                    Text("Remove backup")
                }
            }
        }
    }
}

@Composable
private fun SharesPanel(
    state: AppState,
    shareDialogRequest: ShareDialogRequest?,
    onCopyText: (String, String) -> Unit,
    onCreateShare: (String, String) -> Unit,
    onInviteShareMember: (String, String, String, String, String, String, String) -> Unit,
    onInviteShareMemberFromEvidence: (String, String, String, String) -> Unit,
    onExportShareRecipientEvidence: (String) -> Unit,
    onRecordPendingShareInvite: (String, String, String, String) -> Unit,
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
    onOpenSharePath: (String) -> Unit,
    onDeleteShare: (String) -> Unit,
    onAddShareShortcut: (String, String) -> Unit,
    onRepairShareWraps: (String) -> Unit,
) {
    var sourceInput by remember { mutableStateOf("") }
    var inviteInput by remember { mutableStateOf("") }
    var inviteTarget by remember { mutableStateOf<ShareState?>(null) }
    var revokeTarget by remember { mutableStateOf<Pair<ShareState, ShareMemberState>?>(null) }
    var deleteTarget by remember { mutableStateOf<ShareState?>(null) }
    var invitePrefill by remember { mutableStateOf(ShareInvitePrefill()) }

    LaunchedEffect(shareDialogRequest?.id) {
        val request = shareDialogRequest ?: return@LaunchedEffect
        sourceInput = request.sourcePath
        invitePrefill = ShareInvitePrefill(
            profileId = request.recipientProfileId,
            npubHint = request.recipientNpubHint,
            displayName = request.recipientDisplayName,
        )
    }

    inviteTarget?.let { share ->
        InviteShareMemberDialog(
            share = share,
            prefill = invitePrefill,
            onDismiss = { inviteTarget = null },
            onInvite = { evidenceJson, profileId, appKey, role, npubHint, displayName, label ->
                if (evidenceJson.isNotBlank()) {
                    onInviteShareMemberFromEvidence(share.shareId, evidenceJson, role, displayName)
                } else if (profileId.isBlank() && appKey.isBlank()) {
                    onRecordPendingShareInvite(share.shareId, npubHint, role, displayName)
                } else {
                    onInviteShareMember(share.shareId, profileId, appKey, role, npubHint, displayName, label)
                }
                inviteTarget = null
            },
        )
    }

    revokeTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { revokeTarget = null },
            title = { Text("Revoke access") },
            text = {
                Text(
                    "Revoke ${displayMemberName(target.second)} from ${displayShareName(target.first)}?",
                    color = Muted,
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        onRevokeShareMember(target.first.shareId, target.second.profileId)
                        revokeTarget = null
                    },
                ) {
                    Text("Revoke", color = Danger)
                }
            },
            dismissButton = {
                TextButton(onClick = { revokeTarget = null }) {
                    Text("Cancel")
                }
            },
        )
    }

    deleteTarget?.let { share ->
        AlertDialog(
            onDismissRequest = { deleteTarget = null },
            title = { Text("Delete share?") },
            text = {
                Text(
                    "Delete ${displayShareName(share)} from this device? Folder contents stay in My Drive.",
                    color = Muted,
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        onDeleteShare(share.shareId)
                        deleteTarget = null
                    },
                ) {
                    Text("Delete", color = Danger)
                }
            },
            dismissButton = {
                TextButton(onClick = { deleteTarget = null }) {
                    Text("Cancel")
                }
            },
        )
    }

    CardSection(title = "Shares", trailing = "${state.shares.size}") {
        Text("Create Shared Folder", fontWeight = FontWeight.SemiBold)
        OutlinedTextField(
            value = sourceInput,
            onValueChange = { sourceInput = it },
            label = { Text("Folder path") },
            singleLine = true,
            modifier = Modifier
                .fillMaxWidth()
                .testTag("shareSourceInput"),
        )
        Button(
            onClick = {
                onCreateShare(sourceInput, "")
                sourceInput = ""
            },
            enabled = sourceInput.isNotBlank(),
        ) {
            Text("Create shared folder")
        }

        OutlinedTextField(
            value = inviteInput,
            onValueChange = { inviteInput = it },
            label = { Text("Share invite") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedButton(
                onClick = {
                    onAcceptShareInvite(inviteInput)
                    inviteInput = ""
                },
                enabled = inviteInput.isNotBlank(),
            ) {
                Text("Accept invite")
            }
            OutlinedButton(
                onClick = { onCopyText("Share invite", state.lastShareInvite) },
                enabled = state.lastShareInvite.isNotBlank(),
            ) {
                Text("Copy invite")
            }
        }
        OutlinedButton(
            onClick = {
                onExportShareRecipientEvidence(state.profile?.appKeyLabel.orEmpty())
            },
            enabled = state.profile != null,
            modifier = Modifier.testTag("copyShareIdentityButton"),
        ) {
            Text("Copy my share identity")
        }

        if (state.shares.isEmpty()) {
            Text("No shared folders", color = Muted)
        }
        state.shares.forEach { share ->
            ShareItem(
                share = share,
                localProfileId = state.profile?.profileId.orEmpty(),
                onOpen = { onOpenSharePath(shareOpenPath(share)) },
                onInvite = { inviteTarget = share },
                onRepair = { onRepairShareWraps(share.shareId) },
                onShortcut = { onAddShareShortcut(share.shareId, displayShareName(share)) },
                onDelete = { deleteTarget = share },
                onRevoke = { member -> revokeTarget = share to member },
            )
        }
    }
}

@Composable
private fun InviteShareMemberDialog(
    share: ShareState,
    prefill: ShareInvitePrefill,
    onDismiss: () -> Unit,
    onInvite: (String, String, String, String, String, String, String) -> Unit,
) {
    var evidenceJson by remember { mutableStateOf("") }
    var profileId by remember(share.shareId, prefill.profileId) { mutableStateOf(prefill.profileId) }
    var appKey by remember { mutableStateOf("") }
    var role by remember { mutableStateOf("reader") }
    var npubHint by remember(share.shareId, prefill.npubHint) { mutableStateOf(prefill.npubHint) }
    var displayName by remember(share.shareId, prefill.displayName) { mutableStateOf(prefill.displayName) }
    var label by remember { mutableStateOf("") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Invite to ${displayShareName(share)}") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = evidenceJson,
                    onValueChange = { evidenceJson = it },
                    label = { Text("Recipient identity evidence") },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(112.dp)
                        .testTag("shareRecipientEvidenceInput"),
                    minLines = 3,
                    maxLines = 5,
                )
                OutlinedTextField(
                    value = profileId,
                    onValueChange = { profileId = it },
                    label = { Text("Member profile UUID") },
                    singleLine = true,
                    modifier = Modifier.testTag("shareRecipientProfileInput"),
                )
                OutlinedTextField(
                    value = appKey,
                    onValueChange = { appKey = it },
                    label = { Text("Recipient device key") },
                    singleLine = true,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("reader", "editor", "admin").forEach { option ->
                        OutlinedButton(onClick = { role = option }) {
                            Text(if (role == option) option.uppercase() else option)
                        }
                    }
                }
                OutlinedTextField(
                    value = npubHint,
                    onValueChange = { npubHint = it },
                    label = { Text("User ID") },
                    singleLine = true,
                    modifier = Modifier.testTag("shareRecipientNpubInput"),
                )
                OutlinedTextField(
                    value = displayName,
                    onValueChange = { displayName = it },
                    label = { Text("Name") },
                    singleLine = true,
                    modifier = Modifier.testTag("shareRecipientNameInput"),
                )
                OutlinedTextField(
                    value = label,
                    onValueChange = { label = it },
                    label = { Text("Device label") },
                    singleLine = true,
                )
            }
        },
        confirmButton = {
            TextButton(
                modifier = Modifier.testTag("shareInviteConfirm"),
                onClick = {
                    onInvite(
                        evidenceJson,
                        profileId,
                        appKey,
                        role,
                        npubHint,
                        displayName,
                        label,
                    )
                },
                enabled = evidenceJson.isNotBlank() ||
                    (profileId.isNotBlank() && appKey.isNotBlank()) ||
                    (profileId.isBlank() && appKey.isBlank() && npubHint.isNotBlank()),
            ) {
                Text("Invite")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun ShareItem(
    share: ShareState,
    localProfileId: String,
    onOpen: () -> Unit,
    onInvite: () -> Unit,
    onRepair: () -> Unit,
    onShortcut: () -> Unit,
    onDelete: () -> Unit,
    onRevoke: (ShareMemberState) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(displayShareName(share), fontWeight = FontWeight.SemiBold)
        Text(
            listOfNotNull(
                share.roleLabel.ifBlank { share.role },
                share.keyStatusLabel.ifBlank { share.keyStatus },
                share.sourcePath.takeIf { it.isNotBlank() }?.let(::shortText),
                "${share.participantCount} people",
                share.shortcutPaths.firstOrNull()?.let { "My Drive ${shortText(it)}" },
            ).joinToString(" - "),
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (shareOpenPath(share).isNotBlank()) {
                TextButton(onClick = onOpen) {
                    Text("Open")
                }
            }
            if (share.canAdmin) {
                TextButton(onClick = onInvite) {
                    Text("Invite")
                }
            }
            if (share.repairNeeded || share.missingKeyWraps.isNotEmpty()) {
                TextButton(onClick = onRepair) {
                    Text("Repair")
                }
            }
            if (share.shortcutPaths.isEmpty()) {
                TextButton(onClick = onShortcut) {
                    Text("Add to My Drive")
                }
            }
            TextButton(onClick = onDelete) {
                Text("Delete", color = Danger)
            }
        }
        share.members.forEach { member ->
            ShareMemberRow(
                member = member,
                canRevoke = share.canAdmin && member.status != "revoked" && member.profileId != localProfileId,
                onRevoke = { onRevoke(member) },
            )
        }
        share.pendingInvites.forEach { invite ->
            PendingShareInviteRow(invite)
        }
    }
}

@Composable
private fun ShareMemberRow(
    member: ShareMemberState,
    canRevoke: Boolean,
    onRevoke: () -> Unit,
) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.weight(1f)) {
            Text(displayMemberName(member), color = Ink)
            Text(
                listOf(
                    member.roleLabel.ifBlank { member.role },
                    member.statusLabel.ifBlank { member.status },
                    shortText(member.representativeNpubHint.ifBlank { member.profileId }),
                ).joinToString(" - "),
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (canRevoke) {
            TextButton(onClick = onRevoke) {
                Text("Revoke", color = Danger)
            }
        }
    }
}

@Composable
private fun PendingShareInviteRow(invite: PendingShareInviteState) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(displayPendingInviteName(invite), color = Ink)
        Text(
            listOf(
                invite.roleLabel.ifBlank { invite.role },
                invite.statusLabel.ifBlank { invite.status },
                shortText(invite.representativeNpubHint),
            ).joinToString(" - "),
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun SettingsPanel(
    state: AppState,
    selfUpdateState: AndroidSelfUpdateState,
    selfUpdateActions: SelfUpdateActions,
    onCopyAppKey: () -> Unit,
    onCopyDeviceKey: () -> Unit,
    onCopyText: (String, String) -> Unit,
    onExportRecoverySecret: () -> RecoverySecretExport,
    onLogout: () -> Unit,
    onAddRelay: (String) -> Unit,
    onRemoveRelay: (String) -> Unit,
    onResetRelays: () -> Unit,
) {
    var relayInput by remember { mutableStateOf("") }
    var confirmLogout by remember { mutableStateOf(false) }
    var recoveryExport by remember { mutableStateOf<RecoverySecretExport?>(null) }
    var recoveryWordIndex by remember { mutableStateOf(0) }
    val profile = state.profile

    if (confirmLogout) {
        AlertDialog(
            onDismissRequest = { confirmLogout = false },
            title = { Text("Log out") },
            text = { Text("Remove this local Iris Drive profile from Android?") },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmLogout = false
                        onLogout()
                    },
                ) {
                    Text("Log out")
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmLogout = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    recoveryExport?.let { export ->
        RecoveryPhraseDialog(
            export = export,
            wordIndex = recoveryWordIndex,
            onWordIndexChange = { recoveryWordIndex = it },
            onCopyText = onCopyText,
            onDismiss = { recoveryExport = null },
        )
    }

    CardSection(title = "Settings", trailing = "network") {
        Text("Relays", fontWeight = FontWeight.SemiBold)
        state.relayStatuses.forEach { relay ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Box(
                    modifier = Modifier
                        .size(8.dp)
                        .background(relayHealthColor(relay.health), RoundedCornerShape(4.dp)),
                )
                Spacer(Modifier.size(8.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(relay.url, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(relay.statusLabel, color = Muted, style = MaterialTheme.typography.bodySmall)
                }
                IconButton(onClick = { onRemoveRelay(relay.url) }) {
                    Icon(painterResource(R.drawable.ic_delete), contentDescription = "Remove relay")
                }
            }
        }
        OutlinedTextField(
            value = relayInput,
            onValueChange = { relayInput = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("Relay URL") },
        )
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedButton(
                onClick = {
                    onAddRelay(relayInput)
                    relayInput = ""
                },
                enabled = relayInput.isNotBlank(),
            ) {
                Text("Add relay")
            }
            OutlinedButton(onClick = onResetRelays) {
                Text("Reset relay")
            }
        }
        Text("Device", fontWeight = FontWeight.SemiBold)
        Text(profile?.currentAppKeyNpub.orEmpty(), color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text("Current Device Key", fontWeight = FontWeight.SemiBold)
        Text(profile?.devicePubkey.orEmpty(), color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedButton(onClick = onCopyAppKey) {
                Text("Copy Device")
            }
            OutlinedButton(onClick = onCopyDeviceKey) {
                Text("Copy Device Key")
            }
        }
        if (profile?.canExportRecoveryPhrase == true) {
            OutlinedButton(
                onClick = {
                    recoveryExport = onExportRecoverySecret()
                    recoveryWordIndex = 0
                },
                modifier = Modifier.testTag("openRecoveryPhraseExport"),
            ) {
                Text("Recovery phrase")
            }
        }
        OutlinedButton(onClick = { confirmLogout = true }) {
            Icon(painterResource(R.drawable.ic_delete), contentDescription = null, tint = Danger)
            Spacer(Modifier.size(8.dp))
            Text("Log out", color = Danger)
        }
    }

    if (selfUpdateState.supported) {
        Spacer(Modifier.height(12.dp))
        SelfUpdateCard(state = selfUpdateState, actions = selfUpdateActions)
    }
}

@Composable
private fun SelfUpdateBanner(
    state: AndroidSelfUpdateState,
    actions: SelfUpdateActions,
) {
    CardSection(title = "Update available", trailing = state.version.ifBlank { "ready" }) {
        if (state.status.isNotBlank()) {
            Text(state.status, color = Muted, style = MaterialTheme.typography.bodySmall)
        }
        Button(
            enabled = !state.busy,
            onClick = {
                when {
                    state.downloaded -> actions.install()
                    state.available -> actions.download()
                    else -> actions.check()
                }
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(state.buttonText())
        }
    }
}

@Composable
private fun SelfUpdateCard(
    state: AndroidSelfUpdateState,
    actions: SelfUpdateActions,
) {
    CardSection(title = "Updates", trailing = selfUpdateTrailing(state)) {
        StatRow("Version", BuildConfig.VERSION_NAME)
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Switch(
                checked = state.autoCheckEnabled,
                onCheckedChange = actions.setAutoCheck,
            )
            Spacer(Modifier.size(8.dp))
            Text("Check automatically")
        }
        if (state.status.isNotBlank()) {
            Text(state.status, color = Muted, style = MaterialTheme.typography.bodySmall)
        }
        Button(
            enabled = !state.busy,
            onClick = {
                when {
                    state.downloaded -> actions.install()
                    state.available -> actions.download()
                    else -> actions.check()
                }
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(state.buttonText())
        }
    }
}

private fun selfUpdateTrailing(state: AndroidSelfUpdateState): String =
    when {
        state.downloaded -> "ready"
        state.available -> state.version.ifBlank { "available" }
        state.checking -> "checking"
        state.downloading -> "downloading"
        else -> "current"
    }

@Composable
private fun RecoveryPhraseDialog(
    export: RecoverySecretExport,
    wordIndex: Int,
    onWordIndexChange: (Int) -> Unit,
    onCopyText: (String, String) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Recovery phrase") },
        text = {
            if (export.error.isNotBlank()) {
                Text(export.error, color = Muted)
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("Word ${wordIndex + 1} of $RecoveryPhraseWordCount", color = Muted)
                    Text(
                        export.words.getOrNull(wordIndex).orEmpty(),
                        color = Ink,
                        fontWeight = FontWeight.Bold,
                        style = MaterialTheme.typography.headlineMedium,
                        modifier = Modifier.testTag("recoveryPhraseWord"),
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        OutlinedButton(
                            onClick = { onCopyText("Recovery phrase", export.recoveryPhrase) },
                            modifier = Modifier.weight(1f),
                        ) {
                            Text("Copy recovery phrase")
                        }
                        OutlinedButton(
                            onClick = { onCopyText("Secret key", export.secretKey) },
                            modifier = Modifier.weight(1f),
                        ) {
                            Text("Copy key")
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    if (wordIndex >= RecoveryPhraseWordCount - 1 || export.error.isNotBlank()) {
                        onDismiss()
                    } else {
                        onWordIndexChange(wordIndex + 1)
                    }
                },
            ) {
                Text(if (wordIndex >= RecoveryPhraseWordCount - 1 || export.error.isNotBlank()) "Done" else "Next")
            }
        },
        dismissButton = {
            TextButton(
                onClick = {
                    if (wordIndex == 0) {
                        onDismiss()
                    } else {
                        onWordIndexChange(wordIndex - 1)
                    }
                },
            ) {
                Text(if (wordIndex == 0) "Close" else "Back")
            }
        },
    )
}

@Composable
internal fun Notice(text: String) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.errorContainer, RoundedCornerShape(8.dp))
            .padding(12.dp),
    ) {
        Text(text, color = Danger)
    }
}

@Composable
internal fun CardSection(
    title: String,
    trailing: String,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            SectionHeader(title, trailing)
            content()
        }
    }
}

@Composable
private fun SectionHeader(title: String, trailing: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(title, fontWeight = FontWeight.SemiBold)
        Text(trailing, color = Muted, style = MaterialTheme.typography.labelMedium)
    }
}

@Composable
private fun StatRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, color = Muted)
        Text(value.ifBlank { "-" }, color = Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun relayHealthColor(health: String): Color =
    when (health) {
        "online" -> Color(0xFF16A34A)
        "connecting" -> Color(0xFFF5A524)
        "error" -> Danger
        else -> Muted
    }

@Composable
private fun statusColor(status: String): Color =
    when (status) {
        "ready" -> Teal
        "revoked" -> Danger
        "awaiting_approval" -> Color(0xFFF5A524)
        else -> Muted
    }

private fun byteString(bytes: Long): String {
    if (bytes <= 0L) return "0 bytes"
    val units = listOf("bytes", "KB", "MB", "GB", "TB")
    var value = bytes.toDouble()
    var index = 0
    while (value >= 1000.0 && index < units.lastIndex) {
        value /= 1000.0
        index += 1
    }
    return if (index == 0) {
        "${bytes} bytes"
    } else {
        String.format("%.1f %s", value, units[index])
    }
}

private fun displayShareName(share: ShareState): String =
    share.displayName.ifBlank { "Shared folder" }

private fun displayMemberName(member: ShareMemberState): String =
    member.displayName.ifBlank { "IrisProfile" }

private fun displayPendingInviteName(invite: PendingShareInviteState): String =
    invite.displayName.ifBlank { "Pending contact" }

private fun shareOpenPath(share: ShareState): String =
    share.shortcutPaths.firstOrNull()?.takeIf { it.isNotBlank() }
        ?: share.sourcePath.takeIf { it.isNotBlank() }
        ?: share.sharedWithMePath.takeIf { it.isNotBlank() }
        ?: ""

private fun shortText(value: String): String {
    if (value.length <= 32) return value
    return "${value.take(14)}...${value.takeLast(10)}"
}
