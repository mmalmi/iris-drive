use serde::{Deserialize, Serialize};

#[derive(uniffi::Enum, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NativeAppAction {
    Refresh,
    CreateProfile {
        device_label: String,
    },
    RestoreProfile {
        recovery_secret: String,
        device_label: String,
    },
    AdmitAppKeyWithRecoveryPhrase {
        recovery_phrase: String,
        label: String,
    },
    LinkDevice {
        link_target: String,
        device_label: String,
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
        device_pubkey: String,
    },
    AppointAdmin {
        device_pubkey: String,
    },
    DemoteAdmin {
        device_pubkey: String,
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
    ImportFile {
        display_name: String,
        source_path: String,
    },
}
