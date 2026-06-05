#[allow(clippy::wildcard_imports)]
use super::*;
use std::collections::{BTreeMap, BTreeSet};

use iris_drive_core::device_summary::{
    DeviceConnectionDetails, DeviceConnectivity, DeviceRosterRow, device_roster_rows,
};

#[allow(clippy::too_many_lines)]
pub(crate) fn peer_statuses(
    config_dir: &Path,
    config: &AppConfig,
    daemon_status: Option<&Value>,
) -> Vec<serde_json::Value> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return Vec::new();
    };
    let primary_drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID);

    let daemon_running = daemon_status
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fips_status = daemon_status
        .and_then(|status| status.get("fips_block_sync"))
        .filter(|value| value.is_object());
    let connected_fips = fips_direct_devices_from_status(fips_status)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mesh_fips = fips_mesh_devices_from_status(fips_status)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let online_fips = fips_online_devices_from_status(fips_status)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let authorized_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("authorized_peers")));
    let fips_peer_statuses =
        fips_peer_statuses_by_npub(fips_status.and_then(|status| status.get("peer_statuses")));
    let connectivity = DeviceConnectivity {
        online_devices: online_fips.clone(),
        direct_devices: connected_fips.clone(),
        mesh_devices: mesh_fips.clone(),
        peer_statuses: fips_peer_statuses
            .iter()
            .map(|(npub, status)| {
                (
                    npub.clone(),
                    DeviceConnectionDetails {
                        transport_type: status
                            .get("transport_type")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        srtt_ms: status.get("srtt_ms").and_then(Value::as_u64),
                    },
                )
            })
            .collect(),
    };
    let block_sync_by_root = daemon_status
        .and_then(|status| status.get("block_sync_by_root"))
        .filter(|value| value.is_object());
    let can_manage_devices = account.can_manage_devices();

    device_roster_rows(
        &snapshot.app_actors,
        &account.device_pubkey,
        can_manage_devices,
        daemon_running,
        &connectivity,
    )
    .iter()
    .map(|device| {
        let root = primary_drive.and_then(|drive| drive.device_roots.get(&device.pubkey_hex));
        let root_cid = root.map(|root| root.root_cid.clone());
        let root_private = root_cid.as_deref().and_then(root_is_private);
        let root_available = root_cid
            .as_deref()
            .map(|root| root_file_count(config_dir, root).is_some());
        let fips_peer_status = fips_peer_statuses.get(&device.npub);
        let sync_state =
            device_sync_state(device.is_current_device, root.is_some(), root_available);
        let last_block_sync = root_cid
            .as_ref()
            .and_then(|root| block_sync_by_root.and_then(|map| map.get(root)).cloned());
        let detail = peer_detail(
            device,
            sync_state,
            last_block_sync.as_ref(),
            root_cid.as_deref(),
            root.map(|root| root.dck_generation),
        );
        json!({
            "device_pubkey": device.pubkey_hex,
            "device_npub": device.npub,
            "label": device.label,
            "display_label": device.display_label,
            "role": device.role,
            "role_label": device.role_label,
            "authorized": true,
            "is_current_device": device.is_current_device,
            "added_at": device.added_at,
            "fips_authorized": authorized_fips.contains(&device.npub),
            "fips_online": device.is_online,
            "fips_direct_online": device.is_direct,
            "fips_mesh_online": device.is_mesh,
            "fips_online_via": device.online_via,
            "connection_state": device.connection_state,
            "connection_label": device.connection_label,
            "can_revoke": device.can_revoke,
            "can_appoint_admin": device.can_appoint_admin,
            "can_demote_admin": device.can_demote_admin,
            "fips_transport_type": device.transport_type,
            "fips_transport_addr": fips_peer_status
                .and_then(|status| status.get("transport_addr"))
                .and_then(Value::as_str),
            "fips_srtt_ms": device.srtt_ms,
            "fips_ping_ms": device.srtt_ms,
            "fips_packets_sent": fips_peer_status
                .and_then(|status| status.get("packets_sent"))
                .and_then(Value::as_u64),
            "fips_packets_recv": fips_peer_status
                .and_then(|status| status.get("packets_recv"))
                .and_then(Value::as_u64),
            "fips_bytes_sent": fips_peer_status
                .and_then(|status| status.get("bytes_sent"))
                .and_then(Value::as_u64),
            "fips_bytes_recv": fips_peer_status
                .and_then(|status| status.get("bytes_recv"))
                .and_then(Value::as_u64),
            "has_root": root.is_some(),
            "root_cid": root_cid,
            "root_private": root_private,
            "root_available": root_available,
            "sync_state": sync_state,
            "detail": detail,
            "last_block_sync": last_block_sync,
            "published_at": root.map(|root| root.published_at),
            "dck_generation": root.map(|root| root.dck_generation),
            "device_seq": root.map(|root| root.device_seq),
        })
    })
    .collect()
}

fn fips_peer_statuses_by_npub(value: Option<&Value>) -> BTreeMap<String, Value> {
    value
        .and_then(Value::as_array)
        .map(|statuses| {
            statuses
                .iter()
                .filter_map(|status| {
                    status
                        .get("npub")
                        .and_then(Value::as_str)
                        .map(|npub| (npub.to_owned(), status.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn peer_detail(
    device: &DeviceRosterRow,
    sync_state: &str,
    last_block_sync: Option<&Value>,
    root_cid: Option<&str>,
    dck_generation: Option<u64>,
) -> String {
    let mut parts = Vec::new();
    if device.is_current_device {
        parts.push("This AppKey".to_owned());
    }
    if !device.role_label.trim().is_empty() {
        parts.push(device.role_label.clone());
    }
    if !sync_state.trim().is_empty() {
        parts.push(sync_state.to_owned());
    }
    if let Some(block_sync) = last_block_sync {
        let transport = block_sync
            .get("transport")
            .and_then(Value::as_str)
            .filter(|transport| !transport.trim().is_empty());
        let fetched = block_sync
            .get("fetched")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let total = block_sync
            .get("total_hashes")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if let Some(transport) = transport
            && total > 0
        {
            parts.push(format!("{transport} {fetched}/{total}"));
        }
    }
    if let Some(root) = root_cid.filter(|root| !root.trim().is_empty()) {
        parts.push(short_status_value(root));
    }
    if let Some(dck) = dck_generation.filter(|dck| *dck > 0) {
        parts.push(format!("DCK {dck}"));
    }
    parts.join(" | ")
}

fn short_status_value(value: &str) -> String {
    if value.chars().count() <= 32 {
        return value.to_owned();
    }
    let start = value.chars().take(14).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(10)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{start}...{end}")
}
