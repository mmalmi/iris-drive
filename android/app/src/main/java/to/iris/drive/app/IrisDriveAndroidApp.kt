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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.StateFlow
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.BackupState
import to.iris.drive.app.core.DeviceState
import to.iris.drive.app.core.SyncRoot

private const val DocumentsProviderAuthority = "to.iris.drive.documents"
private const val ProviderRoot = "content://to.iris.drive.documents/document/root"

private val Background = Color(0xFFF7FAF8)
private val Ink = Color(0xFF172321)
private val Muted = Color(0xFF657370)
private val Teal = Color(0xFF167C80)
private val SoftTeal = Color(0xFFE7F4F0)
private val Amber = Color(0xFFF5A524)
private val Danger = Color(0xFFB42318)

@Composable
internal fun IrisDriveAndroidApp(
    stateFlow: StateFlow<AppState>,
    onRefresh: () -> Unit,
    onCreateProfile: (String) -> Unit,
    onRestoreProfile: (String, String) -> Unit,
    onLinkDevice: (String, String) -> Unit,
    onCopyText: (String, String) -> Unit,
    onOpenUrl: (String) -> Unit,
    onOpenDriveFolder: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRevokeDevice: (String) -> Unit,
    onAddRelay: (String) -> Unit,
    onRemoveRelay: (String) -> Unit,
    onResetRelays: () -> Unit,
    onAddRoot: (String, String) -> Unit,
    onRemoveRoot: (String) -> Unit,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
    onRestartSync: () -> Unit,
) {
    val state by stateFlow.collectAsState()
    var addRootOpen by remember { mutableStateOf(false) }
    val account = state.account

    IrisDriveTheme {
        Scaffold(
            containerColor = Background,
            topBar = {
                AppTopBar(
                    onRefresh = onRefresh,
                    onAddRoot = { addRootOpen = true },
                )
            },
        ) { padding ->
            if (account == null) {
                SetupContent(
                    padding = padding,
                    error = state.error,
                    onCreateProfile = { label ->
                        onCreateProfile(label)
                        onAddRoot("My Drive", ProviderRoot)
                    },
                    onRestoreProfile = { secret, label ->
                        onRestoreProfile(secret, label)
                        onAddRoot("My Drive", ProviderRoot)
                    },
                    onLinkDevice = { owner, label ->
                        onLinkDevice(owner, label)
                        onAddRoot("My Drive", ProviderRoot)
                    },
                )
            } else {
                DriveContent(
                    padding = padding,
                    state = state,
                    onStartSync = onStartSync,
                    onStopSync = onStopSync,
                    onRestartSync = onRestartSync,
                    onCopyOwnerKey = { onCopyText("Owner key", account.ownerPubkey) },
                    onCopyDeviceKey = { onCopyText("Device key", account.devicePubkey) },
                    onCopyLinkRequest = { onCopyText("Link request", account.deviceLinkRequest) },
                    onCopySnapshotLink = { onCopyText("Snapshot link", state.snapshotLink) },
                    onOpenSnapshotLink = { onOpenUrl(state.snapshotLink) },
                    onOpenDriveFolder = onOpenDriveFolder,
                    onApproveDevice = onApproveDevice,
                    onRevokeDevice = onRevokeDevice,
                    onAddRelay = onAddRelay,
                    onRemoveRelay = onRemoveRelay,
                    onResetRelays = onResetRelays,
                    onRemoveRoot = onRemoveRoot,
                    onAddRoot = { addRootOpen = true },
                )
            }
        }
        if (addRootOpen) {
            AddRootDialog(
                onDismiss = { addRootOpen = false },
                onAdd = { name, path ->
                    addRootOpen = false
                    onAddRoot(name, path)
                },
            )
        }
    }
}

