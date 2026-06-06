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
import to.iris.drive.app.core.RecoverySecretExport
import to.iris.drive.app.core.ShareMemberState
import to.iris.drive.app.core.ShareState

private data class ShareInvitePrefill(
    val profileId: String = "",
    val npubHint: String = "",
    val displayName: String = "",
)

@Composable
internal fun AuthenticatedContent(
    padding: PaddingValues,
    selectedTab: MainTab,
    onSelectTab: (MainTab) -> Unit,
    shareDialogRequest: ShareDialogRequest?,
    state: AppState,
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
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
    onAddShareShortcut: (String, String) -> Unit,
    onRepairShareWraps: (String) -> Unit,
) {
    when (selectedTab) {
        MainTab.MyDrive -> DriveContent(
            padding = padding,
            state = state,
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
            onAddBackupTarget = onAddBackupTarget,
            onRemoveBackupTarget = onRemoveBackupTarget,
            onAddBlossomServer = onAddBlossomServer,
            onRemoveBlossomServer = onRemoveBlossomServer,
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
            onAcceptShareInvite = onAcceptShareInvite,
            onRevokeShareMember = onRevokeShareMember,
            onAddShareShortcut = onAddShareShortcut,
            onRepairShareWraps = onRepairShareWraps,
        )
        MainTab.Settings -> SettingsContent(
            padding = padding,
            state = state,
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
    onAddBackupTarget: (String, String) -> Unit,
    onRemoveBackupTarget: (String) -> Unit,
    onAddBlossomServer: (String) -> Unit,
    onRemoveBlossomServer: (String) -> Unit,
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
                onAddBackupTarget = onAddBackupTarget,
                onRemoveBackupTarget = onRemoveBackupTarget,
                onAddBlossomServer = onAddBlossomServer,
                onRemoveBlossomServer = onRemoveBlossomServer,
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
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
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
                onAcceptShareInvite = onAcceptShareInvite,
                onRevokeShareMember = onRevokeShareMember,
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
            "${state.fileCount} files - ${byteString(state.visibleFileBytes)} - ${state.onlineDeviceCount}/${state.authorizedDeviceCount} AppKeys",
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
                Text("AppKeys", color = Muted)
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
    onAddBackupTarget: (String, String) -> Unit,
    onRemoveBackupTarget: (String) -> Unit,
    onAddBlossomServer: (String) -> Unit,
    onRemoveBlossomServer: (String) -> Unit,
    onSyncBackups: (String) -> Unit,
    onCheckBackups: (String) -> Unit,
) {
    var backupInput by remember { mutableStateOf("") }
    var backupLabel by remember { mutableStateOf("") }
    var blossomInput by remember { mutableStateOf("") }

    CardSection(title = "Backups", trailing = "${backups.size}") {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                onClick = { onSyncBackups("") },
                enabled = backups.isNotEmpty(),
            ) {
                Text("Sync Now")
            }
            OutlinedButton(
                onClick = { onCheckBackups("") },
                enabled = backups.isNotEmpty(),
            ) {
                Text("Check All")
            }
        }
        OutlinedTextField(
            value = backupInput,
            onValueChange = { backupInput = it },
            label = { Text("Destination") },
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
        OutlinedTextField(
            value = blossomInput,
            onValueChange = { blossomInput = it },
            label = { Text("Blossom endpoint") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Button(
            onClick = {
                onAddBlossomServer(blossomInput)
                blossomInput = ""
            },
            enabled = blossomInput.isNotBlank(),
        ) {
            Text("Add Blossom")
        }
        if (backups.isEmpty()) {
            Text("No Blossom remotes configured", color = Muted)
        }
        backups.forEach { backup ->
            Text(backup.label, fontWeight = FontWeight.SemiBold)
            Text(backup.state, color = Muted, style = MaterialTheme.typography.bodySmall)
            Text(backup.detail, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = { onCheckBackups(backup.target) }) {
                    Text("Check")
                }
                TextButton(onClick = { onRemoveBackupTarget(backup.target) }) {
                    Text("Remove backup")
                }
                if (backup.kind == "blossom") {
                    TextButton(onClick = { onRemoveBlossomServer(backup.target) }) {
                        Text("Remove Blossom")
                    }
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
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
    onAddShareShortcut: (String, String) -> Unit,
    onRepairShareWraps: (String) -> Unit,
) {
    var sourceInput by remember { mutableStateOf("") }
    var nameInput by remember { mutableStateOf("") }
    var inviteInput by remember { mutableStateOf("") }
    var inviteTarget by remember { mutableStateOf<ShareState?>(null) }
    var revokeTarget by remember { mutableStateOf<Pair<ShareState, ShareMemberState>?>(null) }
    var invitePrefill by remember { mutableStateOf(ShareInvitePrefill()) }

    LaunchedEffect(shareDialogRequest?.id) {
        val request = shareDialogRequest ?: return@LaunchedEffect
        sourceInput = request.sourcePath
        nameInput = request.displayName
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

    CardSection(title = "Shares", trailing = "${state.shares.size}") {
        OutlinedTextField(
            value = sourceInput,
            onValueChange = { sourceInput = it },
            label = { Text("Folder path") },
            singleLine = true,
            modifier = Modifier
                .fillMaxWidth()
                .testTag("shareSourceInput"),
        )
        OutlinedTextField(
            value = nameInput,
            onValueChange = { nameInput = it },
            label = { Text("Name") },
            singleLine = true,
            modifier = Modifier
                .fillMaxWidth()
                .testTag("shareNameInput"),
        )
        Button(
            onClick = {
                onCreateShare(sourceInput, nameInput)
                sourceInput = ""
                nameInput = ""
            },
            enabled = sourceInput.isNotBlank(),
        ) {
            Text("Create share")
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

        if (state.shares.isEmpty()) {
            Text("No shared folders", color = Muted)
        }
        state.shares.forEach { share ->
            ShareItem(
                share = share,
                localProfileId = state.profile?.profileId.orEmpty(),
                onInvite = { inviteTarget = share },
                onRepair = { onRepairShareWraps(share.shareId) },
                onShortcut = { onAddShareShortcut(share.shareId, displayShareName(share)) },
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
                    label = { Text("Recipient AppActor pubkey") },
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
                    label = { Text("Contact npub") },
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
                    label = { Text("AppActor label") },
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
                enabled = evidenceJson.isNotBlank() || (profileId.isNotBlank() && appKey.isNotBlank()),
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
    onInvite: () -> Unit,
    onRepair: () -> Unit,
    onShortcut: () -> Unit,
    onRevoke: (ShareMemberState) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(displayShareName(share), fontWeight = FontWeight.SemiBold)
        Text(
            listOfNotNull(
                share.roleLabel.ifBlank { share.role },
                share.keyStatusLabel.ifBlank { share.keyStatus },
                "${share.participantCount} people",
                share.shortcutPaths.firstOrNull()?.let { "shortcut ${shortText(it)}" },
            ).joinToString(" - "),
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
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
                    Text("Shortcut")
                }
            }
        }
        share.members.forEach { member ->
            ShareMemberRow(
                member = member,
                canRevoke = share.canAdmin && member.status != "revoked" && member.profileId != localProfileId,
                onRevoke = { onRevoke(member) },
            )
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
private fun SettingsPanel(
    state: AppState,
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
        Text("AppKey", fontWeight = FontWeight.SemiBold)
        Text(profile?.currentAppKeyNpub.orEmpty(), color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text("Current AppKey", fontWeight = FontWeight.SemiBold)
        Text(profile?.devicePubkey.orEmpty(), color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedButton(onClick = onCopyAppKey) {
                Text("Copy AppKey")
            }
            OutlinedButton(onClick = onCopyDeviceKey) {
                Text("Copy AppKey")
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

private fun shortText(value: String): String {
    if (value.length <= 32) return value
    return "${value.take(14)}...${value.takeLast(10)}"
}
