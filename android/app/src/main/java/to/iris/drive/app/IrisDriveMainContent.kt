package to.iris.drive.app

import android.content.Intent
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
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import to.iris.drive.app.core.AppState
import to.iris.drive.app.photos.PhotoBackupSettings
import to.iris.drive.app.core.BackupState
import to.iris.drive.app.core.PendingShareInviteState
import to.iris.drive.app.core.RecoverySecretExport
import to.iris.drive.app.core.ShareMemberState
import to.iris.drive.app.core.ShareState
import to.iris.drive.app.update.AndroidSelfUpdateState
import to.iris.drive.app.update.SelfUpdateActions
import to.iris.drive.app.update.buttonText


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
    isOpeningIrisApps: Boolean,
    androidCalendarSyncEnabled: Boolean,
    onCopyDeviceKey: () -> Unit,
    onCopyText: (String, String) -> Unit,
    onExportRecoverySecret: () -> RecoverySecretExport,
    onSetAndroidCalendarSync: (Boolean) -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
    onOpenIrisApps: () -> Unit,
    onOpenDriveFolder: () -> Unit,
    onAddRecoveryKey: (String) -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRejectDevice: (String) -> Unit,
    onDeleteDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
    onRenameDevice: (String, String) -> Unit,
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
    onCreateShareLink: (String, String) -> Unit,
    onDisableShareLinks: (String) -> Unit,
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
    onOpenSharePath: (String) -> Unit,
    onDeleteShare: (String) -> Unit,
    onAddShareShortcut: (String, String) -> Unit,
    onRemoveShareShortcuts: (String) -> Unit,
    onCancelShareJoin: (String) -> Unit,
    onRepairShareWraps: (String) -> Unit,
) {
    when (selectedTab) {
        MainTab.MyDrive -> DriveContent(
            padding = padding,
            state = state,
            selfUpdateState = selfUpdateState,
            selfUpdateActions = selfUpdateActions,
            isOpeningIrisApps = isOpeningIrisApps,
            onShowDevices = { onSelectTab(MainTab.Devices) },
            onCopySnapshotLink = onCopySnapshotLink,
            onOpenSnapshotLink = onOpenSnapshotLink,
            onOpenIrisApps = onOpenIrisApps,
            onOpenDriveFolder = onOpenDriveFolder,
        )
        MainTab.Devices -> DevicesContent(
            padding = padding,
            state = state,
            onCopyText = onCopyText,
            onAddRecoveryKey = onAddRecoveryKey,
            onApproveDevice = onApproveDevice,
            onRejectDevice = onRejectDevice,
            onDeleteDevice = onDeleteDevice,
            onAppointAdmin = onAppointAdmin,
            onDemoteAdmin = onDemoteAdmin,
            onRenameDevice = onRenameDevice,
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
            onCreateShareLink = onCreateShareLink,
            onDisableShareLinks = onDisableShareLinks,
            onAcceptShareInvite = onAcceptShareInvite,
            onRevokeShareMember = onRevokeShareMember,
            onOpenSharePath = onOpenSharePath,
            onDeleteShare = onDeleteShare,
            onAddShareShortcut = onAddShareShortcut,
            onRemoveShareShortcuts = onRemoveShareShortcuts,
            onCancelShareJoin = onCancelShareJoin,
            onRepairShareWraps = onRepairShareWraps,
        )
        MainTab.Settings -> SettingsContent(
            padding = padding,
            state = state,
            selfUpdateState = selfUpdateState,
            selfUpdateActions = selfUpdateActions,
            onCopyDeviceKey = onCopyDeviceKey,
            onCopyText = onCopyText,
            onExportRecoverySecret = onExportRecoverySecret,
            androidCalendarSyncEnabled = androidCalendarSyncEnabled,
            onSetAndroidCalendarSync = onSetAndroidCalendarSync,
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
    isOpeningIrisApps: Boolean,
    onShowDevices: () -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
    onOpenIrisApps: () -> Unit,
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
                localNhashResolverEnabled = state.localNhashResolverEnabled,
                isOpeningIrisApps = isOpeningIrisApps,
                onOpenDriveFolder = onOpenDriveFolder,
                onCopySnapshotLink = onCopySnapshotLink,
                onOpenSnapshotLink = onOpenSnapshotLink,
                onOpenIrisApps = onOpenIrisApps,
            )
        }
        item {
            SyncPanel(state = state)
        }
    }
}

