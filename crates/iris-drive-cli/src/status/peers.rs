use super::*;
use std::collections::BTreeSet;

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
    let block_sync_by_root = daemon_status
        .and_then(|status| status.get("block_sync_by_root"))
        .filter(|value| value.is_object());

    snapshot
        .devices
        .iter()
        .map(|device| {
            let root = primary_drive.and_then(|drive| drive.device_roots.get(&device.pubkey));
            let root_cid = root.map(|root| root.root_cid.clone());
            let root_private = root_cid.as_deref().and_then(root_is_private);
            let root_available = root_cid
                .as_deref()
                .map(|root| root_file_count(config_dir, root).is_some());
            let device_npub = account_npub(&device.pubkey);
            let is_current_device = device.pubkey == account.device_pubkey;
            let fips_direct_online = connected_fips.contains(&device_npub);
            let fips_mesh_online = mesh_fips.contains(&device_npub);
            let fips_peer_status = fips_peer_statuses.get(&device_npub);
            let fips_transport_type = fips_peer_status
                .and_then(|status| status.get("transport_type"))
                .and_then(Value::as_str);
            let fips_srtt_ms = fips_peer_status
                .and_then(|status| status.get("srtt_ms"))
                .and_then(Value::as_u64);
            let fips_online = if is_current_device {
                daemon_running
            } else {
                online_fips.contains(&device_npub)
            };
            let fips_online_via = if is_current_device && fips_online {
                Some("local")
            } else if fips_direct_online {
                Some("direct")
            } else if fips_mesh_online {
                Some("mesh")
            } else {
                None
            };
            let connection_state = peer_connection_state(
                is_current_device,
                fips_online,
                fips_direct_online,
                fips_mesh_online,
            );
            let connection_label =
                peer_connection_label(connection_state, fips_transport_type, fips_srtt_ms);
            let sync_state = device_sync_state(is_current_device, root.is_some(), root_available);
            let last_block_sync = root_cid
                .as_ref()
                .and_then(|root| block_sync_by_root.and_then(|map| map.get(root)).cloned());
            json!({
                "device_pubkey": device.pubkey,
                "device_npub": device_npub,
                "label": device.label,
                "display_label": peer_display_label(
                    is_current_device,
                    device.label.as_deref(),
                    &device_npub
                ),
                "role": device_role_label(device.role),
                "role_label": device_role_display_label(device.role),
                "authorized": true,
                "is_current_device": is_current_device,
                "added_at": device.added_at,
                "fips_authorized": authorized_fips.contains(&device_npub),
                "fips_online": fips_online,
                "fips_direct_online": fips_direct_online,
                "fips_mesh_online": fips_mesh_online,
                "fips_online_via": fips_online_via,
                "connection_state": connection_state,
                "connection_label": connection_label,
                "fips_transport_type": fips_transport_type,
                "fips_transport_addr": fips_peer_status
                    .and_then(|status| status.get("transport_addr"))
                    .and_then(Value::as_str),
                "fips_srtt_ms": fips_srtt_ms,
                "fips_ping_ms": fips_srtt_ms,
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
                "last_block_sync": last_block_sync,
                "published_at": root.map(|root| root.published_at),
                "dck_generation": root.map(|root| root.dck_generation),
                "device_seq": root.map(|root| root.device_seq),
            })
        })
        .collect()
}

fn peer_display_label(is_current_device: bool, label: Option<&str>, fallback: &str) -> String {
    if is_current_device {
        return "This device".to_owned();
    }
    label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn device_role_display_label(role: iris_drive_core::DeviceRole) -> &'static str {
    match role {
        iris_drive_core::DeviceRole::Admin => "Admin",
        iris_drive_core::DeviceRole::Member => "Member",
    }
}

fn peer_connection_state(
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

fn peer_connection_label(
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
