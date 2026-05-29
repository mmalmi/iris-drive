package to.iris.drive.app

import androidx.compose.foundation.Image
import androidx.compose.foundation.Canvas
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
import androidx.compose.ui.draw.clip
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
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
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.Dp
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.BackupState
import to.iris.drive.app.core.DeviceLinkRequestState
import to.iris.drive.app.core.DeviceState
import to.iris.drive.app.core.NativeCore

private const val ProviderRoot = "content://to.iris.drive.documents/document/root"

private val IrisLightBackground = Color(0xFFF7FAF8)
private val IrisLightSurface = Color.White
private val IrisLightInk = Color(0xFF172321)
private val IrisLightMuted = Color(0xFF657370)
private val IrisLightSoftTeal = Color(0xFFE7F4F0)
private val IrisDarkBackground = Color(0xFF0C0A09)
private val IrisDarkSurface = Color(0xFF1C1917)
private val IrisDarkSurfaceVariant = Color(0xFF44403C)
private val IrisDarkInk = Color(0xFFF5F5F4)
private val IrisDarkMuted = Color(0xFFD6D3D1)
private val IrisTeal = Color(0xFF167C80)
private val IrisDarkTeal = Color(0xFF5EEAD4)
private val IrisAmber = Color(0xFFF5A524)
private val IrisDanger = Color(0xFFB42318)
private val IrisDarkDanger = Color(0xFFFB7185)
private val IrisErrorContainer = Color(0xFFFEE4E2)
private val IrisDarkErrorContainer = Color(0xFF4C0519)

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
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
) {
    val state by stateFlow.collectAsState()
    val account = state.account

    IrisDriveTheme {
        Scaffold(
            containerColor = Background,
            topBar = {
                if (account != null) {
                    AppTopBar()
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
                    onCopyLinkInvite = { onCopyText("Invite link", account.deviceLinkInvite) },
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
                )
            }
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
        primary = IrisDarkInk,
        secondary = IrisDarkTeal,
        tertiary = IrisDarkDanger,
        background = IrisDarkBackground,
        surface = IrisDarkSurface,
        surfaceVariant = IrisDarkSurfaceVariant,
        primaryContainer = IrisDarkSurfaceVariant,
        error = IrisDarkDanger,
        errorContainer = IrisDarkErrorContainer,
        onPrimary = Color(0xFF111827),
        onSecondary = Color(0xFF042F2E),
        onBackground = IrisDarkInk,
        onSurface = IrisDarkInk,
        onSurfaceVariant = IrisDarkMuted,
        onPrimaryContainer = IrisDarkInk,
        onErrorContainer = Color(0xFFFFD9E2),
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
private fun AppTopBar() {
    TopAppBar(
        title = {
            Column {
                Text("Iris Drive", fontWeight = FontWeight.SemiBold)
                Text("Android", color = Muted, style = MaterialTheme.typography.labelMedium)
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
    var showLinkScanner by remember { mutableStateOf(false) }
    val photoPicker = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        selectedPhoto = uri?.lastPathSegment.orEmpty()
    }

    if (showLinkScanner) {
        QrScannerDialog(
            onDismiss = { showLinkScanner = false },
            onScanned = { code ->
                linkOwner = code
                showLinkScanner = false
                null
            },
        )
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
                        label = { Text("Owner public key or invite link") },
                    )
                    SetupPrimaryButton(
                        text = "Link device",
                        onClick = { onLinkDevice(linkOwner) },
                        enabled = linkOwner.isNotBlank(),
                    )
                    SetupSecondaryButton(
                        text = "Scan invite QR",
                        onClick = { showLinkScanner = true },
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
    onCopyLinkInvite: () -> Unit,
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
                linkInvite = state.account?.deviceLinkInvite.orEmpty(),
                inboundRequests = state.account?.inboundDeviceLinkRequests.orEmpty(),
                canApprove = state.account?.hasOwnerSigningAuthority == true,
                onCopyLinkInvite = onCopyLinkInvite,
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
                onAddRelay = onAddRelay,
                onRemoveRelay = onRemoveRelay,
                onResetRelays = onResetRelays,
            )
        }
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
        StatRow("Storage", byteString(state.visibleFileBytes))
        StatRow("Devices", state.authorizedDeviceCount.toString())
    }
}

@Composable
private fun SyncPanel(
    state: AppState,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
) {
    CardSection(title = "Sync", trailing = if (state.sync.running) "on" else "paused") {
        StatRow("State", state.sync.status.ifBlank { if (state.sync.running) "on" else "paused" })
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Button(onClick = onStartSync, enabled = !state.sync.running) {
                Icon(painterResource(R.drawable.ic_play), contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Resume")
            }
            OutlinedButton(onClick = onStopSync, enabled = state.sync.running) {
                Icon(painterResource(R.drawable.ic_stop), contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Pause")
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
    linkInvite: String,
    inboundRequests: List<DeviceLinkRequestState>,
    canApprove: Boolean,
    onCopyLinkInvite: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRevokeDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
) {
    var request by remember { mutableStateOf("") }
    var label by remember { mutableStateOf("") }

    CardSection(title = "Devices", trailing = "${devices.size}") {
        if (linkInvite.isNotBlank()) {
            Text("Invite device", fontWeight = FontWeight.SemiBold)
            QrCode(linkInvite, side = 220.dp, modifier = Modifier.align(Alignment.CenterHorizontally))
            Text(linkInvite, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            OutlinedButton(onClick = onCopyLinkInvite) {
                Text("Copy invite link")
            }
        }
        inboundRequests.forEach { inbound ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.weight(1f)) {
                    Text(inbound.label.ifBlank { "New device" }, fontWeight = FontWeight.SemiBold)
                    Text(inbound.devicePubkey, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Button(
                    onClick = { onApproveDevice(inbound.requestLink, inbound.label) },
                    enabled = canApprove,
                ) {
                    Text("Approve")
                }
            }
        }
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
                    if (device.isCurrentDevice) {
                        Text("Device ID: ${device.pubkey}", color = Muted, style = MaterialTheme.typography.bodySmall)
                    }
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
            label = { Text("Device ID") },
        )
        OutlinedTextField(
            value = label,
            onValueChange = { label = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("Label") },
        )
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
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
private fun QrCode(
    value: String,
    modifier: Modifier = Modifier,
    side: Dp = 180.dp,
) {
    val qr = remember(value) {
        runCatching { JSONObject(NativeCore.qrMatrixJson(value)) }.getOrElse { JSONObject() }
    }
    val width = qr.optInt("width")
    val cells = qr.optJSONArray("cells")
    Canvas(
        modifier = modifier
            .size(side)
            .clip(RoundedCornerShape(8.dp))
            .background(Color.White),
    ) {
        drawRect(Color.White)
        if (width <= 0 || cells == null) return@Canvas
        val quiet = 3
        val modules = width + quiet * 2
        val cell = size.minDimension / modules
        for (y in 0 until width) {
            for (x in 0 until width) {
                if (cells.optBoolean(y * width + x)) {
                    drawRect(
                        color = Color(0xFF111827),
                        topLeft = androidx.compose.ui.geometry.Offset((x + quiet) * cell, (y + quiet) * cell),
                        size = Size(cell, cell),
                    )
                }
            }
        }
    }
}

@Composable
private fun SettingsPanel(
    state: AppState,
    onCopyOwnerKey: () -> Unit,
    onCopyDeviceKey: () -> Unit,
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
