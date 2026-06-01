use std::collections::{BTreeMap, BTreeSet};

use crate::account::DeviceAuthorizationState;
use crate::app_keys::{DeviceEntry, DeviceRole};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;

#[must_use]
pub fn authorization_state_key(state: DeviceAuthorizationState) -> &'static str {
    match state {
        DeviceAuthorizationState::Authorized => "authorized",
        DeviceAuthorizationState::AwaitingApproval => "awaiting_approval",
        DeviceAuthorizationState::Revoked => "revoked",
    }
}

#[must_use]
pub fn primary_status_for_setup_state(setup_state: &str) -> &'static str {
    match setup_state {
        "authorized" => "ready",
        "awaiting_approval" => "awaiting_approval",
        "revoked" => "revoked",
        _ => "not_setup",
    }
}

#[must_use]
pub fn setup_label_for_setup_state(setup_state: &str) -> &'static str {
    match setup_state {
        "authorized" => "Linked",
        "awaiting_approval" => "Awaiting approval",
        "revoked" => "Revoked",
        _ => "Not linked",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupStateFlags {
    pub setup_complete: bool,
    pub awaiting_approval: bool,
    pub revoked: bool,
}

#[must_use]
pub fn setup_state_flags(setup_state: &str) -> SetupStateFlags {
    SetupStateFlags {
        setup_complete: setup_state == "authorized",
        awaiting_approval: setup_state == "awaiting_approval",
        revoked: setup_state == "revoked",
    }
}

#[must_use]
pub fn primary_status_label(primary_status: &str) -> &'static str {
    match primary_status {
        "revoked" => "Device removed",
        "awaiting_approval" => "Waiting for approval",
        _ => "Ready",
    }
}

#[must_use]
pub fn sync_status_label(sync_status: &str) -> String {
    match sync_status {
        "running" => "Sync on".to_owned(),
        "syncing" => "Syncing".to_owned(),
        "synced" => "Synced".to_owned(),
        "root synced" => "Root synced".to_owned(),
        "up to date" => "Up to date".to_owned(),
        "sync error" => "Sync failed".to_owned(),
        "paused" => "Sync paused".to_owned(),
        value if value.trim().is_empty() => "Sync paused".to_owned(),
        value => value.to_owned(),
    }
}

#[must_use]
pub fn device_role_key(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Admin => "admin",
        DeviceRole::Member => "member",
    }
}

#[must_use]
pub fn device_role_label(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Admin => "Admin",
        DeviceRole::Member => "Member",
    }
}

#[must_use]
pub fn device_display_label(
    is_current_device: bool,
    label: Option<&str>,
    fallback: &str,
) -> String {
    if is_current_device {
        return "This device".to_owned();
    }
    label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

#[allow(clippy::fn_params_excessive_bools)]
#[must_use]
pub fn device_connection_state(
    is_current_device: bool,
    is_online: bool,
    is_direct: bool,
    is_mesh: bool,
) -> &'static str {
    if is_current_device {
        "local"
    } else if is_direct {
        "direct"
    } else if is_mesh {
        "mesh"
    } else if is_online {
        "online"
    } else {
        "offline"
    }
}

