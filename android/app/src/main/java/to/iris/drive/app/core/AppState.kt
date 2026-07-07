package to.iris.drive.app.core

import org.json.JSONArray
import org.json.JSONObject

internal data class AppState(
    val isLoaded: Boolean = true,
    val profile: ProfileState? = null,
    val roots: List<SyncRoot> = emptyList(),
    val shares: List<ShareState> = emptyList(),
    val devices: List<DeviceState> = emptyList(),
    val relays: List<String> = emptyList(),
    val relayStatuses: List<RelayStatus> = emptyList(),
    val backups: List<BackupState> = emptyList(),
    val paths: PathState = PathState(),
    val sync: SyncState = SyncState(),
    val fips: FipsState = FipsState(),
    val snapshotLink: String = "",
    val localNhashResolverEnabled: Boolean = true,
    val sitesPortalUrl: String = "",
    val lastShareInvite: String = "",
    val lastShareRecipientEvidence: String = "",
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
                isLoaded = true,
                profile = ui.optJSONObject("profile")?.toProfile(),
                roots = ui.optJSONArray("roots").toRoots(),
                shares = ui.optJSONArray("shares").toShares(),
                devices = ui.optJSONArray("app_actors").toDevices(),
                relays = ui.optJSONArray("relays").toStrings(),
                relayStatuses = ui.optJSONArray("relay_statuses").toRelayStatuses(),
                backups = ui.optJSONArray("backups").toBackups(),
                paths = ui.optJSONObject("paths")?.toPaths() ?: PathState(),
                sync = ui.optJSONObject("sync")?.toSync() ?: SyncState(),
                fips = ui.optJSONObject("fips")?.toFips() ?: FipsState(),
                snapshotLink = ui.optString("snapshot_link"),
                localNhashResolverEnabled = ui.optBoolean("local_nhash_resolver_enabled", true),
                sitesPortalUrl = ui.optString("sites_portal_url"),
                lastShareInvite = ui.optString("last_share_invite"),
                lastShareRecipientEvidence = ui.optString("last_share_recipient_evidence"),
                error = json.optString("error"),
                setupState = ui.optString("setup_state", "not_configured"),
                setupLabel = ui.optString("setup_label", "Not linked"),
                primaryStatus = ui.optString("primary_status", "not_setup"),
                primaryStatusLabel = ui.optString("primary_status_label", "Ready"),
                isSetupComplete = ui.optBoolean("setup_complete"),
                isAwaitingApproval = ui.optBoolean("awaiting_approval"),
                isRevoked = ui.optBoolean("revoked"),
                authorizedDeviceCount = ui.optInt("authorized_app_key_count"),
                onlineDeviceCount = ui.optInt("online_app_key_count"),
                fileCount = ui.optInt("file_count"),
                visibleFileBytes = ui.optLong("visible_file_bytes"),
            )
        }
    }
}

