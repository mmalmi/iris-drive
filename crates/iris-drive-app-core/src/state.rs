use serde::{Deserialize, Serialize};

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiState {
    pub roots: Vec<UiSyncRoot>,
    pub account: Option<UiAccount>,
    pub devices: Vec<UiDevice>,
    pub relays: Vec<String>,
    pub relay_statuses: Vec<UiRelayStatus>,
    pub backups: Vec<UiBackup>,
    pub paths: UiPaths,
    pub sync: UiSyncStatus,
    pub fips: UiFipsStatus,
    pub setup_state: String,
    pub primary_status: String,
    pub authorized_device_count: u64,
    pub online_device_count: u64,
    pub file_count: u64,
    pub visible_file_bytes: u64,
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
    pub device_link_invite: String,
    pub inbound_device_link_requests: Vec<UiDeviceLinkRequest>,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiDeviceLinkRequest {
    pub device_pubkey: String,
    pub label: String,
    pub requested_at: u64,
    pub request_link: String,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiDevice {
    pub pubkey: String,
    pub label: String,
    pub role: String,
    pub state: String,
    pub detail: String,
    pub is_current_device: bool,
    pub is_online: bool,
    pub can_revoke: bool,
    pub can_appoint_admin: bool,
    pub can_demote_admin: bool,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiBackup {
    pub label: String,
    pub state: String,
    pub detail: String,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiRelayStatus {
    pub url: String,
    pub status: String,
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
pub struct UiFipsStatus {
    pub enabled: bool,
    pub running: bool,
    pub fresh: bool,
    pub endpoint_npub: String,
    pub online_device_count: u64,
    pub direct_device_count: u64,
    pub mesh_device_count: u64,
    pub online_devices: Vec<String>,
    pub direct_devices: Vec<String>,
    pub mesh_devices: Vec<String>,
    pub error: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeAppState {
    pub ui: UiState,
    pub error: String,
}
