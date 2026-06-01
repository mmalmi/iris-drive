package to.iris.drive.app.core

import org.json.JSONArray
import org.json.JSONObject

internal data class AppState(
    val account: AccountState? = null,
    val roots: List<SyncRoot> = emptyList(),
    val devices: List<DeviceState> = emptyList(),
    val relays: List<String> = emptyList(),
    val relayStatuses: List<RelayStatus> = emptyList(),
    val backups: List<BackupState> = emptyList(),
    val paths: PathState = PathState(),
    val sync: SyncState = SyncState(),
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
                account = ui.optJSONObject("account")?.toAccount(),
                roots = ui.optJSONArray("roots").toRoots(),
                devices = ui.optJSONArray("devices").toDevices(),
                relays = ui.optJSONArray("relays").toStrings(),
                relayStatuses = ui.optJSONArray("relay_statuses").toRelayStatuses(),
                backups = ui.optJSONArray("backups").toBackups(),
                paths = ui.optJSONObject("paths")?.toPaths() ?: PathState(),
                sync = ui.optJSONObject("sync")?.toSync() ?: SyncState(),
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

internal data class AccountState(
    val ownerPubkey: String,
    val devicePubkey: String,
    val deviceLabel: String,
    val authorizationState: String,
    val hasOwnerSigningAuthority: Boolean,
    val deviceLinkRequest: String,
    val deviceLinkInvite: String,
    val inboundDeviceLinkRequests: List<DeviceLinkRequestState>,
)

internal data class DeviceLinkRequestState(
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
    val label: String,
    val state: String,
    val detail: String,
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
            .put("device_label", deviceLabel)
            .toString()

    fun restoreProfile(secret: String, deviceLabel: String): String =
        JSONObject()
            .put("type", "restore_profile")
            .put("secret", secret)
            .put("device_label", deviceLabel)
            .toString()

    fun linkDevice(ownerPubkey: String, deviceLabel: String): String =
        JSONObject()
            .put("type", "link_device")
            .put("owner_pubkey", ownerPubkey)
            .put("device_label", deviceLabel)
            .toString()

    fun logout(): String = JSONObject().put("type", "logout").toString()

    fun approveDevice(request: String, label: String): String =
        JSONObject()
            .put("type", "approve_device")
            .put("request", request)
            .put("label", label)
            .toString()

    fun resetInvite(): String = JSONObject().put("type", "reset_invite").toString()

    fun revokeDevice(devicePubkey: String): String =
        JSONObject()
            .put("type", "revoke_device")
            .put("device_pubkey", devicePubkey)
            .toString()

    fun deleteDevice(devicePubkey: String): String =
        JSONObject()
            .put("type", "delete_device")
            .put("device_pubkey", devicePubkey)
            .toString()

    fun appointAdmin(devicePubkey: String): String =
        JSONObject()
            .put("type", "appoint_admin")
            .put("device_pubkey", devicePubkey)
            .toString()

    fun demoteAdmin(devicePubkey: String): String =
        JSONObject()
            .put("type", "demote_admin")
            .put("device_pubkey", devicePubkey)
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

private fun JSONObject.toAccount(): AccountState =
    AccountState(
        ownerPubkey = optString("owner_pubkey"),
        devicePubkey = optString("device_pubkey"),
        deviceLabel = optString("device_label"),
        authorizationState = optString("authorization_state"),
        hasOwnerSigningAuthority = optBoolean("has_owner_signing_authority"),
        deviceLinkRequest = optString("device_link_request"),
        deviceLinkInvite = optString("device_link_invite"),
        inboundDeviceLinkRequests = optJSONArray("inbound_device_link_requests").toDeviceLinkRequests(),
    )

private fun JSONArray?.toDeviceLinkRequests(): List<DeviceLinkRequestState> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(
                DeviceLinkRequestState(
                    devicePubkey = item.optString("device_pubkey"),
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
                    label = item.optString("label"),
                    state = item.optString("state"),
                    detail = item.optString("detail"),
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

private fun JSONArray?.toStrings(): List<String> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val value = optString(index)
            if (value.isNotBlank()) add(value)
        }
    }
}