#[must_use]
pub fn device_connection_label(
    connection_state: &str,
    transport_type: Option<&str>,
    srtt_ms: Option<u64>,
) -> String {
    if connection_state == "local" {
        return "This device".to_owned();
    }
    if connection_state == "offline" {
        return "Offline".to_owned();
    }
    let transport = transport_type.map(str::to_uppercase);
    match (transport, srtt_ms, connection_state) {
        (Some(transport), Some(srtt_ms), _) => format!("Online ({transport}, {srtt_ms} ms)"),
        (Some(transport), None, _) => format!("Online ({transport})"),
        (None, Some(srtt_ms), _) => format!("Online ({srtt_ms} ms)"),
        (None, None, "mesh") => "Online (Mesh)".to_owned(),
        _ => "Online".to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceManagementActions {
    pub can_revoke: bool,
    pub can_appoint_admin: bool,
    pub can_demote_admin: bool,
}

#[must_use]
pub fn device_management_actions(
    can_manage_devices: bool,
    is_current_device: bool,
    is_admin: bool,
    admin_count: usize,
) -> DeviceManagementActions {
    DeviceManagementActions {
        can_revoke: can_manage_devices && !is_current_device,
        can_appoint_admin: can_manage_devices && !is_current_device && !is_admin,
        can_demote_admin: can_manage_devices && !is_current_device && is_admin && admin_count > 1,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceConnectionDetails {
    pub transport_type: Option<String>,
    pub srtt_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceConnectivity {
    pub online_devices: BTreeSet<String>,
    pub direct_devices: BTreeSet<String>,
    pub mesh_devices: BTreeSet<String>,
    pub peer_statuses: BTreeMap<String, DeviceConnectionDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRosterRow {
    pub pubkey_hex: String,
    pub npub: String,
    pub label: Option<String>,
    pub display_label: String,
    pub role: String,
    pub role_label: String,
    pub state: String,
    pub state_label: String,
    pub is_current_device: bool,
    pub is_online: bool,
    pub is_direct: bool,
    pub is_mesh: bool,
    pub online_via: Option<String>,
    pub connection_state: String,
    pub connection_label: String,
    pub transport_type: Option<String>,
    pub srtt_ms: Option<u64>,
    pub can_revoke: bool,
    pub can_appoint_admin: bool,
    pub can_demote_admin: bool,
    pub added_at: i64,
}

#[must_use]
pub fn pubkey_npub(hex: &str) -> String {
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| hex.to_owned())
}

#[must_use]
pub fn device_roster_rows(
    devices: &[DeviceEntry],
    current_device_pubkey: &str,
    can_manage_devices: bool,
    current_device_online: bool,
    connectivity: &DeviceConnectivity,
) -> Vec<DeviceRosterRow> {
    let admin_count = devices
        .iter()
        .filter(|device| device.role == DeviceRole::Admin)
        .count();

    devices
        .iter()
        .map(|device| {
            let npub = pubkey_npub(&device.pubkey);
            let is_current_device = device.pubkey == current_device_pubkey;
            let is_direct = !is_current_device && connectivity.direct_devices.contains(&npub);
            let is_mesh = !is_current_device && connectivity.mesh_devices.contains(&npub);
            let is_online = if is_current_device {
                current_device_online
            } else {
                connectivity.online_devices.contains(&npub) || is_direct || is_mesh
            };
            let online_via = device_online_via(is_current_device, is_online, is_direct, is_mesh);
            let connection_state =
                device_connection_state(is_current_device, is_online, is_direct, is_mesh)
                    .to_owned();
            let connection = connectivity.peer_statuses.get(&npub);
            let transport_type = connection.and_then(|status| status.transport_type.clone());
            let srtt_ms = connection.and_then(|status| status.srtt_ms);
            let actions = device_management_actions(
                can_manage_devices,
                is_current_device,
                device.role == DeviceRole::Admin,
                admin_count,
            );
            DeviceRosterRow {
                pubkey_hex: device.pubkey.clone(),
                npub: npub.clone(),
                label: device.label.clone(),
                display_label: device_display_label(
                    is_current_device,
                    device.label.as_deref(),
                    &npub,
                ),
                role: device_role_key(device.role).to_owned(),
                role_label: device_role_label(device.role).to_owned(),
                state: "Linked".to_owned(),
                state_label: "Linked".to_owned(),
                is_current_device,
                is_online,
                is_direct,
                is_mesh,
                online_via,
                connection_label: device_connection_label(
                    &connection_state,
                    transport_type.as_deref(),
                    srtt_ms,
                ),
                connection_state,
                transport_type,
                srtt_ms,
                can_revoke: actions.can_revoke,
                can_appoint_admin: actions.can_appoint_admin,
                can_demote_admin: actions.can_demote_admin,
                added_at: device.added_at,
            }
        })
        .collect()
}

fn device_online_via(
    is_current_device: bool,
    is_online: bool,
    is_direct: bool,
    is_mesh: bool,
) -> Option<String> {
    if is_current_device && is_online {
        Some("local".to_owned())
    } else if is_direct {
        Some("direct".to_owned())
    } else if is_mesh {
        Some("mesh".to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceAuthorizationState, DeviceRole};

    #[test]
    fn shared_device_summary_labels_match_native_clients() {
        assert_eq!(
            authorization_state_key(DeviceAuthorizationState::Authorized),
            "authorized"
        );
        assert_eq!(
            authorization_state_key(DeviceAuthorizationState::AwaitingApproval),
            "awaiting_approval"
        );
        assert_eq!(
            authorization_state_key(DeviceAuthorizationState::Revoked),
            "revoked"
        );

        assert_eq!(primary_status_for_setup_state("authorized"), "ready");
        assert_eq!(setup_label_for_setup_state("authorized"), "Linked");
        assert_eq!(
            setup_state_flags("authorized"),
            SetupStateFlags {
                setup_complete: true,
                awaiting_approval: false,
                revoked: false,
            }
        );
        assert_eq!(
            setup_state_flags("awaiting_approval"),
            SetupStateFlags {
                setup_complete: false,
                awaiting_approval: true,
                revoked: false,
            }
        );
        assert_eq!(
            setup_state_flags("revoked"),
            SetupStateFlags {
                setup_complete: false,
                awaiting_approval: false,
                revoked: true,
            }
        );
        assert_eq!(
            primary_status_label("awaiting_approval"),
            "Waiting for approval"
        );
        assert_eq!(sync_status_label("running"), "Sync on");
        assert_eq!(sync_status_label("up to date"), "Up to date");
        assert_eq!(sync_status_label("paused"), "Sync paused");

        assert_eq!(device_role_key(DeviceRole::Admin), "admin");
        assert_eq!(device_role_label(DeviceRole::Member), "Member");
        assert_eq!(
            device_display_label(true, Some("Mac"), "npub1x"),
            "This device"
        );
        assert_eq!(
            device_display_label(false, Some("  Phone  "), "npub1x"),
            "Phone"
        );
        assert_eq!(device_display_label(false, Some("  "), "npub1x"), "npub1x");

        let direct = device_connection_state(false, true, true, false);
        assert_eq!(direct, "direct");
        assert_eq!(
            device_connection_label(direct, Some("tcp"), Some(17)),
            "Online (TCP, 17 ms)"
        );
        assert_eq!(device_connection_label("mesh", None, None), "Online (Mesh)");
        assert_eq!(device_connection_label("offline", None, None), "Offline");

        let member = device_management_actions(true, false, false, 2);
        assert!(member.can_revoke);
        assert!(member.can_appoint_admin);
        assert!(!member.can_demote_admin);

        let peer_admin = device_management_actions(true, false, true, 2);
        assert!(peer_admin.can_revoke);
        assert!(!peer_admin.can_appoint_admin);
        assert!(peer_admin.can_demote_admin);

        let sole_admin = device_management_actions(true, false, true, 1);
        assert!(!sole_admin.can_demote_admin);

        let current = device_management_actions(true, true, true, 2);
        assert!(!current.can_revoke);
        assert!(!current.can_appoint_admin);
        assert!(!current.can_demote_admin);
    }

    #[test]
    fn shared_device_rows_include_presence_roles_and_actions() {
        let current = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let remote = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let current_npub = pubkey_npub(current);
        let remote_npub = pubkey_npub(remote);
        let devices = vec![
            crate::app_keys::DeviceEntry::admin(current.to_owned(), 10, Some("Mac".to_owned())),
            crate::app_keys::DeviceEntry::member(remote.to_owned(), 11, Some("Phone".to_owned())),
        ];
        let connectivity = DeviceConnectivity {
            online_devices: [remote_npub.clone()].into_iter().collect(),
            direct_devices: [remote_npub.clone()].into_iter().collect(),
            peer_statuses: [(
                remote_npub.clone(),
                DeviceConnectionDetails {
                    transport_type: Some("tcp".to_owned()),
                    srtt_ms: Some(12),
                },
            )]
            .into_iter()
            .collect(),
            ..DeviceConnectivity::default()
        };

        let rows = device_roster_rows(&devices, current, true, true, &connectivity);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].npub, current_npub);
        assert_eq!(rows[0].display_label, "This device");
        assert_eq!(rows[0].role, "admin");
        assert_eq!(rows[0].role_label, "Admin");
        assert_eq!(rows[0].connection_state, "local");
        assert_eq!(rows[0].connection_label, "This device");
        assert!(!rows[0].can_revoke);

        assert_eq!(rows[1].npub, remote_npub);
        assert_eq!(rows[1].display_label, "Phone");
        assert_eq!(rows[1].role, "member");
        assert_eq!(rows[1].role_label, "Member");
        assert!(rows[1].is_online);
        assert!(rows[1].is_direct);
        assert_eq!(rows[1].online_via.as_deref(), Some("direct"));
        assert_eq!(rows[1].connection_state, "direct");
        assert_eq!(rows[1].connection_label, "Online (TCP, 12 ms)");
        assert!(rows[1].can_revoke);
        assert!(rows[1].can_appoint_admin);
        assert!(!rows[1].can_demote_admin);
    }
}