@Composable
private fun IrisDriveTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = lightColorScheme(
            primary = Teal,
            secondary = Amber,
            background = Background,
            surface = Color.White,
            error = Danger,
            onPrimary = Color.White,
            onSecondary = Ink,
            onBackground = Ink,
            onSurface = Ink,
        ),
        content = content,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppTopBar(onRefresh: () -> Unit, onAddRoot: () -> Unit) {
    TopAppBar(
        title = {
            Column {
                Text("Iris Drive", fontWeight = FontWeight.SemiBold)
                Text("Android", color = Muted, style = MaterialTheme.typography.labelMedium)
            }
        },
        actions = {
            IconButton(onClick = onRefresh) {
                Icon(painterResource(R.drawable.ic_refresh), contentDescription = "Refresh")
            }
            FilledIconButton(onClick = onAddRoot) {
                Icon(painterResource(R.drawable.ic_add), contentDescription = "Add root")
            }
        },
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = Color.White,
            titleContentColor = Ink,
            actionIconContentColor = Teal,
        ),
    )
}

@Composable
private fun SetupContent(
    padding: PaddingValues,
    error: String,
    onCreateProfile: (String) -> Unit,
    onRestoreProfile: (String, String) -> Unit,
    onLinkDevice: (String, String) -> Unit,
) {
    var deviceLabel by remember { mutableStateOf("Android device") }
    var restoreSecret by remember { mutableStateOf("") }
    var linkOwner by remember { mutableStateOf("") }

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding),
        contentPadding = PaddingValues(18.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (error.isNotBlank()) {
            item { Notice(error) }
        }
        item {
            CardSection(title = "Iris Drive", trailing = "setup") {
                Image(
                    painter = painterResource(id = R.drawable.brand_icon),
                    contentDescription = "Iris Drive",
                    modifier = Modifier
                        .align(Alignment.CenterHorizontally)
                        .size(96.dp),
                )
                OutlinedTextField(
                    value = deviceLabel,
                    onValueChange = { deviceLabel = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Device label") },
                )
            }
        }
        item {
            CardSection(title = "Create Profile", trailing = "owner") {
                Button(onClick = { onCreateProfile(deviceLabel) }) {
                    Icon(painterResource(R.drawable.ic_add), contentDescription = null)
                    Spacer(Modifier.size(8.dp))
                    Text("Create profile")
                }
            }
        }
        item {
            CardSection(title = "Sign In", trailing = "restore") {
                OutlinedTextField(
                    value = restoreSecret,
                    onValueChange = { restoreSecret = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Secret key") },
                )
                Button(
                    onClick = { onRestoreProfile(restoreSecret, deviceLabel) },
                    enabled = restoreSecret.isNotBlank(),
                ) {
                    Text("Sign in")
                }
            }
        }
        item {
            CardSection(title = "Link Device", trailing = "request") {
                OutlinedTextField(
                    value = linkOwner,
                    onValueChange = { linkOwner = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Owner public key") },
                )
                OutlinedButton(
                    onClick = { onLinkDevice(linkOwner, deviceLabel) },
                    enabled = linkOwner.isNotBlank(),
                ) {
                    Text("Link this device")
                }
            }
        }
    }
}

@Composable
private fun DriveContent(
    padding: PaddingValues,
    state: AppState,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
    onRestartSync: () -> Unit,
    onCopyOwnerKey: () -> Unit,
    onCopyDeviceKey: () -> Unit,
    onCopyLinkRequest: () -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
    onOpenDriveFolder: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRevokeDevice: (String) -> Unit,
    onAddRelay: (String) -> Unit,
    onRemoveRelay: (String) -> Unit,
    onResetRelays: () -> Unit,
    onRemoveRoot: (String) -> Unit,
    onAddRoot: () -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding),
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
            SyncPanel(
                isRunning = state.sync.running,
                onStartSync = onStartSync,
                onStopSync = onStopSync,
                onRestartSync = onRestartSync,
            )
        }
        item {
            ProviderPanel(
                snapshotLink = state.snapshotLink.ifBlank { "https://drive.iris.to/snapshot/local" },
                onOpenDriveFolder = onOpenDriveFolder,
                onCopySnapshotLink = onCopySnapshotLink,
                onOpenSnapshotLink = onOpenSnapshotLink,
            )
        }
        item {
            DevicesPanel(
                devices = state.devices,
                canApprove = state.account?.hasOwnerSigningAuthority == true,
                onApproveDevice = onApproveDevice,
                onRevokeDevice = onRevokeDevice,
            )
        }
        item {
            BackupsPanel(backups = state.backups)
        }
        item {
            SettingsPanel(
                state = state,
                onCopyOwnerKey = onCopyOwnerKey,
                onCopyDeviceKey = onCopyDeviceKey,
                onCopyLinkRequest = onCopyLinkRequest,
                onAddRelay = onAddRelay,
                onRemoveRelay = onRemoveRelay,
                onResetRelays = onResetRelays,
            )
        }
        item {
            SectionHeader("Roots", "${state.roots.size}")
        }
        if (state.roots.isEmpty()) {
            item { EmptyRoots(onAddRoot = onAddRoot) }
        } else {
            items(state.roots, key = { it.name }) { root ->
                RootRow(root = root, onRemoveRoot = onRemoveRoot)
            }
        }
    }
}

