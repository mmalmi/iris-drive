package to.iris.drive.app.core

import org.json.JSONArray
import org.json.JSONObject

internal data class AppState(
    val profile: ProfileState? = null,
    val roots: List<SyncRoot> = emptyList(),
    val devices: List<DeviceState> = emptyList(),
    val relays: List<String> = emptyList(),
    val relayStatuses: List<RelayStatus> = emptyList(),
    val backups: List<BackupState> = emptyList(),
    val paths: PathState = PathState(),
    val sync: SyncState = SyncState(),
    val fips: FipsState = FipsState(),
    val snapshotLink: String = "",
    val error: String = "",
    val setupState: String = "not_configured",
    val setupLabel: String = "Not linked",
    val primaryStatus: String = "not_setup",
    val primaryStatusLabel: String = "Ready",
    val isSetupComplete: Boolean = false,
    val isAwaitingApproval: Boolean = false,
    val isRevoked: Boolean = false,
    val authorizedDeviceCount: Int = 0,
    val onlineDeviceCount: Int = 0,
    val fileCount: Int = 0,
    val visibleFileBytes: Long = 0,
) {
    companion object {
        fun fromJson(jsonText: String): AppState {
            val json = runCatching { JSONObject(jsonText) }.getOrElse {
                return AppState(error = it.message ?: "invalid native state JSON")
            }
            val ui = json.optJSONObject("ui") ?: JSONObject()
            return AppState(
                profile = ui.optJSONObject("profile")?.toProfile(),
                roots = ui.optJSONArray("roots").toRoots(),
                devices = ui.optJSONArray("devices").toDevices(),
                relays = ui.optJSONArray("relays").toStrings(),
                relayStatuses = ui.optJSONArray("relay_statuses").toRelayStatuses(),
                backups = ui.optJSONArray("backups").toBackups(),
                paths = ui.optJSONObject("paths")?.toPaths() ?: PathState(),
                sync = ui.optJSONObject("sync")?.toSync() ?: SyncState(),
                fips = ui.optJSONObject("fips")?.toFips() ?: FipsState(),
                snapshotLink = ui.optString("snapshot_link"),
                error = json.optString("error"),
                setupState = ui.optString("setup_state", "not_configured"),
                setupLabel = ui.optString("setup_label", "Not linked"),
                primaryStatus = ui.optString("primary_status", "not_setup"),
                primaryStatusLabel = ui.optString("primary_status_label", "Ready"),
                isSetupComplete = ui.optBoolean("setup_complete"),
                isAwaitingApproval = ui.optBoolean("awaiting_approval"),
                isRevoked = ui.optBoolean("revoked"),
                authorizedDeviceCount = ui.optInt("authorized_device_count"),
                onlineDeviceCount = ui.optInt("online_device_count"),
                fileCount = ui.optInt("file_count"),
                visibleFileBytes = ui.optLong("visible_file_bytes"),
            )
        }
    }
}

internal data class ProfileState(
    val currentAppKeyNpub: String,
    val devicePubkey: String,
    val appKeyLabel: String,
    val authorizationState: String,
    val canAdminProfile: Boolean,
    val canExportRecoveryPhrase: Boolean = false,
    val appKeyLinkRequest: String,
    val appKeyLinkInvite: String,
    val inboundAppKeyLinkRequests: List<AppKeyLinkRequestState>,
)

internal data class RecoverySecretExport(
    val canExport: Boolean = false,
    val recoveryPhrase: String = "",
    val words: List<String> = emptyList(),
    val secretKey: String = "",
    val error: String = "",
)

internal data class AppKeyLinkRequestState(
    val devicePubkey: String,
    val label: String,
    val requestedAt: Long,
    val requestLink: String,
)

internal data class DeviceState(
    val pubkey: String,
    val label: String,
    val displayLabel: String,
    val role: String,
    val roleLabel: String,
    val state: String,
    val stateLabel: String,
    val detail: String,
    val isCurrentDevice: Boolean,
    val isOnline: Boolean,
    val connectionState: String,
    val connectionLabel: String,
    val canRevoke: Boolean,
    val canAppointAdmin: Boolean,
    val canDemoteAdmin: Boolean,
)

internal data class BackupState(
    val id: String,
    val kind: String,
    val target: String,
    val label: String,
    val configuredLabel: String,
    val state: String,
    val detail: String,
    val enabled: Boolean,
)

internal data class RelayStatus(
    val url: String,
    val status: String,
    val statusLabel: String,
    val health: String,
)

internal data class PathState(
    val dataDir: String = "",
    val configPath: String = "",
    val blocksDir: String = "",
)

internal data class SyncState(
    val running: Boolean = false,
    val status: String = "",
    val statusLabel: String = "Sync paused",
)

