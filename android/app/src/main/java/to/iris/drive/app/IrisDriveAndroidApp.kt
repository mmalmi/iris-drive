package to.iris.drive.app

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.material3.darkColorScheme
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

private val IrisLightBackground = Color(0xFFF7FAF8)
private val IrisLightSurface = Color.White
private val IrisLightInk = Color(0xFF172321)
private val IrisLightMuted = Color(0xFF657370)
private val IrisLightSoftTeal = Color(0xFFE7F4F0)
private val IrisDarkBackground = Color(0xFF101815)
private val IrisDarkSurface = Color(0xFF18231F)
private val IrisDarkInk = Color(0xFFE7F0EC)
private val IrisDarkMuted = Color(0xFFA9B8B3)
private val IrisDarkSoftTeal = Color(0xFF143A3C)
private val IrisTeal = Color(0xFF167C80)
private val IrisAmber = Color(0xFFF5A524)
private val IrisDanger = Color(0xFFB42318)
private val IrisDarkDanger = Color(0xFFFFB4AB)
private val IrisErrorContainer = Color(0xFFFEE4E2)
private val IrisDarkErrorContainer = Color(0xFF5F1815)

private val Background: Color
    @Composable get() = MaterialTheme.colorScheme.background

private val Ink: Color
    @Composable get() = MaterialTheme.colorScheme.onSurface

private val Muted: Color
    @Composable get() = MaterialTheme.colorScheme.onSurfaceVariant

private val Teal: Color
    @Composable get() = MaterialTheme.colorScheme.primary

private val SoftTeal: Color
    @Composable get() = MaterialTheme.colorScheme.primaryContainer

private val Danger: Color
    @Composable get() = MaterialTheme.colorScheme.error

private enum class SetupRoute {
    Welcome,
    CreateProfile,
    CreatePhoto,
    SignIn,
    LinkDevice,
}

