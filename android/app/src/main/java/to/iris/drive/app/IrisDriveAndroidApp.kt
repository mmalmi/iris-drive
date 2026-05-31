package to.iris.drive.app

import androidx.compose.foundation.Image
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeCore

private val ProviderRoot: String
    get() = "content://${BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY}/document/root"

private fun isCompleteDeviceLinkOwnerInput(value: String): Boolean {
    return runCatching {
        JSONObject(NativeCore.classifyLinkInputJson(value.trim())).optBoolean("is_complete")
    }.getOrDefault(false)
}

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

internal val Ink: Color
    @Composable get() = MaterialTheme.colorScheme.onSurface

internal val Muted: Color
    @Composable get() = MaterialTheme.colorScheme.onSurfaceVariant

internal val Teal: Color
    @Composable get() = MaterialTheme.colorScheme.primary

internal val SoftTeal: Color
    @Composable get() = MaterialTheme.colorScheme.primaryContainer

internal val Danger: Color
    @Composable get() = MaterialTheme.colorScheme.error

private enum class SetupRoute {
    Welcome,
    CreateProfile,
    CreatePhoto,
    SignIn,
    LinkDevice,
}

internal enum class MainTab(
    val label: String,
    val testTag: String,
    val iconRes: Int,
) {
    MyDrive("My Drive", "tabMyDrive", R.drawable.ic_drive),
    Devices("Devices", "tabDevices", R.drawable.ic_devices),
    Backups("Backups", "tabBackups", R.drawable.ic_backup),
    Settings("Settings", "tabSettings", R.drawable.ic_settings),
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
    onResetInvite: () -> Unit,
    onDeleteDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
    onLogout: () -> Unit,
    onAddRelay: (String) -> Unit,
    onRemoveRelay: (String) -> Unit,
    onResetRelays: () -> Unit,
    onAddRoot: (String, String) -> Unit,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
) {
    val state by stateFlow.collectAsState()
    val account = state.account
    var selectedTab by remember { mutableStateOf(MainTab.MyDrive) }

    IrisDriveTheme {
        Scaffold(
            containerColor = Background,
            topBar = {
                if (state.isSetupComplete) {
                    AppTopBar(title = selectedTab.label)
                }
            },
            bottomBar = {
                if (state.isSetupComplete) {
                    MainNavigationBar(
                        selectedTab = selectedTab,
                        onSelectTab = { selectedTab = it },
                    )
                }
            },
        ) { padding ->
            if (!state.isSetupComplete) {
                if (state.isRevoked && account != null) {
                    RevokedDeviceContent(
                        padding = padding,
                        state = state,
                        onCopyText = onCopyText,
                        onRelink = {
                            val label = account.deviceLabel.ifBlank { "Android" }
                            onLinkDevice(account.ownerPubkey, label)
                        },
                        onLogout = onLogout,
                    )
                } else if (state.isAwaitingApproval && account != null) {
                    AwaitingApprovalContent(
                        padding = padding,
                        state = state,
                        onCopyText = onCopyText,
                        onLogout = onLogout,
                    )
                } else {
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
                        },
                )
                }
            } else {
                val activeAccount = account ?: return@Scaffold
                AuthenticatedContent(
                    padding = padding,
                    selectedTab = selectedTab,
                    onSelectTab = { selectedTab = it },
                    state = state,
                    onStartSync = onStartSync,
                    onStopSync = onStopSync,
                    onCopyOwnerKey = { onCopyText("Owner key", activeAccount.ownerPubkey) },
                    onCopyDeviceKey = { onCopyText("Device key", activeAccount.devicePubkey) },
                    onCopyLinkInvite = { onCopyText("Invite link", activeAccount.deviceLinkInvite) },
                    onCopySnapshotLink = { onCopyText("drive.iris.to link", state.snapshotLink) },
                    onOpenSnapshotLink = { onOpenUrl(state.snapshotLink) },
                    onOpenDriveFolder = onOpenDriveFolder,
                    onApproveDevice = onApproveDevice,
                    onResetInvite = onResetInvite,
                    onDeleteDevice = onDeleteDevice,
                    onAppointAdmin = onAppointAdmin,
                    onDemoteAdmin = onDemoteAdmin,
                    onLogout = onLogout,
                    onAddRelay = onAddRelay,
                    onRemoveRelay = onRemoveRelay,
                    onResetRelays = onResetRelays,
                )
            }
        }
    }
}

