use serde::{Deserialize, Serialize};

#[derive(uniffi::Enum, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NativeAppAction {
    Refresh,
    CreateProfile {
        app_key_label: String,
    },
    RestoreProfile {
        recovery_secret: String,
        app_key_label: String,
    },
    AdmitAppKeyWithRecoveryPhrase {
        recovery_phrase: String,
        label: String,
    },
    AddRecoveryDevice {
        recovery_pubkey: String,
    },
    LinkDevice {
        link_target: String,
        app_key_label: String,
    },
    Logout,
    ApproveDevice {
        request: String,
        label: String,
    },
    RejectDevice {
        request: String,
    },
    ResetInvite,
    #[serde(alias = "delete_device")]
    RevokeDevice {
        app_key_pubkey: String,
    },
    AppointAdmin {
        app_key_pubkey: String,
    },
    DemoteAdmin {
        app_key_pubkey: String,
    },
    AddRelay {
        url: String,
    },
    RemoveRelay {
        url: String,
    },
    ResetRelays,
    AddBackupTarget {
        target: String,
        label: String,
    },
    RemoveBackupTarget {
        target: String,
    },
    AddBlossomServer {
        url: String,
    },
    RemoveBlossomServer {
        url: String,
    },
    SetLaunchOnStartup {
        enabled: bool,
    },
    SyncBackups {
        target: String,
    },
    CheckBackups {
        target: String,
    },
    StartSync,
    StopSync,
    RestartSync,
    AddRoot {
        name: String,
        local_path: String,
    },
    RemoveRoot {
        name: String,
    },
    CreateShare {
        source_path: String,
        display_name: String,
    },
    DeleteShare {
        share_id: String,
    },
    InviteShareMember {
        share_id: String,
        profile_id: String,
        app_key: String,
        role: String,
        representative_npub_hint: String,
        display_name: String,
        label: String,
    },
    InviteShareMemberFromEvidence {
        share_id: String,
        evidence_json: String,
        role: String,
        display_name: String,
    },
    RecordPendingShareInvite {
        share_id: String,
        representative_npub_hint: String,
        role: String,
        display_name: String,
    },
    ExportShareRecipientEvidence {
        display_name: String,
    },
    AcceptShareInvite {
        invite: String,
    },
    RevokeShareMember {
        share_id: String,
        profile_id: String,
        reason: String,
    },
    SetShareMemberRole {
        share_id: String,
        profile_id: String,
        role: String,
    },
    AddShareShortcut {
        share_id: String,
        path: String,
        parent: String,
        target_path: String,
    },
    RepairShareWraps {
        share_id: String,
    },
    ImportFile {
        display_name: String,
        source_path: String,
    },
    ImportContentLink {
        link: String,
    },
}
