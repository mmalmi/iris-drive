package to.iris.drive.app

import android.content.ClipboardManager
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
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
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.StateFlow
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.core.RecoverySecretExport

private val ProviderRoot: String
    get() = "content://${BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY}/document/root"
internal const val RecoveryPhraseWordCount = 12

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
    RestoreOptions,
    RestoreRecoveryPhrase,
    RestoreSecretKey,
    LinkDevice,
}

internal data class RecoveryWordsInputResult(
    val words: List<String>,
    val index: Int,
)

internal data class ShareDialogRequest(
    val id: Long,
    val sourcePath: String,
    val displayName: String,
)

internal fun fillRecoveryWords(
    words: List<String>,
    startIndex: Int,
    input: String,
): RecoveryWordsInputResult {
    val lastWordIndex = RecoveryPhraseWordCount - 1
    val normalizedWords = MutableList(RecoveryPhraseWordCount) { index -> words.getOrElse(index) { "" } }
    val normalizedParts = input
        .trim()
        .split(Regex("\\s+"))
        .filter { it.isNotBlank() }
        .map { it.lowercase() }
    if (normalizedParts.size <= 1) {
        return RecoveryWordsInputResult(
            words = normalizedWords.also { current ->
                current[startIndex.coerceIn(0, lastWordIndex)] = input.trim().lowercase()
            },
            index = startIndex.coerceIn(0, lastWordIndex),
        )
    }
    val next = normalizedWords
    val boundedStart = startIndex.coerceIn(0, lastWordIndex)
    normalizedParts.forEachIndexed { offset, word ->
        val target = boundedStart + offset
        if (target <= lastWordIndex) {
            next[target] = word
        }
    }
    return RecoveryWordsInputResult(
        words = next,
        index = (boundedStart + normalizedParts.size - 1).coerceAtMost(lastWordIndex),
    )
}

internal fun recoveryPhraseFromWords(words: List<String>): String =
    words.take(RecoveryPhraseWordCount).joinToString(" ") { it.trim().lowercase() }

internal enum class MainTab(
    val label: String,
    val testTag: String,
    val iconRes: Int,
) {
    MyDrive("My Drive", "tabMyDrive", R.drawable.ic_drive),
    Devices("AppKeys", "tabDevices", R.drawable.ic_devices),
    Shares("Shares", "tabShares", R.drawable.ic_drive),
    Backups("Backups", "tabBackups", R.drawable.ic_backup),
    Settings("Settings", "tabSettings", R.drawable.ic_settings),
}