@Composable
private fun StatusPanel(state: AppState) {
    val account = state.account
    CardSection(title = "My Drive", trailing = state.sync.status.ifBlank { "paused" }) {
        Text(account?.deviceLabel ?: "This device", fontWeight = FontWeight.SemiBold)
        Text(account?.authorizationState ?: "not linked", color = Muted)
        Text(
            if (state.sync.running) "Foreground sync is active" else "Foreground sync is paused",
            color = if (state.sync.running) Teal else Muted,
        )
    }
}

@Composable
private fun SyncPanel(
    isRunning: Boolean,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
    onRestartSync: () -> Unit,
) {
    CardSection(title = "Sync", trailing = if (isRunning) "running" else "paused") {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Button(onClick = onStartSync) {
                Icon(painterResource(R.drawable.ic_play), contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Start")
            }
            OutlinedButton(onClick = onStopSync) {
                Icon(painterResource(R.drawable.ic_stop), contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Stop")
            }
            OutlinedButton(onClick = onRestartSync) {
                Icon(painterResource(R.drawable.ic_refresh), contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Restart")
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
    CardSection(title = "Files", trailing = "DocumentsProvider") {
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
                    Text(DocumentsProviderAuthority, color = Muted, style = MaterialTheme.typography.bodySmall)
                    Text(snapshotLink, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        Button(onClick = onOpenDriveFolder) {
            Icon(painterResource(R.drawable.ic_drive), contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Open drive")
        }
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedButton(onClick = onCopySnapshotLink) {
                Text("Copy snapshot link")
            }
            OutlinedButton(onClick = onOpenSnapshotLink) {
                Text("Open snapshot link")
            }
        }
    }
}

@Composable
private fun DevicesPanel(
    devices: List<DeviceState>,
    canApprove: Boolean,
    onApproveDevice: (String, String) -> Unit,
    onRevokeDevice: (String) -> Unit,
) {
    var request by remember { mutableStateOf("") }
    var label by remember { mutableStateOf("") }

    CardSection(title = "Devices", trailing = "${devices.size}") {
        devices.forEach { device ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Icon(
                    painterResource(R.drawable.ic_drive),
                    contentDescription = null,
                    tint = if (device.isOnline) Teal else Muted,
                )
                Spacer(Modifier.size(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(device.label, fontWeight = FontWeight.SemiBold)
                    Text(device.state, color = Muted, style = MaterialTheme.typography.bodySmall)
                    Text(device.detail, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (device.canRevoke) {
                    IconButton(onClick = { onRevokeDevice(device.pubkey) }) {
                        Icon(
                            painterResource(R.drawable.ic_delete),
                            contentDescription = "Revoke ${device.label}",
                            tint = Danger,
                        )
                    }
                }
            }
        }
        OutlinedTextField(
            value = request,
            onValueChange = { request = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("Device request") },
        )
        OutlinedTextField(
            value = label,
            onValueChange = { label = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("Label") },
        )
        Button(
            onClick = {
                onApproveDevice(request, label)
                request = ""
                label = ""
            },
            enabled = canApprove && request.isNotBlank(),
        ) {
            Text("Approve Device")
        }
    }
}

@Composable
private fun BackupsPanel(backups: List<BackupState>) {
    CardSection(title = "Backups", trailing = "${backups.size}") {
        if (backups.isEmpty()) {
            Text("No fallback servers configured", color = Muted)
        }
        backups.forEach { backup ->
            Text(backup.label, fontWeight = FontWeight.SemiBold)
            Text(backup.state, color = Muted, style = MaterialTheme.typography.bodySmall)
            Text(backup.detail, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun SettingsPanel(
    state: AppState,
    onCopyOwnerKey: () -> Unit,
    onCopyDeviceKey: () -> Unit,
    onCopyLinkRequest: () -> Unit,
    onAddRelay: (String) -> Unit,
    onRemoveRelay: (String) -> Unit,
    onResetRelays: () -> Unit,
) {
    var relayInput by remember { mutableStateOf("") }
    val account = state.account

    CardSection(title = "Settings", trailing = "network") {
        Text("Relays", fontWeight = FontWeight.SemiBold)
        state.relays.forEach { relay ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Text(relay, color = Muted, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                IconButton(onClick = { onRemoveRelay(relay) }) {
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
        Text("Owner key", fontWeight = FontWeight.SemiBold)
        Text(account?.ownerPubkey.orEmpty(), color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text("Device key", fontWeight = FontWeight.SemiBold)
        Text(account?.devicePubkey.orEmpty(), color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedButton(onClick = onCopyOwnerKey) {
                Text("Copy owner key")
            }
            OutlinedButton(onClick = onCopyDeviceKey) {
                Text("Copy device key")
            }
        }
        OutlinedButton(onClick = onCopyLinkRequest) {
            Text("Copy link request")
        }
        Text("Data path", fontWeight = FontWeight.SemiBold)
        Text(state.paths.dataDir, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(state.paths.configPath, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(state.paths.blocksDir, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun RootRow(root: SyncRoot, onRemoveRoot: (String) -> Unit) {
    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = Color.White),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(painterResource(R.drawable.ic_drive), contentDescription = null, tint = Teal)
            Spacer(Modifier.size(12.dp))
            Column(Modifier.weight(1f)) {
                Text(root.name, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(root.status, color = Muted, style = MaterialTheme.typography.bodySmall)
                Text(root.localPath, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            IconButton(onClick = { onRemoveRoot(root.name) }) {
                Icon(
                    painterResource(R.drawable.ic_delete),
                    contentDescription = "Remove ${root.name}",
                    tint = Danger,
                )
            }
        }
    }
}

@Composable
private fun EmptyRoots(onAddRoot: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 18.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Image(
            painter = painterResource(id = R.drawable.brand_icon),
            contentDescription = "Iris Drive",
            modifier = Modifier.size(96.dp),
        )
        Text("No roots", color = Muted)
        OutlinedButton(onClick = onAddRoot) {
            Icon(painterResource(R.drawable.ic_add), contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Add")
        }
    }
}

@Composable
private fun Notice(text: String) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(Color(0xFFFEE4E2), RoundedCornerShape(8.dp))
            .padding(12.dp),
    ) {
        Text(text, color = Danger)
    }
}

@Composable
private fun CardSection(
    title: String,
    trailing: String,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(shape = RoundedCornerShape(8.dp), colors = CardDefaults.cardColors(Color.White)) {
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
private fun AddRootDialog(
    onDismiss: () -> Unit,
    onAdd: (String, String) -> Unit,
) {
    var name by remember { mutableStateOf("My Drive") }
    var path by remember { mutableStateOf(ProviderRoot) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Root") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Name") },
                )
                OutlinedTextField(
                    value = path,
                    onValueChange = { path = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Path") },
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { onAdd(name, path) }) {
                Text("Add")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}
