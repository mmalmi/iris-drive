use serde::{Deserialize, Serialize};

#[derive(uniffi::Enum, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NativeAppAction {
    Refresh,
    CreateProfile {
        device_label: String,
    },
    RestoreProfile {
        secret: String,
        device_label: String,
    },
    LinkDevice {
        owner_pubkey: String,
        device_label: String,
    },
    ApproveDevice {
        request: String,
        label: String,
    },
    RevokeDevice {
        device_pubkey: String,
    },
    AddRelay {
        url: String,
    },
    RemoveRelay {
        url: String,
    },
    ResetRelays,
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
}
