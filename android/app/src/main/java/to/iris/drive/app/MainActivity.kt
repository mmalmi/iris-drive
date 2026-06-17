package to.iris.drive.app

import android.Manifest
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.DocumentsContract
import android.system.Os
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeActions
import to.iris.drive.app.core.NativeCore
import to.iris.drive.app.core.recoverySecretExportFromJson
import to.iris.drive.app.sync.BackgroundSyncPolicy
import to.iris.drive.app.sync.IrisDriveBackgroundSync
import to.iris.drive.app.sync.IrisDriveSyncService
import to.iris.drive.app.update.AndroidSelfUpdateManager
import to.iris.drive.app.update.SelfUpdateActions

class MainActivity : ComponentActivity() {
    private val stateFlow = MutableStateFlow(AppState(isLoaded = false))
    private val backupCheckProgressFlow = MutableStateFlow(BackupCheckProgress())
    private val shareDialogFlow = MutableStateFlow<ShareDialogRequest?>(null)
    private val nativeCoreExecutor = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "IrisDriveNativeCore")
    }
    private val nativeCoreDispatcher = nativeCoreExecutor.asCoroutineDispatcher()
    private var nativeHandle: Long = 0
    private var refreshJob: Job? = null
    private var nativeRefreshInFlight = false
    private var nativeRefreshPending = false
    private val nativeRefreshCallbacks = mutableListOf<(AppState) -> Unit>()
    private var nextShareDialogRequestId = 0L
    private lateinit var selfUpdateManager: AndroidSelfUpdateManager

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        applyDebugEnvironment(intent)
        selfUpdateManager =
            AndroidSelfUpdateManager(
                context = applicationContext,
                scope = lifecycleScope,
                ioDispatcher = Dispatchers.IO,
                dataDir = filesDir,
            )

        setContent {
            val notificationPermissionLauncher =
                rememberLauncherForActivityResult(
                    ActivityResultContracts.RequestPermission(),
                ) { granted ->
                    if (granted) {
                        dispatch(NativeActions.startSync(), ::autoStartSyncIfNeeded)
                        startSyncService()
                    }
                }

            IrisDriveAndroidApp(
                stateFlow = stateFlow,
                shareDialogFlow = shareDialogFlow,
                selfUpdateStateFlow = selfUpdateManager.state,
                backupCheckProgressFlow = backupCheckProgressFlow,
                selfUpdateActions = SelfUpdateActions(
                    setAutoCheck = selfUpdateManager::setAutoCheckEnabled,
                    check = { selfUpdateManager.check() },
                    download = { selfUpdateManager.download() },
                    install = { selfUpdateManager.install(this@MainActivity) },
                ),
                onCreateProfile = { deviceLabel ->
                    dispatch(NativeActions.createProfile(resolveDeviceLabel(deviceLabel)), ::autoStartSyncIfNeeded)
                },
                onRestoreProfile = { secret, deviceLabel ->
                    dispatch(
                        NativeActions.restoreProfile(secret, resolveDeviceLabel(deviceLabel)),
                        ::autoStartSyncIfNeeded,
                    )
                },
                onLinkDevice = { linkTarget, deviceLabel ->
                    dispatch(
                        NativeActions.linkDevice(linkTarget, resolveDeviceLabel(deviceLabel)),
                        ::autoStartSyncIfNeeded,
                    )
                },
                onCopyText = ::copyToClipboard,
                onExportRecoverySecret = {
                    recoverySecretExportFromJson(
                        NativeCore.exportRecoverySecretJson(filesDir.absolutePath),
                    )
                },
                onOpenUrl = ::openUrl,
                onOpenDriveFolder = ::openDriveFolder,
                onApproveDevice = { request, label ->
                    dispatch(NativeActions.approveDevice(request, label), ::autoStartSyncIfNeeded)
                },
                onRejectDevice = { request ->
                    dispatch(NativeActions.rejectDevice(request))
                },
                onResetInvite = { dispatch(NativeActions.resetInvite()) },
                onAddRecoveryKey = { recoveryPubkey ->
                    dispatch(NativeActions.addRecoveryDevice(recoveryPubkey))
                },
                onDeleteDevice = { devicePubkey ->
                    dispatch(NativeActions.deleteDevice(devicePubkey))
                },
                onAppointAdmin = { devicePubkey ->
                    dispatch(NativeActions.appointAdmin(devicePubkey))
                },
                onDemoteAdmin = { devicePubkey ->
                    dispatch(NativeActions.demoteAdmin(devicePubkey))
                },
                onLogout = {
                    IrisDriveBackgroundSync.cancel(applicationContext)
                    stopSyncService()
                    dispatch(NativeActions.logout())
                },
                onAddRelay = { url -> dispatch(NativeActions.addRelay(url)) },
                onRemoveRelay = { url -> dispatch(NativeActions.removeRelay(url)) },
                onResetRelays = { dispatch(NativeActions.resetRelays()) },
                onAddBackupTarget = { target, label ->
                    dispatch(NativeActions.addBackupTarget(target, label))
                },
                onRemoveBackupTarget = { target ->
                    dispatch(NativeActions.removeBackupTarget(target))
                },
                onAddBlossomServer = { url ->
                    dispatch(NativeActions.addBlossomServer(url))
                },
                onRemoveBlossomServer = { url ->
                    dispatch(NativeActions.removeBlossomServer(url))
                },
                onSyncBackups = { target ->
                    dispatch(NativeActions.syncBackups(target))
                },
                onCheckBackups = { target ->
                    checkBackupsWithProgress(target)
                },
                onCreateShare = { sourcePath, displayName ->
                    createShareFromProviderPath(sourcePath, displayName)
                },
                onInviteShareMember = { shareId, profileId, appKey, role, representativeNpubHint, displayName, label ->
                    dispatch(
                        NativeActions.inviteShareMember(
                            shareId = shareId,
                            profileId = profileId,
                            appKey = appKey,
                            role = role,
                            representativeNpubHint = representativeNpubHint,
                            displayName = displayName,
                            label = label,
                        ),
                    ) { state ->
                        if (state.lastShareInvite.isNotBlank()) {
                            copyToClipboard("Share invite", state.lastShareInvite)
                        }
                    }
                },
                onInviteShareMemberFromEvidence = { shareId, evidenceJson, role, displayName ->
                    dispatch(
                        NativeActions.inviteShareMemberFromEvidence(
                            shareId = shareId,
                            evidenceJson = evidenceJson,
                            role = role,
                            displayName = displayName,
                        ),
                    ) { state ->
                        if (state.lastShareInvite.isNotBlank()) {
                            copyToClipboard("Share invite", state.lastShareInvite)
                        }
                    }
                },
                onExportShareRecipientEvidence = { displayName ->
                    dispatch(NativeActions.exportShareRecipientEvidence(displayName)) { state ->
                        if (state.lastShareRecipientEvidence.isNotBlank()) {
                            copyToClipboard("Share identity", state.lastShareRecipientEvidence)
                        }
                    }
                },
                onRecordPendingShareInvite = { shareId, representativeNpubHint, role, displayName ->
                    dispatch(
                        NativeActions.recordPendingShareInvite(
                            shareId = shareId,
                            representativeNpubHint = representativeNpubHint,
                            role = role,
                            displayName = displayName,
                        ),
                    )
                },
                onAcceptShareInvite = { invite ->
                    dispatch(NativeActions.acceptShareInvite(invite))
                },
                onRevokeShareMember = { shareId, profileId ->
                    dispatch(NativeActions.revokeShareMember(shareId, profileId))
                },
                onOpenSharePath = { path ->
                    openProviderPath(path)
                },
                onDeleteShare = { shareId ->
                    dispatch(NativeActions.deleteShare(shareId))
                },
                onAddShareShortcut = { shareId, path ->
                    dispatch(NativeActions.addShareShortcut(shareId, path))
                },
                onRepairShareWraps = { shareId ->
                    dispatch(NativeActions.repairShareWraps(shareId))
                },
                onAddRoot = { name, path -> dispatch(NativeActions.addRoot(name, path)) },
                onStartSync = {
                    if (needsNotificationPermission()) {
                        notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                    } else {
                        dispatch(NativeActions.startSync(), ::autoStartSyncIfNeeded)
                        startSyncService()
                    }
                },
                onStopSync = {
                    dispatch(NativeActions.stopSync()) { state ->
                        IrisDriveBackgroundSync.scheduleIfNeeded(applicationContext, state)
                    }
                    stopSyncService()
                },
            )
        }
        startNativeCore(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        applyDebugEnvironment(intent)
        handleLaunchIntent(intent)
    }

    override fun onDestroy() {
        refreshJob?.cancel()
        refreshJob = null
        if (::selfUpdateManager.isInitialized) {
            selfUpdateManager.stopAutomaticChecks()
        }
        val handle = nativeHandle
        nativeHandle = 0
        if (handle != 0L) {
            nativeCoreExecutor.execute { NativeCore.appFree(handle) }
        }
        nativeCoreDispatcher.close()
        super.onDestroy()
    }

    private fun startNativeCore(launchIntent: Intent?) {
        lifecycleScope.launch(nativeCoreDispatcher) {
            NativeCore.initializeAndroidContext(applicationContext)
            val handle = NativeCore.appNew(filesDir.absolutePath, BuildConfig.VERSION_NAME)
            var installed = false
            try {
                val initialJson = NativeCore.stateJson(handle)
                val initialState = stateFromJson(initialJson)
                withContext(Dispatchers.Main) {
                    if (nativeHandle != 0L) {
                        return@withContext
                    }
                    nativeHandle = handle
                    installed = true
                    stateFlow.value = initialState
                    writeDebugState(initialJson)
                    IrisDriveBackgroundSync.scheduleIfNeeded(applicationContext, initialState)
                    refresh(::autoStartSyncIfNeeded)
                    refreshJob = lifecycleScope.launch {
                        while (true) {
                            delay(2_000)
                            refresh()
                        }
                    }
                    handleLaunchIntent(launchIntent)
                    selfUpdateManager.startAutomaticChecks()
                }
            } finally {
                if (!installed) {
                    NativeCore.appFree(handle)
                }
            }
        }
    }

    private fun refresh(onState: ((AppState) -> Unit)? = null) {
        val handle = nativeHandle
        if (handle == 0L) return
        if (nativeRefreshInFlight) {
            nativeRefreshPending = true
            if (onState != null) {
                nativeRefreshCallbacks.add(onState)
            }
            return
        }
        nativeRefreshInFlight = true
        lifecycleScope.launch(nativeCoreDispatcher) {
            val json = NativeCore.refreshJson(handle)
            val state = stateFromJson(json)
            withContext(Dispatchers.Main) {
                val pendingCallbacks = nativeRefreshCallbacks.toList()
                nativeRefreshCallbacks.clear()
                nativeRefreshInFlight = false
                applyNativeState(state, onState, json)
                pendingCallbacks.forEach { it(state) }
                if (nativeRefreshPending) {
                    nativeRefreshPending = false
                    refresh()
                }
            }
        }
    }

    private fun applyNativeState(
        state: AppState,
        onState: ((AppState) -> Unit)? = null,
        debugJson: String? = null,
    ) {
        stateFlow.value = state
        writeDebugState(debugJson)
        IrisDriveBackgroundSync.scheduleIfNeeded(applicationContext, state)
        onState?.invoke(state)
    }

    private fun dispatch(actionJson: String, onState: ((AppState) -> Unit)? = null) {
        val handle = nativeHandle
        if (handle == 0L) return
        lifecycleScope.launch(nativeCoreDispatcher) {
            val json = NativeCore.dispatchJson(handle, actionJson)
            val state = stateFromJson(json)
            withContext(Dispatchers.Main) {
                applyNativeState(state, onState, json)
            }
        }
    }

    private fun checkBackupsWithProgress(target: String) {
        if (backupCheckProgressFlow.value.isRunning) return
        val targets =
            if (target.isBlank()) {
                stateFlow.value.backups
                    .map { it.target.trim() }
                    .filter { it.isNotEmpty() }
            } else {
                listOf(target.trim()).filter { it.isNotEmpty() }
            }
        if (targets.isEmpty()) return

        val handle = nativeHandle
        if (handle == 0L) return
        backupCheckProgressFlow.value = BackupCheckProgress(
            checked = 0,
            total = targets.size,
            activeTarget = targets.first(),
        )
        lifecycleScope.launch(nativeCoreDispatcher) {
            try {
                for ((index, currentTarget) in targets.withIndex()) {
                    withContext(Dispatchers.Main) {
                        backupCheckProgressFlow.value = BackupCheckProgress(
                            checked = index,
                            total = targets.size,
                            activeTarget = currentTarget,
                        )
                    }
                    val json = NativeCore.dispatchJson(handle, NativeActions.checkBackups(currentTarget))
                    val state = stateFromJson(json)
                    withContext(Dispatchers.Main) {
                        stateFlow.value = state
                        writeDebugState(json)
                        IrisDriveBackgroundSync.scheduleIfNeeded(applicationContext, state)
                        backupCheckProgressFlow.value = BackupCheckProgress(
                            checked = index + 1,
                            total = targets.size,
                            activeTarget = targets.getOrNull(index + 1).orEmpty(),
                        )
                    }
                }
                delay(350)
            } finally {
                withContext(Dispatchers.Main) {
                    backupCheckProgressFlow.value = BackupCheckProgress()
                }
            }
        }
    }

    private fun autoStartSyncIfNeeded(state: AppState) {
        if (state.isRevoked) {
            IrisDriveBackgroundSync.cancel(applicationContext)
            stopSyncService()
            return
        }
        if (BackgroundSyncPolicy.shouldSchedule(state)) {
            IrisDriveBackgroundSync.schedule(applicationContext)
            startSyncService()
        }
    }

    private fun startSyncService() {
        val intent = Intent(this, IrisDriveSyncService::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun stopSyncService() {
        startService(
            Intent(this, IrisDriveSyncService::class.java)
                .setAction(IrisDriveSyncService.ACTION_STOP),
        )
    }

    private fun copyToClipboard(label: String, value: String) {
        if (value.isBlank()) return
        val manager = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        manager.setPrimaryClip(ClipData.newPlainText(label, value))
        Toast.makeText(this, "$label copied", Toast.LENGTH_SHORT).show()
    }

    private fun openUrl(url: String) {
        if (url.isBlank()) return
        runCatching {
            startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
        }.onFailure {
            Toast.makeText(this, "No app can open this link", Toast.LENGTH_SHORT).show()
        }
    }

    private fun openContentLink(classification: JSONObject) {
        val displayName = classification.optString("open_display_name").trim()
        val label = displayName.ifBlank { "file" }
        val localOpenUrl = classification.optString("local_open_url").trim()
        if (!classification.optBoolean("is_valid") || localOpenUrl.isBlank()) {
            val error = classification.optString("error").trim()
            Toast.makeText(
                this,
                error.ifBlank { "Could not open $label" },
                Toast.LENGTH_SHORT,
            ).show()
            return
        }
        val link = classification.optString("normalized_input").trim()
        AlertDialog.Builder(this)
            .setTitle("Open $label?")
            .setMessage("Open it now or save a copy to Iris Drive.")
            .setPositiveButton("Open") { _, _ ->
                Toast.makeText(this, "Opening $label", Toast.LENGTH_SHORT).show()
                openUrl(localOpenUrl)
            }
            .setNegativeButton("Save to Drive") { _, _ ->
                if (link.isBlank()) {
                    Toast.makeText(this, "Could not save $label", Toast.LENGTH_SHORT).show()
                    return@setNegativeButton
                }
                Toast.makeText(this, "Saving $label", Toast.LENGTH_SHORT).show()
                dispatch(NativeActions.importContentLink(link)) { state ->
                    val error = state.error.trim()
                    Toast.makeText(
                        this,
                        if (error.isBlank()) "Saved $label to Iris Drive" else error,
                        Toast.LENGTH_SHORT,
                    ).show()
                }
            }
            .setNeutralButton("Cancel", null)
            .show()
    }

    private fun openDriveFolder() {
        runCatching {
            startActivity(irisDriveFilesIntent(BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY))
        }.onFailure {
            Toast.makeText(this, "No app can open the drive folder", Toast.LENGTH_SHORT).show()
        }
    }

    private fun openProviderPath(path: String) {
        val normalized = normalizeProviderPath(path)
        runCatching {
            val documentId = if (normalized.isBlank()) {
                DOCUMENTS_ROOT_DOCUMENT_ID
            } else {
                "$DOCUMENTS_ROOT_DOCUMENT_ID/$normalized"
            }
            val uri = DocumentsContract.buildDocumentUri(
                BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY,
                documentId,
            )
            startActivity(
                Intent(Intent.ACTION_VIEW)
                    .setData(uri)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION),
            )
        }.onFailure {
            Toast.makeText(this, "No app can open this folder", Toast.LENGTH_SHORT).show()
        }
    }

    private fun createShareFromProviderPath(sourcePath: String, displayName: String) {
        val normalized = normalizeProviderPath(sourcePath)
        if (normalized.isBlank()) {
            Toast.makeText(this, "Share folder path required", Toast.LENGTH_SHORT).show()
            return
        }
        lifecycleScope.launch(Dispatchers.IO) {
            val result = resolveShareSourcePathForCreate(normalized)
            withContext(Dispatchers.Main) {
                val error = result.second
                if (error.isNotBlank()) {
                    Toast.makeText(this@MainActivity, error, Toast.LENGTH_SHORT).show()
                } else {
                    dispatch(NativeActions.createShare(result.first, displayName))
                }
            }
        }
    }

    private fun resolveShareSourcePathForCreate(sourcePath: String): Pair<String, String> {
        providerEntryKind(sourcePath)?.let { kind ->
            return if (kind == "directory") {
                sourcePath to ""
            } else {
                "" to "Share path must be a folder"
            }
        }
        val createdPath = defaultCreatedShareSourcePath(sourcePath)
        providerEntryKind(createdPath)?.let { kind ->
            return if (kind == "directory") {
                createdPath to ""
            } else {
                "" to "Share path must be a folder"
            }
        }
        val mkdirJson = runCatching {
            JSONObject(NativeCore.providerMkdirJson(filesDir.absolutePath, createdPath))
        }.getOrElse { error ->
            return "" to (error.message ?: "Creating share folder failed")
        }
        val mkdirError = mkdirJson.optString("error")
        if (mkdirError.isNotBlank()) {
            return "" to mkdirError
        }
        return mkdirJson.optString("path").ifBlank { createdPath } to ""
    }

    private fun providerEntryKind(path: String): String? =
        runCatching {
            val entries = JSONObject(NativeCore.providerListJson(filesDir.absolutePath))
                .optJSONArray("entries")
            if (entries == null) {
                null
            } else {
                var kind: String? = null
                for (index in 0 until entries.length()) {
                    val entry = entries.optJSONObject(index) ?: continue
                    if (entry.optString("path") == path) {
                        kind = entry.optString("kind")
                        break
                    }
                }
                kind
            }
        }.getOrNull()

    private fun defaultCreatedShareSourcePath(sourcePath: String): String =
        if (sourcePath == "Shared" || sourcePath.startsWith("Shared/")) {
            sourcePath
        } else {
            "Shared/$sourcePath"
        }

    private fun normalizeProviderPath(path: String): String =
        NativeCore.normalizedProviderPath(path).orEmpty()

    private fun stateFromJson(json: String): AppState =
        AppState.fromJson(json)

    private fun resolveDeviceLabel(label: String): String =
        label.trim().ifBlank { defaultDeviceLabel() }

    private fun defaultDeviceLabel(): String {
        val model = Build.MODEL.orEmpty().trim()
        val manufacturer = Build.MANUFACTURER.orEmpty().trim()
        val label = when {
            model.isBlank() -> "Android"
            manufacturer.isBlank() -> model
            model.startsWith(manufacturer, ignoreCase = true) -> model
            model.contains("Pixel", ignoreCase = true) -> model
            else -> "$manufacturer $model"
        }
        return label.replace(Regex("\\s+"), " ").takeIf { it.isNotBlank() } ?: "Android"
    }

    private fun needsNotificationPermission(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.POST_NOTIFICATIONS,
            ) != PackageManager.PERMISSION_GRANTED

    private fun handleLaunchIntent(intent: Intent?) {
        val uri = intent?.data
        if (uri != null) {
            val classification = NativeCore.classifyLinkInput(uri.toString())
            when (classification.optString("kind")) {
                "share_dialog" -> {
                    openShareDialog(
                        classification.optString("share_source_path"),
                        classification.optString("share_display_name"),
                        classification.optString("share_recipient_npub_hint"),
                        classification.optString("share_recipient_display_name"),
                        classification.optString("share_recipient_profile_id"),
                    )
                }

                "nhash_file", "mutable_file" -> {
                    openContentLink(classification)
                }

                "app_key_approval" -> {
                    dispatch(NativeActions.approveDevice(uri.toString(), defaultDeviceLabel()))
                }

                "invite" -> {
                    dispatch(NativeActions.linkDevice(uri.toString(), defaultDeviceLabel()))
                }
            }
        }
        when (intent?.getStringExtra(DEBUG_ACTION_EXTRA)) {
            "create-profile" -> dispatch(NativeActions.createProfile("Android smoke"))
            "link-device" -> {
                val owner = intent.getStringExtra(DEBUG_OWNER_EXTRA).orEmpty()
                dispatch(NativeActions.linkDevice(owner, "Android smoke"))
            }
            "approve-device" -> {
                val request = intent.getStringExtra(DEBUG_REQUEST_EXTRA).orEmpty()
                dispatch(NativeActions.approveDevice(request, "Android smoke"))
            }
            "add-root" -> dispatch(
                NativeActions.addRoot(
                    "Android smoke",
                    providerRootDocumentUri(),
                ),
            )
            "start-sync" -> dispatch(NativeActions.startSync())
            "dump-provider-list" -> writeDebugProviderList()
            "refresh" -> refresh()
        }
    }

    private fun applyDebugEnvironment(intent: Intent?) {
        if (!BuildConfig.DEBUG) return
        val extras = intent?.extras ?: return
        extras.keySet()
            .filter { it.startsWith(DEBUG_ENV_EXTRA_PREFIX) }
            .forEach { key ->
                val value = extras.getString(key) ?: return@forEach
                runCatching {
                    Os.setenv(key, value, true)
                }
            }
    }

    private fun writeDebugState(jsonText: String?) {
        if (!BuildConfig.DEBUG) return
        if (jsonText.isNullOrBlank()) return
        runCatching {
            File(filesDir, DEBUG_STATE_FILE).writeText(jsonText)
        }
    }

    private fun writeDebugProviderList() {
        if (!BuildConfig.DEBUG) return
        runCatching {
            File(filesDir, DEBUG_PROVIDER_LIST_FILE)
                .writeText(NativeCore.providerListJson(filesDir.absolutePath))
        }
    }

    private fun providerRootDocumentUri(): String =
        DocumentsContract.buildDocumentUri(
            BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY,
            DOCUMENTS_ROOT_DOCUMENT_ID,
        ).toString()

    private fun openShareDialog(
        sourcePath: String,
        displayName: String,
        recipientNpubHint: String = "",
        recipientDisplayName: String = "",
        recipientProfileId: String = "",
    ) {
        val trimmedPath = sourcePath.trim()
        if (trimmedPath.isBlank()) {
            Toast.makeText(this, "Share folder path required", Toast.LENGTH_SHORT).show()
            return
        }
        nextShareDialogRequestId += 1
        shareDialogFlow.value = ShareDialogRequest(
            id = nextShareDialogRequestId,
            sourcePath = trimmedPath,
            displayName = displayName.trim(),
            recipientNpubHint = recipientNpubHint.trim(),
            recipientDisplayName = recipientDisplayName.trim(),
            recipientProfileId = recipientProfileId.trim(),
        )
    }

    companion object {
        const val DEBUG_ACTION_EXTRA = "to.iris.drive.DEBUG_ACTION"
        const val DEBUG_OWNER_EXTRA = "to.iris.drive.DEBUG_OWNER"
        const val DEBUG_REQUEST_EXTRA = "to.iris.drive.DEBUG_REQUEST"
        const val DEBUG_STATE_FILE = "debug-state.json"
        const val DEBUG_PROVIDER_LIST_FILE = "debug-provider-list.json"
        private const val DEBUG_ENV_EXTRA_PREFIX = "IRIS_DRIVE_"
        private const val DOCUMENTS_ROOT_DOCUMENT_ID = "root"
    }
}
