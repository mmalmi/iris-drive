use serde::{Deserialize, Serialize};

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiState {
    pub roots: Vec<UiSyncRoot>,
    pub account: Option<UiAccount>,
    pub devices: Vec<UiDevice>,
    pub relays: Vec<String>,
    pub backups: Vec<UiBackup>,
    pub paths: UiPaths,
    pub sync: UiSyncStatus,
    pub snapshot_link: String,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiSyncRoot {
    pub name: String,
    pub local_path: String,
    pub status: String,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiAccount {
    pub owner_pubkey: String,
    pub device_pubkey: String,
    pub device_label: String,
    pub authorization_state: String,
    pub has_owner_signing_authority: bool,
    pub device_link_request: String,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiDevice {
    pub pubkey: String,
    pub label: String,
    pub state: String,
    pub detail: String,
    pub is_online: bool,
    pub can_revoke: bool,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiBackup {
    pub label: String,
    pub state: String,
    pub detail: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiPaths {
    pub data_dir: String,
    pub config_path: String,
    pub blocks_dir: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiSyncStatus {
    pub running: bool,
    pub status: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeAppState {
    pub ui: UiState,
    pub error: String,
}