internal data class ProfileState(
    val profileId: String,
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
    val actorKind: String = "device",
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

internal data class ShareState(
    val shareId: String,
    val displayName: String,
    val sourcePath: String,
    val sharedWithMePath: String,
    val role: String,
    val roleLabel: String,
    val keyStatus: String,
    val keyStatusLabel: String,
    val writeAuthorization: String,
    val writeAuthorizationLabel: String,
    val canWrite: Boolean,
    val canAdmin: Boolean,
    val currentKeyEpoch: Long?,
    val hasCurrentKeyWrap: Boolean,
    val keyUnavailable: Boolean,
    val repairNeeded: Boolean,
    val missingKeyWraps: List<String>,
    val participantCount: Int,
    val appKeyCount: Int,
    val members: List<ShareMemberState>,
    val pendingInvites: List<PendingShareInviteState> = emptyList(),
    val shortcutPaths: List<String>,
)

internal data class PendingShareInviteState(
    val representativeNpubHint: String,
    val displayName: String,
    val role: String,
    val roleLabel: String,
    val status: String,
    val statusLabel: String,
)

internal data class ShareMemberState(
    val profileId: String,
    val displayName: String,
    val representativeNpubHint: String,
    val role: String,
    val roleLabel: String,
    val status: String,
    val statusLabel: String,
    val appKeyCount: Int,
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
    val status: String = "ready",
    val statusLabel: String = "Ready",
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

    fun refreshProfile(): String = JSONObject().put("type", "refresh_profile").toString()

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

    fun startJoinRequest(deviceLabel: String): String =
        JSONObject()
            .put("type", "start_join_request")
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

    fun addRecoveryDevice(recoveryPubkey: String): String =
        JSONObject()
            .put("type", "add_recovery_device")
            .put("recovery_pubkey", recoveryPubkey)
            .toString()

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

    fun renameDevice(devicePubkey: String, label: String): String =
        JSONObject()
            .put("type", "rename_device")
            .put("app_key_pubkey", devicePubkey)
            .put("label", label)
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

    fun createShare(sourcePath: String, displayName: String): String =
        JSONObject()
            .put("type", "create_share")
            .put("source_path", sourcePath)
            .put("display_name", displayName)
            .toString()

    fun deleteShare(shareId: String): String =
        JSONObject()
            .put("type", "delete_share")
            .put("share_id", shareId)
            .toString()

    fun inviteShareMember(
        shareId: String,
        profileId: String,
        appKey: String,
        role: String,
        representativeNpubHint: String,
        displayName: String,
        label: String,
    ): String =
        JSONObject()
            .put("type", "invite_share_member")
            .put("share_id", shareId)
            .put("profile_id", profileId)
            .put("app_key", appKey)
            .put("role", role)
            .put("representative_npub_hint", representativeNpubHint)
            .put("display_name", displayName)
            .put("label", label)
            .toString()

    fun inviteShareMemberFromEvidence(
        shareId: String,
        evidenceJson: String,
        role: String,
        displayName: String,
    ): String =
        JSONObject()
            .put("type", "invite_share_member_from_evidence")
            .put("share_id", shareId)
            .put("evidence_json", evidenceJson)
            .put("role", role)
            .put("display_name", displayName)
            .toString()

    fun exportShareRecipientEvidence(displayName: String): String =
        JSONObject()
            .put("type", "export_share_recipient_evidence")
            .put("display_name", displayName)
            .toString()

    fun recordPendingShareInvite(
        shareId: String,
        representativeNpubHint: String,
        role: String,
        displayName: String,
    ): String =
        JSONObject()
            .put("type", "record_pending_share_invite")
            .put("share_id", shareId)
            .put("representative_npub_hint", representativeNpubHint)
            .put("role", role)
            .put("display_name", displayName)
            .toString()

    fun acceptShareInvite(invite: String): String =
        JSONObject()
            .put("type", "accept_share_invite")
            .put("invite", invite)
            .toString()

    fun revokeShareMember(shareId: String, profileId: String): String =
        JSONObject()
            .put("type", "revoke_share_member")
            .put("share_id", shareId)
            .put("profile_id", profileId)
            .put("reason", "")
            .toString()

    fun addShareShortcut(shareId: String, path: String): String =
        JSONObject()
            .put("type", "add_share_shortcut")
            .put("share_id", shareId)
            .put("path", "")
            .put("parent", "")
            .put("target_path", "")
            .toString()

    fun repairShareWraps(shareId: String): String =
        JSONObject()
            .put("type", "repair_share_wraps")
            .put("share_id", shareId)
            .toString()

    fun importContentLink(link: String): String =
        JSONObject()
            .put("type", "import_content_link")
            .put("link", link)
            .toString()
}

private fun JSONObject.toProfile(): ProfileState =
    ProfileState(
        profileId = optString("profile_id"),
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
        status = optString("status", "ready"),
        statusLabel = optString("status_label", "Ready"),
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
            val isCurrentDevice = item.optBoolean("is_current_app_key")
            val isOnline = item.optBoolean("is_online")
            val role = item.optString("role")
            val connectionState = item.optString("connection_state")
            add(
                DeviceState(
                    pubkey = item.optString("pubkey"),
                    label = item.optString("label"),
                    displayLabel = item.optString("display_label"),
                    role = role,
                    roleLabel = item.optString("role_label"),
                    state = item.optString("state"),
                    stateLabel = item.optString("state_label"),
                    detail = item.optString("detail"),
                    isCurrentDevice = isCurrentDevice,
                    isOnline = isOnline,
                    connectionState = connectionState,
                    connectionLabel = item.optString("connection_label"),
                    canRevoke = item.optBoolean("can_revoke"),
                    canAppointAdmin = item.optBoolean("can_appoint_admin"),
                    canDemoteAdmin = item.optBoolean("can_demote_admin"),
                    actorKind = item.optString(
                        "actor_kind",
                        if (role == "recovery" || connectionState == "recovery") {
                            "recovery_key"
                        } else {
                            "device"
                        },
                    ),
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

private fun JSONArray?.toShares(): List<ShareState> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(
                ShareState(
                    shareId = item.optString("share_id"),
                    displayName = item.optString("display_name"),
                    sourcePath = item.optString("source_path"),
                    sharedWithMePath = item.optString("shared_with_me_path"),
                    role = item.optString("role"),
                    roleLabel = item.optString("role_label"),
                    keyStatus = item.optString("key_status"),
                    keyStatusLabel = item.optString("key_status_label"),
                    writeAuthorization = item.optString("write_authorization"),
                    writeAuthorizationLabel = item.optString("write_authorization_label"),
                    canWrite = item.optBoolean("can_write"),
                    canAdmin = item.optBoolean("can_admin"),
                    currentKeyEpoch = item.opt("current_key_epoch")?.let { (it as? Number)?.toLong() },
                    hasCurrentKeyWrap = item.optBoolean("has_current_key_wrap"),
                    keyUnavailable = item.optBoolean("key_unavailable"),
                    repairNeeded = item.optBoolean("repair_needed"),
                    missingKeyWraps = item.optJSONArray("missing_key_wraps").toStringList(),
                    participantCount = item.optInt("participant_count"),
                    appKeyCount = item.optInt("app_key_count"),
                    members = item.optJSONArray("members").toShareMembers(),
                    pendingInvites = item.optJSONArray("pending_invites").toPendingShareInvites(),
                    shortcutPaths = item.optJSONArray("shortcut_paths").toStringList(),
                ),
            )
        }
    }
}

private fun JSONArray?.toShareMembers(): List<ShareMemberState> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(
                ShareMemberState(
                    profileId = item.optString("profile_id"),
                    displayName = item.optString("display_name"),
                    representativeNpubHint = item.optString("representative_npub_hint"),
                    role = item.optString("role"),
                    roleLabel = item.optString("role_label"),
                    status = item.optString("status"),
                    statusLabel = item.optString("status_label"),
                    appKeyCount = item.optInt("app_key_count"),
                ),
            )
        }
    }
}

private fun JSONArray?.toPendingShareInvites(): List<PendingShareInviteState> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(
                PendingShareInviteState(
                    representativeNpubHint = item.optString("representative_npub_hint"),
                    displayName = item.optString("display_name"),
                    role = item.optString("role"),
                    roleLabel = item.optString("role_label"),
                    status = item.optString("status"),
                    statusLabel = item.optString("status_label"),
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