@Composable
private fun DevicesContent(
    padding: PaddingValues,
    state: AppState,
    onCopyText: (String, String) -> Unit,
    onAddRecoveryKey: (String) -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRejectDevice: (String) -> Unit,
    onDeleteDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
    onRenameDevice: (String, String) -> Unit,
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
                deviceStorage = state.deviceStorage,
                inboundRequests = state.profile?.inboundAppKeyLinkRequests.orEmpty(),
                canApprove = state.profile?.canAdminProfile == true,
                onAddRecoveryKey = onAddRecoveryKey,
                onApproveDevice = onApproveDevice,
                onRejectDevice = onRejectDevice,
                onDeleteDevice = onDeleteDevice,
                onAppointAdmin = onAppointAdmin,
                onDemoteAdmin = onDemoteAdmin,
                onRenameDevice = onRenameDevice,
                onCopyDeviceKey = { onCopyText("Device ID", it) },
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
    onCreateShareLink: (String, String) -> Unit,
    onDisableShareLinks: (String) -> Unit,
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
    onOpenSharePath: (String) -> Unit,
    onDeleteShare: (String) -> Unit,
    onAddShareShortcut: (String, String) -> Unit,
    onRemoveShareShortcuts: (String) -> Unit,
    onCancelShareJoin: (String) -> Unit,
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
                onCreateShareLink = onCreateShareLink,
                onDisableShareLinks = onDisableShareLinks,
                onAcceptShareInvite = onAcceptShareInvite,
                onRevokeShareMember = onRevokeShareMember,
                onOpenSharePath = onOpenSharePath,
                onDeleteShare = onDeleteShare,
                onAddShareShortcut = onAddShareShortcut,
                onRemoveShareShortcuts = onRemoveShareShortcuts,
                onCancelShareJoin = onCancelShareJoin,
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
    onCopyDeviceKey: () -> Unit,
    onCopyText: (String, String) -> Unit,
    onExportRecoverySecret: () -> RecoverySecretExport,
    androidCalendarSyncEnabled: Boolean,
    onSetAndroidCalendarSync: (Boolean) -> Unit,
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
        state.profile?.profileId?.takeIf { state.isSetupComplete && !state.isRevoked }?.let { profileId ->
            item { PhotoBackupSettings(profileId) }
        }
        item {
            SettingsPanel(
                state = state,
                selfUpdateState = selfUpdateState,
                selfUpdateActions = selfUpdateActions,
                onCopyDeviceKey = onCopyDeviceKey,
                onCopyText = onCopyText,
                onExportRecoverySecret = onExportRecoverySecret,
                androidCalendarSyncEnabled = androidCalendarSyncEnabled,
                onSetAndroidCalendarSync = onSetAndroidCalendarSync,
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
) {
    // The status is said once: the card has nothing else to show.
    CardSection(title = "Sync", trailing = "") {
        StatRow("State", state.sync.statusLabel)
    }
}

@Composable
private fun ProviderPanel(
    snapshotLink: String,
    localNhashResolverEnabled: Boolean,
    isOpeningIrisApps: Boolean,
    onOpenDriveFolder: () -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
    onOpenIrisApps: () -> Unit,
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
        OutlinedButton(
            onClick = onOpenIrisApps,
            enabled = localNhashResolverEnabled && !isOpeningIrisApps,
        ) {
            Text(if (isOpeningIrisApps) "Opening Iris Apps" else "Open Iris Apps")
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
    var adding by remember { mutableStateOf(false) }

    CardSection(title = "Backups of My Drive", trailing = "${backups.size}") {
        Text(
            "Encrypted copies of My Drive kept on a file server, with a friend, or in a folder.",
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
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
        if (backups.isNotEmpty()) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { onSyncBackups("") }) {
                    Text("Sync Now")
                }
                OutlinedButton(
                    onClick = { onCheckBackups("") },
                    enabled = !backupCheckProgress.isRunning,
                ) {
                    Text(if (backupCheckProgress.isRunning) backupCheckProgress.label else "Check All")
                }
            }
        }
        if (backupCheckProgress.isRunning) {
            BackupProgressIndicator(backupCheckProgress)
        }
        if (!adding) {
            TextButton(onClick = { adding = true }) {
                Text("Add Backup")
            }
        } else {
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
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = {
                        onAddBackupTarget(backupInput, backupLabel)
                        backupInput = ""
                        backupLabel = ""
                        adding = false
                    },
                    enabled = backupInput.isNotBlank(),
                ) {
                    Text("Add")
                }
                TextButton(onClick = { adding = false }) {
                    Text("Cancel")
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
    onCreateShareLink: (String, String) -> Unit,
    onDisableShareLinks: (String) -> Unit,
    onAcceptShareInvite: (String) -> Unit,
    onRevokeShareMember: (String, String) -> Unit,
    onOpenSharePath: (String) -> Unit,
    onDeleteShare: (String) -> Unit,
    onAddShareShortcut: (String, String) -> Unit,
    onRemoveShareShortcuts: (String) -> Unit,
    onCancelShareJoin: (String) -> Unit,
    onRepairShareWraps: (String) -> Unit,
) {
    var sourceInput by remember { mutableStateOf("") }
    var inviteInput by remember { mutableStateOf("") }
    var inviteTarget by remember { mutableStateOf<ShareState?>(null) }
    var revokeTarget by remember { mutableStateOf<Pair<ShareState, ShareMemberState>?>(null) }
    var deleteTarget by remember { mutableStateOf<ShareState?>(null) }

    LaunchedEffect(shareDialogRequest?.id) {
        val request = shareDialogRequest ?: return@LaunchedEffect
        if (request.joinLink.isNotBlank()) inviteInput = request.joinLink else sourceInput = request.sourcePath
    }

    inviteTarget?.let { share ->
        InviteShareMemberDialog(
            share = share,
            link = state.lastShareInvite,
            onRole = { role -> onCreateShareLink(share.shareId, role) },
            onCopy = { onCopyText("Link", state.lastShareInvite) },
            onDisable = {
                onDisableShareLinks(share.shareId)
                inviteTarget = null
            },
            onDismiss = { inviteTarget = null },
        )
    }

    revokeTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { revokeTarget = null },
            title = { Text("Remove access?") },
            text = {
                Text(
                    "${displayMemberName(target.second)} will no longer get changes to “${displayShareName(target.first)}”.",
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
                    Text("Remove", color = Danger)
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
            title = { Text(if (share.owned) "Stop sharing?" else "Leave shared folder?") },
            text = {
                Text(
                    if (share.owned) {
                        "“${displayShareName(share)}” stays in your My Drive. People you shared it with keep what they have and get no further changes."
                    } else {
                        "“${displayShareName(share)}” is removed from this device. You can join again with a link."
                    },
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
                    Text(if (share.owned) "Stop sharing" else "Leave", color = Danger)
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
            label = { Text("Folder in My Drive, e.g. Photos/Trip") },
            singleLine = true,
            modifier = Modifier
                .fillMaxWidth()
                .testTag("shareSourceInput"),
        )
        if (state.shareableFolders.isNotEmpty()) {
            var choosing by remember { mutableStateOf(false) }
            Box {
                OutlinedButton(onClick = { choosing = true }, modifier = Modifier.testTag("shareFolderMenu")) {
                    Text("Choose folder")
                }
                DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
                    state.shareableFolders.take(200).forEach { folder ->
                        DropdownMenuItem(
                            text = { Text(folder) },
                            onClick = {
                                sourceInput = folder
                                choosing = false
                            },
                        )
                    }
                }
            }
        }
        Button(
            onClick = {
                onCreateShare(sourceInput, "")
                sourceInput = ""
            },
            enabled = sourceInput.isNotBlank(),
        ) {
            Text("Create shared folder")
        }

        Text("Join Shared Folder", fontWeight = FontWeight.SemiBold)
        state.pendingShareJoins.forEach { join ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.weight(1f).testTag("pendingShareJoin")) {
                    Text("Waiting to join “${join.displayName}”", color = Ink)
                    Text("You get access when someone who manages it has Iris Drive open.", color = Muted, style = MaterialTheme.typography.bodySmall)
                }
                TextButton(onClick = { onCancelShareJoin(join.shareId) }) { Text("Cancel") }
            }
        }
        OutlinedTextField(
            value = inviteInput,
            onValueChange = { inviteInput = it },
            label = { Text("Paste a share link") },
            singleLine = true,
            modifier = Modifier
                .fillMaxWidth()
                .testTag("shareInviteInput"),
        )
        OutlinedButton(
            onClick = {
                onAcceptShareInvite(inviteInput)
                inviteInput = ""
            },
            enabled = inviteInput.isNotBlank(),
            modifier = Modifier.testTag("acceptShareInviteButton"),
        ) {
            Text("Join")
        }

        if (state.shares.isEmpty()) {
            Text("No shared folders", color = Muted)
        }
        state.shares.forEach { share ->
            ShareItem(
                share = share,
                localProfileId = state.profile?.profileId.orEmpty(),
                onOpen = { onOpenSharePath(share.location) },
                onInvite = { inviteTarget = share },
                onRepair = { onRepairShareWraps(share.shareId) },
                onShortcut = { onAddShareShortcut(share.shareId, displayShareName(share)) },
                onRemoveShortcut = { onRemoveShareShortcuts(share.shareId) },
                onDelete = { deleteTarget = share },
                onRevoke = { member -> revokeTarget = share to member },
            )
        }
    }
}

/** Share by link: anyone who opens it can ask to join; no app needed to open it. */
@Composable
private fun InviteShareMemberDialog(
    share: ShareState,
    link: String,
    onRole: (String) -> Unit,
    onCopy: () -> Unit,
    onDisable: () -> Unit,
    onDismiss: () -> Unit,
) {
    var role by remember(share.shareId) { mutableStateOf("reader") }
    val context = LocalContext.current
    LaunchedEffect(share.shareId, role) { onRole(role) }
    val ready = link.contains("/share-join/")
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Share “${displayShareName(share)}”") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("People with the link can", color = Muted)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("reader" to "View", "editor" to "Edit").forEach { (value, label) ->
                        if (role == value) {
                            Button(onClick = {}, modifier = Modifier.testTag("shareLinkRole_$value")) { Text(label) }
                        } else {
                            OutlinedButton(onClick = { role = value }, modifier = Modifier.testTag("shareLinkRole_$value")) { Text(label) }
                        }
                    }
                }
                Text(
                    "Anyone with this link can join. Iris Drive lets them in when it syncs. They don't need the app — the link opens in a browser.",
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        onClick = {
                            val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, link)
                            context.startActivity(Intent.createChooser(send, "Share link"))
                        },
                        enabled = ready,
                        modifier = Modifier.testTag("shareLinkSend"),
                    ) { Text("Send link") }
                    OutlinedButton(onClick = onCopy, enabled = ready, modifier = Modifier.testTag("shareLinkCopy")) {
                        Text("Copy link")
                    }
                }
                TextButton(onClick = onDisable) { Text("Turn off links", color = Danger) }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Done") } },
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
    onRemoveShortcut: () -> Unit,
    onDelete: () -> Unit,
    onRevoke: (ShareMemberState) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(displayShareName(share), fontWeight = FontWeight.SemiBold)
        Text(
            listOf(
                share.roleLabel.ifBlank { share.role },
                if (share.participantCount == 1) "1 person" else "${share.participantCount} people",
                if (share.location.isBlank()) "Not in My Drive" else "My Drive/${shortText(share.location)}",
            ).joinToString(" · "),
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (share.location.isBlank()) {
                TextButton(onClick = onShortcut, modifier = Modifier.testTag("addShareToMyDrive")) {
                    Text("Add to My Drive")
                }
            } else {
                TextButton(onClick = onOpen) {
                    Text("Open")
                }
            }
            if (share.canAdmin) {
                TextButton(onClick = onInvite) {
                    Text("Share")
                }
            }
            if (share.repairNeeded || share.missingKeyWraps.isNotEmpty()) {
                TextButton(onClick = onRepair) {
                    Text("Repair")
                }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (!share.owned && share.location.isNotBlank()) {
                TextButton(onClick = onRemoveShortcut, modifier = Modifier.testTag("removeShareFromMyDrive")) {
                    Text("Remove from My Drive")
                }
            }
            TextButton(onClick = onDelete) {
                Text(if (share.owned) "Stop sharing" else "Leave", color = Danger)
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
                Text("Remove", color = Danger)
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
    onCopyDeviceKey: () -> Unit,
    onCopyText: (String, String) -> Unit,
    onExportRecoverySecret: () -> RecoverySecretExport,
    androidCalendarSyncEnabled: Boolean,
    onSetAndroidCalendarSync: (Boolean) -> Unit,
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
        Text("Current Device ID", fontWeight = FontWeight.SemiBold)
        Text(profile?.devicePubkey.orEmpty(), color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedButton(onClick = onCopyDeviceKey) {
                Text("Copy Device ID")
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
        Text("Android Calendar", fontWeight = FontWeight.SemiBold)
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier
                .fillMaxWidth()
                .testTag("androidCalendarSyncRow"),
        ) {
            Text("Keep Android Calendar in sync", modifier = Modifier.weight(1f))
            Switch(
                checked = androidCalendarSyncEnabled,
                onCheckedChange = onSetAndroidCalendarSync,
                modifier = Modifier.testTag("androidCalendarSyncToggle"),
            )
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
    member.displayName.ifBlank { "NostrIdentity" }

private fun displayPendingInviteName(invite: PendingShareInviteState): String =
    invite.displayName.ifBlank { "Pending contact" }

private fun shortText(value: String): String {
    if (value.length <= 32) return value
    return "${value.take(14)}...${value.takeLast(10)}"
}