@Composable
internal fun IrisDriveAndroidApp(
    stateFlow: StateFlow<AppState>,
    shareDialogFlow: StateFlow<ShareDialogRequest?>,
    onCreateProfile: (String) -> Unit,
    onRestoreProfile: (String, String) -> Unit,
    onLinkDevice: (String, String) -> Unit,
    onCopyText: (String, String) -> Unit,
    onExportRecoverySecret: () -> RecoverySecretExport,
    onOpenUrl: (String) -> Unit,
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
    onAddRoot: (String, String) -> Unit,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
) {
    val state by stateFlow.collectAsState()
    val shareDialogRequest by shareDialogFlow.collectAsState()
    val profile = state.profile
    var selectedTab by remember { mutableStateOf(MainTab.MyDrive) }

    LaunchedEffect(shareDialogRequest?.id, state.isSetupComplete) {
        if (state.isSetupComplete && shareDialogRequest != null) {
            selectedTab = MainTab.Shares
        }
    }

    IrisDriveTheme {
        Scaffold(
            modifier = Modifier.testTag("irisDriveApp"),
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
                if (state.isRevoked && profile != null) {
                    RevokedDeviceContent(
                        padding = padding,
                        state = state,
                        onCopyText = onCopyText,
                        onRelink = {
                            val label = profile.appKeyLabel.ifBlank { "Android" }
                            onLinkDevice(profile.currentAppKeyNpub, label)
                        },
                        onLogout = onLogout,
                    )
                } else if (state.isAwaitingApproval && profile != null) {
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
                val activeProfile = profile ?: return@Scaffold
                AuthenticatedContent(
                    padding = padding,
                    selectedTab = selectedTab,
                    onSelectTab = { selectedTab = it },
                    shareDialogRequest = shareDialogRequest,
                    state = state,
                    onStartSync = onStartSync,
                    onStopSync = onStopSync,
                    onCopyAppKey = { onCopyText("AppKey", activeProfile.currentAppKeyNpub) },
                    onCopyDeviceKey = { onCopyText("AppKey", activeProfile.devicePubkey) },
                    onCopyText = onCopyText,
                    onExportRecoverySecret = onExportRecoverySecret,
                    onCopyLinkInvite = { onCopyText("Invite link", activeProfile.appKeyLinkInvite) },
                    onCopySnapshotLink = { onCopyText("drive.iris.to link", state.snapshotLink) },
                    onOpenSnapshotLink = { onOpenUrl(state.snapshotLink) },
                    onOpenDriveFolder = onOpenDriveFolder,
                    onApproveDevice = onApproveDevice,
                    onRejectDevice = onRejectDevice,
                    onResetInvite = onResetInvite,
                    onDeleteDevice = onDeleteDevice,
                    onAppointAdmin = onAppointAdmin,
                    onDemoteAdmin = onDemoteAdmin,
                    onLogout = onLogout,
                    onAddRelay = onAddRelay,
                    onRemoveRelay = onRemoveRelay,
                    onResetRelays = onResetRelays,
                    onAddBackupTarget = onAddBackupTarget,
                    onRemoveBackupTarget = onRemoveBackupTarget,
                    onAddBlossomServer = onAddBlossomServer,
                    onRemoveBlossomServer = onRemoveBlossomServer,
                    onSyncBackups = onSyncBackups,
                    onCheckBackups = onCheckBackups,
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
}

@Composable
private fun RevokedDeviceContent(
    padding: PaddingValues,
    state: AppState,
    onCopyText: (String, String) -> Unit,
    onRelink: () -> Unit,
    onLogout: () -> Unit,
) {
    val profile = state.profile ?: return
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
            Text("AppKey removed", color = Ink, fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.headlineSmall)
            Text("This app install no longer has access to Iris Drive.", color = Muted)
            Text(profile.currentAppKeyNpub, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(profile.devicePubkey, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            SetupPrimaryButton(
                text = "Link this app install again",
                onClick = onRelink,
                testTag = "relinkRevokedDevice",
            )
            SetupSecondaryButton(
                text = "Copy AppKey",
                onClick = { onCopyText("AppKey", profile.devicePubkey) },
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
    val profile = state.profile ?: return
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
            Text(profile.currentAppKeyNpub, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(profile.devicePubkey, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            SetupSecondaryButton(
                text = "Copy AppKey",
                onClick = { onCopyText("AppKey", profile.devicePubkey) },
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
    var recoveryWords by remember { mutableStateOf(List(RecoveryPhraseWordCount) { "" }) }
    var recoveryWordIndex by remember { mutableStateOf(0) }
    var linkOwner by remember { mutableStateOf("") }
    var submittedLinkOwner by remember { mutableStateOf("") }
    var route by remember { mutableStateOf(SetupRoute.Welcome) }
    var showLinkScanner by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val linkOwnerIsComplete = remember(linkOwner) {
        NativeCore.isCompleteLinkInput(linkOwner)
    }
    val photoPicker = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        selectedPhoto = uri?.lastPathSegment.orEmpty()
    }
    fun submitLinkOwner(value: String, force: Boolean) {
        val trimmed = value.trim()
        if (trimmed.isBlank()) return
        if (!NativeCore.isCompleteLinkInput(trimmed)) return
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
                        onClick = { route = SetupRoute.RestoreOptions },
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
                SetupRoute.RestoreOptions -> {
                    SetupFormHeader(title = "Restore", onBack = { route = SetupRoute.Welcome })
                    SetupSecondaryButton(
                        text = "Link app install",
                        onClick = { route = SetupRoute.LinkDevice },
                        testTag = "openLinkDevice",
                    )
                    SetupSecondaryButton(
                        text = "Restore from recovery phrase",
                        onClick = { route = SetupRoute.RestoreRecoveryPhrase },
                        testTag = "openRecoveryPhrase",
                    )
                    SetupSecondaryButton(
                        text = "Restore from secret key",
                        onClick = { route = SetupRoute.RestoreSecretKey },
                        testTag = "openSecretKey",
                    )
                }
                SetupRoute.RestoreRecoveryPhrase -> {
                    SetupFormHeader(title = "Recovery phrase", onBack = { route = SetupRoute.RestoreOptions })
                    val currentWord = recoveryWords.getOrElse(recoveryWordIndex) { "" }
                    val allWordsFilled = recoveryWords.all { it.isNotBlank() }
                    OutlinedTextField(
                        value = currentWord,
                        onValueChange = { input ->
                            val result = fillRecoveryWords(recoveryWords, recoveryWordIndex, input)
                            recoveryWords = result.words
                            recoveryWordIndex = result.index
                        },
                        modifier = Modifier.fillMaxWidth().testTag("recoveryWordInput"),
                        singleLine = true,
                        label = { Text("Word ${recoveryWordIndex + 1}") },
                        keyboardOptions = KeyboardOptions(
                            imeAction = if (recoveryWordIndex == RecoveryPhraseWordCount - 1) {
                                ImeAction.Done
                            } else {
                                ImeAction.Next
                            },
                        ),
                        keyboardActions = KeyboardActions(
                            onNext = {
                                if (currentWord.isNotBlank()) {
                                    recoveryWordIndex =
                                        (recoveryWordIndex + 1).coerceAtMost(RecoveryPhraseWordCount - 1)
                                }
                            },
                            onDone = {
                                if (allWordsFilled) {
                                    onRestoreProfile(recoveryPhraseFromWords(recoveryWords))
                                }
                            },
                        ),
                    )
                    SetupSecondaryButton(
                        text = "Paste from clipboard",
                        onClick = {
                            val clipboard = context.getSystemService(ClipboardManager::class.java)
                            val clipboardText = clipboard?.primaryClip
                                ?.takeIf { it.itemCount > 0 }
                                ?.getItemAt(0)
                                ?.coerceToText(context)
                                ?.toString()
                                .orEmpty()
                            val result = fillRecoveryWords(
                                recoveryWords,
                                recoveryWordIndex,
                                clipboardText,
                            )
                            recoveryWords = result.words
                            recoveryWordIndex = result.index
                        },
                        testTag = "pasteRecoveryPhrase",
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        OutlinedButton(
                            onClick = { recoveryWordIndex = (recoveryWordIndex - 1).coerceAtLeast(0) },
                            enabled = recoveryWordIndex > 0,
                            modifier = Modifier.weight(1f),
                        ) {
                            Text("Back")
                        }
                        Button(
                            onClick = {
                                if (recoveryWordIndex == RecoveryPhraseWordCount - 1) {
                                    onRestoreProfile(recoveryPhraseFromWords(recoveryWords))
                                } else {
                                    recoveryWordIndex =
                                        (recoveryWordIndex + 1).coerceAtMost(RecoveryPhraseWordCount - 1)
                                }
                            },
                            enabled = if (recoveryWordIndex == RecoveryPhraseWordCount - 1) {
                                allWordsFilled
                            } else {
                                currentWord.isNotBlank()
                            },
                            modifier = Modifier.weight(1f).testTag(
                                if (recoveryWordIndex == RecoveryPhraseWordCount - 1) {
                                    "restoreRecoveryPhraseSubmit"
                                } else {
                                    "restoreRecoveryPhraseNext"
                                },
                            ),
                            shape = RoundedCornerShape(6.dp),
                        ) {
                            Text(if (recoveryWordIndex == RecoveryPhraseWordCount - 1) "Restore" else "Next")
                        }
                    }
                }
                SetupRoute.RestoreSecretKey -> {
                    SetupFormHeader(title = "Secret key", onBack = { route = SetupRoute.RestoreOptions })
                    OutlinedTextField(
                        value = restoreSecret,
                        onValueChange = { restoreSecret = it },
                        modifier = Modifier.fillMaxWidth().testTag("restoreSecretKeyInput"),
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
                        text = "Restore",
                        onClick = { onRestoreProfile(restoreSecret) },
                        enabled = restoreSecret.isNotBlank(),
                        testTag = "restoreSecretKeySubmit",
                    )
                }
                SetupRoute.LinkDevice -> {
                    SetupFormHeader(title = "Link app install", onBack = { route = SetupRoute.RestoreOptions })
                    OutlinedTextField(
                        value = linkOwner,
                        onValueChange = {
                            linkOwner = it
                            submitLinkOwner(it, force = false)
                        },
                        modifier = Modifier.fillMaxWidth().testTag("linkOwnerInput"),
                        singleLine = true,
                        label = { Text("IrisProfile invite link or admin AppKey") },
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(
                            onDone = { submitLinkOwner(linkOwner, force = true) },
                        ),
                    )
                    SetupPrimaryButton(
                        text = "Link app install",
                        onClick = { submitLinkOwner(linkOwner, force = true) },
                        enabled = linkOwnerIsComplete,
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