internal data class FipsState(
    val enabled: Boolean = false,
    val running: Boolean = false,
    val fresh: Boolean = false,
    val state: String = "paused",
    val stateLabel: String = "Paused",
    val endpointNpub: String = "",
    val discoveryScope: String = "",
    val rosterLabel: String = "0/0 online",
    val rosterPeerCount: Int = 0,
    val rosterOnlineDeviceCount: Int = 0,
    val rosterDirectDeviceCount: Int = 0,
    val onlineDeviceCount: Int = 0,
    val directDeviceCount: Int = 0,
    val meshDeviceCount: Int = 0,
    val otherPeerCount: Int = 0,
    val peerStatuses: List<FipsPeerStatus> = emptyList(),
    val error: String = "",
)

internal data class FipsPeerStatus(
    val npub: String,
    val transportType: String,
    val srttMs: Long?,
    val connectionLabel: String,
)

internal data class SyncRoot(
    val name: String,
    val localPath: String,
    val status: String,
)

internal object NativeActions {
    fun refresh(): String = JSONObject().put("type", "refresh").toString()

    fun createProfile(deviceLabel: String): String =
        JSONObject()
            .put("type", "create_profile")
            .put("app_key_label", deviceLabel)
            .toString()

    fun restoreProfile(recoverySecret: String, deviceLabel: String): String =
        JSONObject()
            .put("type", "restore_profile")
            .put("recovery_secret", recoverySecret)
            .put("app_key_label", deviceLabel)
            .toString()

    fun linkDevice(linkTarget: String, deviceLabel: String): String =
        JSONObject()
            .put("type", "link_device")
            .put("link_target", linkTarget)
            .put("app_key_label", deviceLabel)
            .toString()

    fun logout(): String = JSONObject().put("type", "logout").toString()

    fun approveDevice(request: String, label: String): String =
        JSONObject()
            .put("type", "approve_device")
            .put("request", request)
            .put("label", label)
            .toString()

    fun rejectDevice(request: String): String =
        JSONObject()
            .put("type", "reject_device")
            .put("request", request)
            .toString()

    fun resetInvite(): String = JSONObject().put("type", "reset_invite").toString()

    fun revokeDevice(devicePubkey: String): String =
        JSONObject()
            .put("type", "revoke_device")
            .put("app_key_pubkey", devicePubkey)
            .toString()

    fun deleteDevice(devicePubkey: String): String =
        JSONObject()
            .put("type", "delete_device")
            .put("app_key_pubkey", devicePubkey)
            .toString()

    fun appointAdmin(devicePubkey: String): String =
        JSONObject()
            .put("type", "appoint_admin")
            .put("app_key_pubkey", devicePubkey)
            .toString()

    fun demoteAdmin(devicePubkey: String): String =
        JSONObject()
            .put("type", "demote_admin")
            .put("app_key_pubkey", devicePubkey)
            .toString()

    fun addRelay(url: String): String =
        JSONObject()
            .put("type", "add_relay")
            .put("url", url)
            .toString()

    fun removeRelay(url: String): String =
        JSONObject()
            .put("type", "remove_relay")
            .put("url", url)
            .toString()

    fun resetRelays(): String = JSONObject().put("type", "reset_relays").toString()

    fun addBackupTarget(target: String, label: String): String =
        JSONObject()
            .put("type", "add_backup_target")
            .put("target", target)
            .put("label", label)
            .toString()

    fun removeBackupTarget(target: String): String =
        JSONObject()
            .put("type", "remove_backup_target")
            .put("target", target)
            .toString()

    fun addBlossomServer(url: String): String =
        JSONObject()
            .put("type", "add_blossom_server")
            .put("url", url)
            .toString()

    fun removeBlossomServer(url: String): String =
        JSONObject()
            .put("type", "remove_blossom_server")
            .put("url", url)
            .toString()

    fun syncBackups(target: String = ""): String =
        JSONObject()
            .put("type", "sync_backups")
            .put("target", target)
            .toString()

    fun checkBackups(target: String = ""): String =
        JSONObject()
            .put("type", "check_backups")
            .put("target", target)
            .toString()

    fun startSync(): String = JSONObject().put("type", "start_sync").toString()

    fun stopSync(): String = JSONObject().put("type", "stop_sync").toString()

    fun restartSync(): String = JSONObject().put("type", "restart_sync").toString()

    fun addRoot(name: String, localPath: String): String =
        JSONObject()
            .put("type", "add_root")
            .put("name", name)
            .put("local_path", localPath)
            .toString()

    fun removeRoot(name: String): String =
        JSONObject()
            .put("type", "remove_root")
            .put("name", name)
            .toString()
}

private fun JSONObject.toProfile(): ProfileState =
    ProfileState(
        currentAppKeyNpub = optString("current_app_key_npub"),
        devicePubkey = optString("current_app_key_npub"),
        appKeyLabel = optString("app_key_label"),
        authorizationState = optString("authorization_state"),
        canAdminProfile = optBoolean("can_admin_profile"),
        canExportRecoveryPhrase = optBoolean("can_export_recovery_phrase"),
        appKeyLinkRequest = optString("app_key_link_request"),
        appKeyLinkInvite = optString("app_key_link_invite"),
        inboundAppKeyLinkRequests = optJSONArray("inbound_app_key_link_requests").toAppKeyLinkRequests(),
    )

