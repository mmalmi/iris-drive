use serde::{Deserialize, Serialize};

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiState {
    pub roots: Vec<UiSyncRoot>,
    pub shares: Vec<UiShare>,
    pub profile: Option<UiProfile>,
    pub app_actors: Vec<UiAppActor>,
    pub relays: Vec<String>,
    pub relay_statuses: Vec<UiRelayStatus>,
    pub backups: Vec<UiBackup>,
    pub paths: UiPaths,
    pub sync: UiSyncStatus,
    pub fips: UiFipsStatus,
    pub setup_state: String,
    pub setup_complete: bool,
    pub awaiting_approval: bool,
    pub revoked: bool,
    pub setup_label: String,
    pub primary_status: String,
    pub primary_status_label: String,
    pub authorized_app_key_count: u64,
    pub online_app_key_count: u64,
    pub file_count: u64,
    pub visible_file_bytes: u64,
    pub provider_change_key: String,
    pub provider_directory_paths: Vec<String>,
    pub snapshot_link: String,
    pub last_share_invite: String,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiSyncRoot {
    pub name: String,
    pub local_path: String,
    pub status: String,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct UiShare {
    pub share_id: String,
    pub display_name: String,
    pub shared_with_me_path: String,
    pub role: String,
    pub role_label: String,
    pub key_status: String,
    pub key_status_label: String,
    pub write_authorization: String,
    pub write_authorization_label: String,
    pub can_write: bool,
    pub can_admin: bool,
    pub current_key_epoch: Option<u64>,
    pub has_current_key_wrap: bool,
    pub key_unavailable: bool,
    pub repair_needed: bool,
    pub missing_key_wrap_count: u64,
    pub participant_count: u64,
    pub app_key_count: u64,
    pub members: Vec<UiShareMember>,
    pub shortcut_paths: Vec<String>,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiShareMember {
    pub profile_id: String,
    pub display_name: String,
    pub representative_npub_hint: String,
    pub role: String,
    pub role_label: String,
    pub status: String,
    pub status_label: String,
    pub app_key_count: u64,
    pub can_revoke: bool,
    pub can_change_role: bool,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct UiProfile {
    pub profile_id: String,
    pub current_app_key_pubkey: String,
    pub current_app_key_npub: String,
    pub current_app_key_label: String,
    pub app_key_label: String,
    pub authorization_state: String,
    pub can_admin_profile: bool,
    pub can_write_roots: bool,
    pub active_app_key_count: u64,
    pub profile_roster_op_count: u64,
    pub current_key_epoch: Option<u64>,
    pub recovery_phrase_facet_count: u64,
    pub nip46_facet_count: u64,
    pub social_profile_facet_count: u64,
    pub missing_key_wraps: Vec<String>,
    pub can_export_recovery_phrase: bool,
    pub app_key_link_request: String,
    pub app_key_link_invite: String,
    pub inbound_app_key_link_requests: Vec<UiAppKeyLinkRequest>,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiAppKeyLinkRequest {
    pub app_key_pubkey: String,
    pub label: String,
    pub requested_at: u64,
    pub request_link: String,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct UiAppActor {
    pub pubkey: String,
    pub label: String,
    pub display_label: String,
    pub role: String,
    pub role_label: String,
    pub state: String,
    pub state_label: String,
    pub connection_state: String,
    pub connection_label: String,
    pub detail: String,
    pub is_current_app_key: bool,
    pub is_online: bool,
    pub can_revoke: bool,
    pub can_appoint_admin: bool,
    pub can_demote_admin: bool,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiBackup {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub label: String,
    pub configured_label: String,
    pub state: String,
    pub detail: String,
    pub enabled: bool,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiRelayStatus {
    pub url: String,
    pub status: String,
    pub status_label: String,
    pub health: String,
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
    pub status_label: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiFipsStatus {
    pub enabled: bool,
    pub running: bool,
    pub fresh: bool,
    pub state: String,
    pub state_label: String,
    pub endpoint_npub: String,
    pub discovery_scope: String,
    pub roster_label: String,
    pub roster_peer_count: u64,
    pub roster_online_device_count: u64,
    pub roster_direct_device_count: u64,
    pub online_device_count: u64,
    pub direct_device_count: u64,
    pub mesh_device_count: u64,
    pub other_peer_count: u64,
    pub online_devices: Vec<String>,
    pub direct_devices: Vec<String>,
    pub mesh_devices: Vec<String>,
    pub peer_statuses: Vec<UiFipsPeerStatus>,
    pub error: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiFipsPeerStatus {
    pub npub: String,
    pub transport_type: String,
    pub srtt_ms: Option<u64>,
    pub connection_label: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeAppState {
    pub ui: UiState,
    pub error: String,
}