@Composable
internal fun IrisDriveAndroidApp(
    stateFlow: StateFlow<AppState>,
    onCreateProfile: (String) -> Unit,
    onRestoreProfile: (String, String) -> Unit,
    onLinkDevice: (String, String) -> Unit,
    onCopyText: (String, String) -> Unit,
    onOpenUrl: (String) -> Unit,
    onOpenDriveFolder: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRevokeDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
    onAddRelay: (String) -> Unit,
    onRemoveRelay: (String) -> Unit,
    onResetRelays: () -> Unit,
    onAddRoot: (String, String) -> Unit,
    onRemoveRoot: (String) -> Unit,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
) {
    val state by stateFlow.collectAsState()
    var addRootOpen by remember { mutableStateOf(false) }
    val account = state.account

    IrisDriveTheme {
        Scaffold(
            containerColor = Background,
            topBar = {
                if (account != null) {
                    AppTopBar(onAddRoot = { addRootOpen = true })
                }
            },
        ) { padding ->
            if (account == null) {
                SetupContent(
                    padding = padding,
                    error = state.error,
                    onCreateProfile = {
                        onCreateProfile("")
                        onAddRoot("My Drive", ProviderRoot)
                    },
                    onRestoreProfile = { secret ->
                        onRestoreProfile(secret, "")
                        onAddRoot("My Drive", ProviderRoot)
                    },
                    onLinkDevice = { owner ->
                        onLinkDevice(owner, "")
                        onAddRoot("My Drive", ProviderRoot)
                    },
                )
            } else {
                DriveContent(
                    padding = padding,
                    state = state,
                    onStartSync = onStartSync,
                    onStopSync = onStopSync,
                    onCopyOwnerKey = { onCopyText("Owner key", account.ownerPubkey) },
                    onCopyDeviceKey = { onCopyText("Device key", account.devicePubkey) },
                    onCopyLinkRequest = { onCopyText("Link request", account.deviceLinkRequest) },
                    onCopySnapshotLink = { onCopyText("Snapshot link", state.snapshotLink) },
                    onOpenSnapshotLink = { onOpenUrl(state.snapshotLink) },
                    onOpenDriveFolder = onOpenDriveFolder,
                    onApproveDevice = onApproveDevice,
                    onRevokeDevice = onRevokeDevice,
                    onAppointAdmin = onAppointAdmin,
                    onDemoteAdmin = onDemoteAdmin,
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
    val darkTheme = isSystemInDarkTheme()

    MaterialTheme(
        colorScheme = irisDriveColorScheme(darkTheme = darkTheme),
        content = content,
    )
}

internal fun irisDriveColorScheme(darkTheme: Boolean) = if (darkTheme) {
    darkColorScheme(
        primary = IrisTeal,
        secondary = IrisAmber,
        background = IrisDarkBackground,
        surface = IrisDarkSurface,
        primaryContainer = IrisDarkSoftTeal,
        error = IrisDarkDanger,
        errorContainer = IrisDarkErrorContainer,
        onPrimary = Color.White,
        onSecondary = Color(0xFF211600),
        onBackground = IrisDarkInk,
        onSurface = IrisDarkInk,
        onSurfaceVariant = IrisDarkMuted,
        onPrimaryContainer = IrisDarkInk,
        onErrorContainer = IrisDarkDanger,
    )
} else {
    lightColorScheme(
        primary = IrisTeal,
        secondary = IrisAmber,
        background = IrisLightBackground,
        surface = IrisLightSurface,
        primaryContainer = IrisLightSoftTeal,
        error = IrisDanger,
        errorContainer = IrisErrorContainer,
        onPrimary = Color.White,
        onSecondary = IrisLightInk,
        onBackground = IrisLightInk,
        onSurface = IrisLightInk,
        onSurfaceVariant = IrisLightMuted,
        onPrimaryContainer = IrisLightInk,
        onErrorContainer = IrisDanger,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppTopBar(onAddRoot: () -> Unit) {
    TopAppBar(
        title = {
            Column {
                Text("Iris Drive", fontWeight = FontWeight.SemiBold)
                Text("Android", color = Muted, style = MaterialTheme.typography.labelMedium)
            }
        },
        actions = {
            FilledIconButton(onClick = onAddRoot) {
                Icon(painterResource(R.drawable.ic_add), contentDescription = "Add root")
            }
        },
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = MaterialTheme.colorScheme.surface,
            titleContentColor = Ink,
            actionIconContentColor = Teal,
        ),
    )
}

@Composable
private fun SetupContent(
    padding: PaddingValues,
    error: String,
    onCreateProfile: () -> Unit,
    onRestoreProfile: (String) -> Unit,
    onLinkDevice: (String) -> Unit,
) {
    var createUsername by remember { mutableStateOf("") }
    var selectedPhoto by remember { mutableStateOf("") }
    var restoreSecret by remember { mutableStateOf("") }
    var linkOwner by remember { mutableStateOf("") }
    var route by remember { mutableStateOf(SetupRoute.Welcome) }
    val photoPicker = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        selectedPhoto = uri?.lastPathSegment.orEmpty()
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .padding(32.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .widthIn(max = 340.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            if (route == SetupRoute.Welcome) {
                SetupBrand()
            }
            if (error.isNotBlank()) {
                Notice(error)
            }
            when (route) {
                SetupRoute.Welcome -> {
                    SetupPrimaryButton(
                        text = "Create profile",
                        onClick = { route = SetupRoute.CreateProfile },
                        icon = true,
                    )
                    SetupSecondaryButton(
                        text = "Sign in",
                        onClick = { route = SetupRoute.SignIn },
                    )
                }
                SetupRoute.CreateProfile -> {
                    SetupFormHeader(title = "Create profile", onBack = { route = SetupRoute.Welcome })
                    OutlinedTextField(
                        value = createUsername,
                        onValueChange = { createUsername = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text("Username (optional)") },
                    )
                    SetupPrimaryButton(
                        text = if (createUsername.isBlank()) "Create profile" else "Continue",
                        onClick = {
                            if (createUsername.isBlank()) {
                                onCreateProfile()
                            } else {
                                route = SetupRoute.CreatePhoto
                            }
                        },
                        icon = true,
                    )
                }
                SetupRoute.CreatePhoto -> {
                    SetupFormHeader(title = "Profile photo", onBack = { route = SetupRoute.CreateProfile })
                    SetupSecondaryButton(
                        text = if (selectedPhoto.isBlank()) "Choose photo" else "Photo selected",
                        onClick = { photoPicker.launch("image/*") },
                    )
                    if (selectedPhoto.isNotBlank()) {
                        Text(selectedPhoto, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        SetupSecondaryButton(
                            text = "Remove photo",
                            onClick = { selectedPhoto = "" },
                        )
                    }
                    SetupPrimaryButton(
                        text = if (selectedPhoto.isBlank()) "Later" else "Create profile",
                        onClick = { onCreateProfile() },
                        icon = true,
                    )
                }
                SetupRoute.SignIn -> {
                    SetupFormHeader(title = "Sign in", onBack = { route = SetupRoute.Welcome })
                    OutlinedTextField(
                        value = restoreSecret,
                        onValueChange = { restoreSecret = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text("Secret key") },
                    )
                    SetupPrimaryButton(
                        text = "Sign in",
                        onClick = { onRestoreProfile(restoreSecret) },
                        enabled = restoreSecret.isNotBlank(),
                    )
                    SetupSecondaryButton(
                        text = "Link this device",
                        onClick = { route = SetupRoute.LinkDevice },
                    )
                }
                SetupRoute.LinkDevice -> {
                    SetupFormHeader(title = "Link this device", onBack = { route = SetupRoute.Welcome })
                    OutlinedTextField(
                        value = linkOwner,
                        onValueChange = { linkOwner = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text("Owner public key") },
                    )
                    SetupPrimaryButton(
                        text = "Link device",
                        onClick = { onLinkDevice(linkOwner) },
                        enabled = linkOwner.isNotBlank(),
                    )
                }
            }
        }
    }
}

@Composable
private fun SetupBrand() {
    Image(
        painter = painterResource(id = R.drawable.brand_icon),
        contentDescription = "Iris Drive",
        modifier = Modifier.size(96.dp),
    )
    Text("Iris Drive", color = Ink, fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.headlineMedium)
    Spacer(Modifier.height(10.dp))
}

@Composable
private fun SetupFormHeader(title: String, onBack: () -> Unit) {
    Column(modifier = Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        TextButton(onClick = onBack) {
            Text("Back")
        }
        Text(title, color = Ink, fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.headlineSmall)
    }
}

@Composable
private fun SetupPrimaryButton(
    text: String,
    onClick: () -> Unit,
    enabled: Boolean = true,
    icon: Boolean = false,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier
            .fillMaxWidth()
            .height(48.dp),
        shape = RoundedCornerShape(6.dp),
    ) {
        if (icon) {
            Icon(painterResource(R.drawable.ic_add), contentDescription = null)
            Spacer(Modifier.size(8.dp))
        }
        Text(text)
    }
}

@Composable
private fun SetupSecondaryButton(text: String, onClick: () -> Unit) {
    OutlinedButton(
        onClick = onClick,
        modifier = Modifier
            .fillMaxWidth()
            .height(48.dp),
        shape = RoundedCornerShape(6.dp),
    ) {
        Text(text)
    }
}

@Composable
private fun DriveContent(
    padding: PaddingValues,
    state: AppState,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
    onCopyOwnerKey: () -> Unit,
    onCopyDeviceKey: () -> Unit,
    onCopyLinkRequest: () -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
    onOpenDriveFolder: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRevokeDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
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
            SummaryPanel(state = state)
        }
        item {
            ProviderPanel(
                state = state,
                snapshotLink = state.snapshotLink.ifBlank { "https://drive.iris.to/snapshot/local" },
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
        item {
            DevicesPanel(
                devices = state.devices,
                canApprove = state.account?.hasOwnerSigningAuthority == true,
                onApproveDevice = onApproveDevice,
                onRevokeDevice = onRevokeDevice,
                onAppointAdmin = onAppointAdmin,
                onDemoteAdmin = onDemoteAdmin,
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
        item { RootsPanel(roots = state.roots, onAddRoot = onAddRoot, onRemoveRoot = onRemoveRoot) }
    }
}

@Composable
private fun StatusPanel(state: AppState) {
    val account = state.account
    val statusText = if (state.sync.running) "Up to date" else "Paused"
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
                Text(statusText, color = if (state.sync.running) Teal else Muted, fontWeight = FontWeight.SemiBold)
                Text(account?.authorizationState ?: "not linked", color = Muted)
            }
        }
        Text(
            "${state.fileCount} files - ${byteString(state.visibleFileBytes)} - ${state.authorizedDeviceCount} devices",
            color = Muted,
        )
    }
}

@Composable
private fun SummaryPanel(state: AppState) {
    CardSection(title = "Summary", trailing = "${state.fileCount} files") {
        StatRow("Files", state.fileCount.toString())
        StatRow("Top level", state.topLevelEntries.toString())
        StatRow("Storage", byteString(state.visibleFileBytes))
        StatRow("Authorized devices", state.authorizedDeviceCount.toString())
        StatRow("Published roots", state.publishedDeviceRoots.toString())
    }
}

@Composable
private fun SyncPanel(
    state: AppState,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
) {
    CardSection(title = "Sync", trailing = if (state.sync.running) "running" else "paused") {
        StatRow("State", state.sync.status.ifBlank { if (state.sync.running) "running" else "paused" })
        StatRow("Account", state.account?.authorizationState ?: "not linked")
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
        }
    }
}

@Composable
private fun ProviderPanel(
    state: AppState,
    snapshotLink: String,
    onOpenDriveFolder: () -> Unit,
    onCopySnapshotLink: () -> Unit,
    onOpenSnapshotLink: () -> Unit,
) {
    val rootStatus = state.roots.firstOrNull()?.status ?: "SAF provider root"
    CardSection(title = "Files", trailing = "DocumentsProvider") {
        StatRow("Provider", DocumentsProviderAuthority)
        StatRow("Root", rootStatus)
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
            Text("Open in Files")
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
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
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
                    Text("${device.role.ifBlank { "member" }} | ${device.state}", color = Muted, style = MaterialTheme.typography.bodySmall)
                    Text(device.detail, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (device.canAppointAdmin) {
                    TextButton(onClick = { onAppointAdmin(device.pubkey) }) {
                        Text("Admin")
                    }
                }
                if (device.canDemoteAdmin) {
                    TextButton(onClick = { onDemoteAdmin(device.pubkey) }) {
                        Text("Member")
                    }
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
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
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
private fun RootsPanel(
    roots: List<SyncRoot>,
    onAddRoot: () -> Unit,
    onRemoveRoot: (String) -> Unit,
) {
    CardSection(title = "Roots", trailing = "${roots.size}") {
        if (roots.isEmpty()) {
            Text("No roots", color = Muted)
        } else {
            roots.forEach { root ->
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
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
        OutlinedButton(onClick = onAddRoot) {
            Icon(painterResource(R.drawable.ic_add), contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Add root")
        }
    }
}

@Composable
private fun Notice(text: String) {
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
private fun CardSection(
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