internal fun recoverySecretExportFromJson(text: String): RecoverySecretExport =
    runCatching {
        val json = JSONObject(text)
        RecoverySecretExport(
            canExport = json.optBoolean("can_export"),
            recoveryPhrase = json.optString("recovery_phrase"),
            words = json.optJSONArray("words").toStringList(),
            secretKey = json.optString("secret_key"),
            error = json.optString("error"),
        )
    }.getOrElse { error ->
        RecoverySecretExport(error = "invalid recovery export JSON: ${error.message}")
    }

private fun JSONArray?.toStringList(): List<String> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            optString(index).takeIf { it.isNotBlank() }?.let(::add)
        }
    }
}

private fun JSONArray?.toAppKeyLinkRequests(): List<AppKeyLinkRequestState> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(
                AppKeyLinkRequestState(
                    devicePubkey = item.optString("app_key_pubkey"),
                    label = item.optString("label"),
                    requestedAt = item.optLong("requested_at"),
                    requestLink = item.optString("request_link"),
                ),
            )
        }
    }
}

private fun JSONObject.toPaths(): PathState =
    PathState(
        dataDir = optString("data_dir"),
        configPath = optString("config_path"),
        blocksDir = optString("blocks_dir"),
    )

private fun JSONObject.toSync(): SyncState =
    SyncState(
        running = optBoolean("running"),
        status = optString("status"),
        statusLabel = optString("status_label", "Sync paused"),
    )

private fun JSONObject.toFips(): FipsState =
    FipsState(
        enabled = optBoolean("enabled"),
        running = optBoolean("running"),
        fresh = optBoolean("fresh"),
        state = optString("state", "paused"),
        stateLabel = optString("state_label", "Paused"),
        endpointNpub = optString("endpoint_npub"),
        discoveryScope = optString("discovery_scope"),
        rosterLabel = optString("roster_label", "0/0 online"),
        rosterPeerCount = optInt("roster_peer_count"),
        rosterOnlineDeviceCount = optInt("roster_online_device_count"),
        rosterDirectDeviceCount = optInt("roster_direct_device_count"),
        onlineDeviceCount = optInt("online_device_count"),
        directDeviceCount = optInt("direct_device_count"),
        meshDeviceCount = optInt("mesh_device_count"),
        otherPeerCount = optInt("other_peer_count"),
        peerStatuses = optJSONArray("peer_statuses").toFipsPeerStatuses(),
        error = optString("error"),
    )

private fun JSONArray?.toRoots(): List<SyncRoot> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(
                SyncRoot(
                    name = item.optString("name"),
                    localPath = item.optString("local_path"),
                    status = item.optString("status"),
                ),
            )
        }
    }
}

private fun JSONArray?.toDevices(): List<DeviceState> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            val isCurrentDevice = item.optBoolean("is_current_device")
            val isOnline = item.optBoolean("is_online")
            add(
                DeviceState(
                    pubkey = item.optString("pubkey"),
                    label = item.optString("label"),
                    displayLabel = item.optString("display_label"),
                    role = item.optString("role"),
                    roleLabel = item.optString("role_label"),
                    state = item.optString("state"),
                    stateLabel = item.optString("state_label"),
                    detail = item.optString("detail"),
                    isCurrentDevice = isCurrentDevice,
                    isOnline = isOnline,
                    connectionState = item.optString("connection_state"),
                    connectionLabel = item.optString("connection_label"),
                    canRevoke = item.optBoolean("can_revoke"),
                    canAppointAdmin = item.optBoolean("can_appoint_admin"),
                    canDemoteAdmin = item.optBoolean("can_demote_admin"),
                ),
            )
        }
    }
}

private fun JSONArray?.toBackups(): List<BackupState> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(
                BackupState(
                    id = item.optString("id"),
                    kind = item.optString("kind"),
                    target = item.optString("target"),
                    label = item.optString("label"),
                    configuredLabel = item.optString("configured_label"),
                    state = item.optString("state"),
                    detail = item.optString("detail"),
                    enabled = item.optBoolean("enabled", true),
                ),
            )
        }
    }
}

private fun JSONArray?.toRelayStatuses(): List<RelayStatus> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            val url = item.optString("url")
            if (url.isBlank()) continue
            add(
                RelayStatus(
                    url = url,
                    status = item.optString("status"),
                    statusLabel = item.optString("status_label"),
                    health = item.optString("health"),
                ),
            )
        }
    }
}

private fun JSONArray?.toFipsPeerStatuses(): List<FipsPeerStatus> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            val npub = item.optString("npub")
            if (npub.isBlank()) continue
            add(
                FipsPeerStatus(
                    npub = npub,
                    transportType = item.optString("transport_type"),
                    srttMs = item.opt("srtt_ms")?.let { (it as? Number)?.toLong() },
                    connectionLabel = item.optString("connection_label", "Online"),
                ),
            )
        }
    }
}

private fun JSONArray?.toStrings(): List<String> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val value = optString(index)
            if (value.isNotBlank()) add(value)
        }
    }
}