@Composable
private fun RevokedDeviceContent(
    padding: PaddingValues,
    state: AppState,
    onCopyText: (String, String) -> Unit,
    onRelink: () -> Unit,
    onLogout: () -> Unit,
) {
    val account = state.account ?: return
    Box(
        modifier = Modifier.fillMaxSize().padding(padding).padding(32.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().widthIn(max = 360.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            SetupBrand()
            Text("Device removed", color = Ink, fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.headlineSmall)
            Text("This device no longer has access to Iris Drive.", color = Muted)
            Text(account.ownerPubkey, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(account.devicePubkey, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            SetupPrimaryButton(
                text = "Link this device again",
                onClick = onRelink,
                testTag = "relinkRevokedDevice",
            )
            SetupSecondaryButton(
                text = "Copy device ID",
                onClick = { onCopyText("Device key", account.devicePubkey) },
            )
            OutlinedButton(
                onClick = onLogout,
                modifier = Modifier.fillMaxWidth().height(48.dp),
                shape = RoundedCornerShape(6.dp),
            ) {
                Text("Log out")
            }
        }
    }
}

@Composable
private fun AwaitingApprovalContent(
    padding: PaddingValues,
    state: AppState,
    onCopyText: (String, String) -> Unit,
    onLogout: () -> Unit,
) {
    val account = state.account ?: return
    Box(
        modifier = Modifier.fillMaxSize().padding(padding).padding(32.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().widthIn(max = 360.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            SetupBrand()
            Text("Waiting for approval", color = Ink, fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.headlineSmall)
            Text(account.ownerPubkey, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(account.devicePubkey, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            SetupSecondaryButton(
                text = "Copy device ID",
                onClick = { onCopyText("Device key", account.devicePubkey) },
            )
            OutlinedButton(
                onClick = onLogout,
                modifier = Modifier.fillMaxWidth().height(48.dp),
                shape = RoundedCornerShape(6.dp),
            ) {
                Text("Log out")
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
private fun AppTopBar(title: String) {
    TopAppBar(
        title = {
            Column {
                Text(title, fontWeight = FontWeight.SemiBold)
                Text("Iris Drive", color = Muted, style = MaterialTheme.typography.labelMedium)
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
private fun MainNavigationBar(
    selectedTab: MainTab,
    onSelectTab: (MainTab) -> Unit,
) {
    NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
        MainTab.values().forEach { tab ->
            NavigationBarItem(
                selected = selectedTab == tab,
                onClick = { onSelectTab(tab) },
                modifier = Modifier.testTag(tab.testTag),
                icon = {
                    Icon(
                        painter = painterResource(tab.iconRes),
                        contentDescription = null,
                    )
                },
                label = { Text(tab.label) },
            )
        }
    }
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
    var submittedLinkOwner by remember { mutableStateOf("") }
    var route by remember { mutableStateOf(SetupRoute.Welcome) }
    var showLinkScanner by remember { mutableStateOf(false) }
    val photoPicker = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        selectedPhoto = uri?.lastPathSegment.orEmpty()
    }
    fun submitLinkOwner(value: String, force: Boolean) {
        val trimmed = value.trim()
        if (trimmed.isBlank()) return
        if (!force && !isCompleteDeviceLinkOwnerInput(trimmed)) return
        if (submittedLinkOwner == trimmed) return
        submittedLinkOwner = trimmed
        onLinkDevice(trimmed)
    }

    if (showLinkScanner) {
        QrScannerDialog(
            onDismiss = { showLinkScanner = false },
            onScanned = { code ->
                linkOwner = code
                submitLinkOwner(code, force = false)
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
                        testTag = "welcomeCreateProfile",
                    )
                    SetupSecondaryButton(
                        text = "Sign in",
                        onClick = { route = SetupRoute.SignIn },
                        testTag = "welcomeSignIn",
                    )
                }
                SetupRoute.CreateProfile -> {
                    SetupFormHeader(title = "Create profile", onBack = { route = SetupRoute.Welcome })
                    OutlinedTextField(
                        value = createUsername,
                        onValueChange = { createUsername = it },
                        modifier = Modifier.fillMaxWidth().testTag("createUsername"),
                        singleLine = true,
                        label = { Text("Username (optional)") },
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(
                            onDone = {
                                if (createUsername.isBlank()) {
                                    onCreateProfile()
                                } else {
                                    route = SetupRoute.CreatePhoto
                                }
                            },
                        ),
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
                        testTag = "createProfileSubmit",
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
                        modifier = Modifier.fillMaxWidth().testTag("restoreSecret"),
                        singleLine = true,
                        label = { Text("Secret key") },
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(
                            onDone = {
                                if (restoreSecret.isNotBlank()) {
                                    onRestoreProfile(restoreSecret)
                                }
                            },
                        ),
                    )
                    SetupPrimaryButton(
                        text = "Sign in",
                        onClick = { onRestoreProfile(restoreSecret) },
                        enabled = restoreSecret.isNotBlank(),
                    )
                    SetupSecondaryButton(
                        text = "Link this device",
                        onClick = { route = SetupRoute.LinkDevice },
                        testTag = "openLinkDevice",
                    )
                }
                SetupRoute.LinkDevice -> {
                    SetupFormHeader(title = "Link this device", onBack = { route = SetupRoute.Welcome })
                    OutlinedTextField(
                        value = linkOwner,
                        onValueChange = {
                            linkOwner = it
                            submitLinkOwner(it, force = false)
                        },
                        modifier = Modifier.fillMaxWidth().testTag("linkOwnerInput"),
                        singleLine = true,
                        label = { Text("Owner public key or invite link") },
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(
                            onDone = { submitLinkOwner(linkOwner, force = true) },
                        ),
                    )
                    SetupPrimaryButton(
                        text = "Link device",
                        onClick = { submitLinkOwner(linkOwner, force = true) },
                        enabled = linkOwner.isNotBlank(),
                        testTag = "linkDeviceSubmit",
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
    testTag: String? = null,
) {
    val modifier = Modifier
        .fillMaxWidth()
        .height(48.dp)
        .let { base -> if (testTag == null) base else base.testTag(testTag) }

    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier,
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
private fun SetupSecondaryButton(text: String, onClick: () -> Unit, testTag: String? = null) {
    val modifier = Modifier
        .fillMaxWidth()
        .height(48.dp)
        .let { base -> if (testTag == null) base else base.testTag(testTag) }

    OutlinedButton(
        onClick = onClick,
        modifier = modifier,
        shape = RoundedCornerShape(6.dp),
    ) {
        Text(text)
    }
}
